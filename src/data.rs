use crate::value::Value;
use std::fmt::{Formatter, Debug};
use std::{fmt, mem, iter, thread, cmp};
use std::collections::{HashMap, BTreeMap, HashSet, BTreeSet};
use std::rc::Rc;
use std::cell::{RefCell, Ref};
use llvm_ir::instruction::{MemoryOrdering, Atomicity};
use crate::layout::Layout;
use std::future::{Future, Ready};
use std::pin::Pin;
use std::any::type_name_of_val;
use std::hash::{Hash, Hasher};
use defer::defer;
use crate::backtrace::Backtrace;
use crate::process::Process;
use crate::thread::{Thread, ThreadId};
use std::ops::Deref;
use std::task::{Poll, Context, Waker};
use crate::util::future::pending_once;
use smallvec::alloc::collections::VecDeque;
use crate::timer;
use smallvec::SmallVec;
use smallvec::smallvec;
use crate::util::rangemap::RangeMap;
use crate::util::freelist::{FreeList, Frc};
use itertools::Itertools;
use crate::util::by_address::ByAddress;
use indexmap::set::IndexSet;
use crate::ordering::Ordering;
use crate::thread::Blocker;

const PIPELINE_SIZE: usize = 1000;

pub struct DataFlowInner {
    process: Process,
    threadid: ThreadId,
    seq: usize,
    freelist: FreeList<ThunkInner>,
    thunks: IndexSet<Thunk>,
    tracing: bool,
}

#[derive(Clone)]
pub struct DataFlow(Rc<RefCell<DataFlowInner>>);

pub enum LoadOrdering {
    Unordered,
    Monotonic,
    Acquire,
    SequentiallyConsistent,
    NotAtomic,
}

pub enum StoreOrdering {
    Unordered,
    Monotonic,
    Release,
    SequentiallyConsistent,
    NotAtomic,
}

pub struct ComputeCtx {
    pub process: Process,
    pub threadid: ThreadId,
}

pub enum ThunkState {
    Pending(Box<dyn FnOnce(&ComputeCtx, &[&Value]) -> Value>),
    Ready(Value),
    Sandbag,
}


pub struct ThunkInner {
    process: Process,
    threadid: ThreadId,
    seq: usize,
    deps: ThunkDeps,
    address: Option<(Thunk, Layout)>,
    atomicity: Ordering,
    backtrace: Backtrace,
    value: RefCell<ThunkState>,
    wakers: RefCell<Option<Waker>>,
    tracing: bool,
}

pub type ThunkDeps = SmallVec<[Thunk; 4]>;

#[derive(Clone)]
pub struct Thunk(Frc<ThunkInner>);


impl Thunk {
    pub fn constant(process: Process, value: Value) -> Self {
        Thunk(FreeList::new().alloc(ThunkInner {
            process: process,
            threadid: ThreadId(0),
            seq: 0,
            deps: smallvec![],
            address: None,
            atomicity: Ordering::None,
            backtrace: Backtrace::empty(),
            value: RefCell::new(ThunkState::Ready(value)),
            wakers: RefCell::new(None),
            tracing: false,
        }))
    }
}

impl Thunk {
    pub fn try_get(&self) -> Option<&Value> {
        let r = self.0.value.borrow();
        match &*r {
            ThunkState::Pending(_) => return None,
            ThunkState::Ready(_) => {}
            ThunkState::Sandbag => unreachable!(),
        }
        let r = Ref::leak(r);
        match r {
            ThunkState::Ready(value) => Some(value),
            _ => unreachable!()
        }
    }
    pub fn step<'a>(&'a self) -> bool {
        let comp = ComputeCtx { process: self.0.process.clone(), threadid: self.0.threadid };
        timer!("Thunk::step");
        let _d = defer(move || if thread::panicking() {
            println!("Panic while forcing {:#?}", self.full_debug());
        });
        let args = self.0.deps.iter().map(|d| d.try_get()).collect::<Option<SmallVec<[_; 4]>>>();
        let args = if let Some(args) = args { args } else { return false; };
        match &*self.0.value.borrow() {
            ThunkState::Ready(_) => return false,
            ThunkState::Pending(_) => {}
            _ => unreachable!(),
        }
        if self.0.tracing {
            print!("res {:10} = ", format!("{:?}", self));
        }
        {
            let mut r = self.0.value.borrow_mut();
            match mem::replace(&mut *r, ThunkState::Sandbag) {
                ThunkState::Pending(compute) => {
                    let v = compute(&comp, args.as_slice());
                    *r = ThunkState::Ready(v);
                    mem::drop(r);
                }
                ThunkState::Ready(_) => { return false; }
                _ => unreachable!(),
            }
        }
        if self.0.tracing {
            println!("{:?}", self);
        }
        if let Some(waker) = self.0.wakers.borrow_mut().take() {
            waker.wake();
        }
        return true;
    }
    pub fn all_deps(&self) -> Vec<&Thunk> {
        let mut frontier: Vec<&Thunk> = vec![self];
        let mut all = BTreeSet::new();
        while let Some(next) = frontier.pop() {
            if all.insert(next) {
                for dep in next.0.deps.iter() {
                    frontier.push(dep)
                }
            }
        }
        all.into_iter().collect()
    }
    pub fn backtrace(&self) -> &Backtrace {
        &self.0.backtrace
    }
    pub fn full_debug<'a>(&'a self) -> impl 'a + Debug {
        (self, self.all_deps(), self.backtrace())
    }
}


impl DataFlow {
    pub fn new(process: Process, threadid: ThreadId) -> Self {
        DataFlow(Rc::new(RefCell::new(DataFlowInner {
            process,
            threadid,
            seq: 0,
            freelist: FreeList::new(),
            thunks: IndexSet::new(),
            tracing: true,
        })))
    }
    pub async fn thunk_impl(
        &self,
        backtrace: Backtrace,
        address: Option<(Thunk, Layout)>,
        atomicity: Ordering,
        deps: ThunkDeps,
        compute: impl 'static + FnOnce(&ComputeCtx, &[&Value]) -> Value) -> Thunk {
        let thread = self.0.borrow().process.thread(self.0.borrow().threadid).unwrap();
        while self.0.borrow().thunks.len() >= PIPELINE_SIZE {
            thread.pending_once(Blocker::Pipeline).await;
        }
        let mut this = self.0.borrow_mut();
        let seq = this.seq;
        this.seq += 1;
        let thunk: Thunk = Thunk(this.freelist.alloc(ThunkInner {
            process: this.process.clone(),
            threadid: this.threadid,
            seq,
            deps,
            address,
            atomicity,
            backtrace,
            value: RefCell::new(ThunkState::Pending(Box::new(compute))),
            wakers: RefCell::new(None),
            tracing: this.tracing,
        }));
        this.thunks.insert(thunk.clone());
        if this.tracing {
            let id = format!("{:?}", thunk);
            let deps = thunk.0.deps.iter().map(|x| format!("{:?}", x)).join(", ");
            let loc = thunk.0.backtrace.iter().next().map(|x| x.loc()).unwrap_or_default();
            let address = thunk.0.address.as_ref().map(|(t, l)| format!("{:?}", t)).unwrap_or_default();
            let layout = thunk.0.address.as_ref().map(|(t, l)| format!("{:?}", l)).unwrap_or_default();
            let order = format!("{:?}", thunk.0.atomicity);
            println!("dec {:10} = [{:40} >= {:60}] {:20} {:10} {:10}",
                     id, deps, loc, address, layout, order);
        }
        thunk
    }
    pub async fn thunk(
        &self,
        backtrace: Backtrace,
        deps: ThunkDeps,
        compute: impl 'static + FnOnce(&ComputeCtx, &[&Value]) -> Value) -> Thunk {
        self.thunk_impl(backtrace, None, Ordering::None, deps, compute).await
    }
    pub async fn constant(&self, backtrace: Backtrace, v: Value) -> Thunk {
        self.thunk(backtrace, smallvec![], |_, _| v).await
    }
    pub async fn store<'a>(&'a self, backtrace: Backtrace, address: Thunk, layout: Layout, value: Thunk, atomicity: Ordering) -> Thunk {
        let process = self.0.borrow().process.clone();
        let threadid = self.0.borrow().threadid;
        self.thunk_impl(
            backtrace,
            Some((address.clone(), layout)),
            atomicity,
            smallvec![address, value],
            move |comp, args| {
                process.memory.store_impl(threadid, args[0].as_u64(), args[1], atomicity);
                Value::from(())
            }).await
    }
    pub async fn load<'a>(&'a self, backtrace: Backtrace, address: Thunk, layout: Layout, atomicity: Ordering) -> Thunk {
        let process = self.0.borrow().process.clone();
        let threadid = self.0.borrow().threadid;
        self.thunk_impl(
            backtrace,
            Some((address.clone(), layout)),
            atomicity,
            smallvec![address],
            move |comp, args| {
                process.memory.load_impl(threadid, args[0].as_u64(), layout, atomicity)
            }).await
    }
    pub fn len(&self) -> usize {
        self.0.borrow().thunks.len()
    }
    pub fn seq(&self) -> usize {
        self.0.borrow().seq
    }
    pub fn step_pure(&self, resolvable: &mut Vec<Thunk>) -> bool {
        let comp = self.comp();
        let mut this = self.0.borrow_mut();
        let this = &mut *this;
        let mut progress = false;
        let mut pending = RangeMap::<u64, ()>::new();
        this.thunks.retain(|t| t.try_get().is_none());
        for thunk in this.thunks.iter() {
            let thunk: &Thunk = thunk;
            let (address, layout) = if let Some((address, layout)) = &thunk.0.address {
                (address, layout)
            } else {
                // No memory access, just do it
                progress |= thunk.step();
                continue;
            };
            let range = if let Some(address) = address.try_get() {
                address.as_u64()..address.as_u64() + layout.bytes()
            } else {
                // Access to unknown memory address which could alias any later memory access, so
                // just stop now.
                break;
            };
            if pending.range(range.clone()).next().is_some() {
                // Aliased by a previous operation
                continue;
            }
            pending.insert(range, ());
            match thunk.0.atomicity {
                Ordering::None => {
                    // Non-atomics cannot communicate with other threads. Observing this action
                    // from another thread would be a data race. So just do it now.
                    progress |= thunk.step();
                }
                Ordering::Relaxed => {
                    // Relaxed operations can be resolved in any order.
                    resolvable.push(thunk.clone());
                }
                Ordering::Release => {
                    // The release barrier only blocks this operation if there are pending operations.
                    if resolvable.is_empty() {
                        resolvable.push(thunk.clone());
                    }
                }
                Ordering::Acquire | Ordering::AcqRel | Ordering::SeqCst => {
                    // The acquire barrier allows this operation to complete, but not later operations.
                    resolvable.push(thunk.clone());
                    break;
                }
            }
        }
        progress
    }
    fn comp(&self) -> ComputeCtx {
        let this = self.0.borrow();
        ComputeCtx { process: this.process.clone(), threadid: this.threadid }
    }
    pub fn set_tracing(&self, tracing: bool) {
        self.0.borrow_mut().tracing = tracing;
    }
    pub fn tracing(&self) -> bool {
        self.0.borrow().tracing
    }
    pub fn threadid(&self) -> ThreadId { self.0.borrow().threadid }
}

struct DebugFlat<T: Debug>(T);

impl<T: Debug> Debug for DebugFlat<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", &self.0)
    }
}

struct DebugDeps<'a>(&'a [Rc<Thunk>]);

impl<'a> Debug for DebugDeps<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.0.iter().map(|thunk| thunk.0.seq)).finish()
    }
}


impl Debug for Thunk {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if let Some(value) = self.try_get() {
            write!(f, "{:?}", value)
        } else {
            write!(f, "{:?}#{:?}", self.0.threadid, self.0.seq)
        }
    }
}

impl Debug for ThunkState {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ThunkState::Pending(_) => write!(f, "Pending"),
            ThunkState::Ready(value) => write!(f, "{:?}", value),
            ThunkState::Sandbag => write!(f, "Sandbag"),
        }
    }
}

impl Future for Thunk {
    type Output = Value;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(v) = self.try_get() {
            self.0.process.thread(self.0.threadid).unwrap().set_blocker(Blocker::Unknown);
            Poll::Ready(v.clone())
        } else {
            let mut wakers = self.0.wakers.borrow_mut();
            assert!(wakers.is_none());
            *wakers = Some(cx.waker().clone());
            self.0.process.thread(self.0.threadid).unwrap().set_blocker(Blocker::Thunk(self.clone()));
            Poll::Pending
        }
    }
}

impl Hash for Thunk {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (&*self.0 as *const ThunkInner).hash(state)
    }
}

impl Eq for Thunk {}

impl PartialEq for Thunk {
    fn eq(&self, other: &Self) -> bool {
        (&*self.0 as *const _) == (&*other.0 as *const _)
    }
}

impl Ord for Thunk {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.0.seq.cmp(&other.0.seq).then((&*self.0 as *const ThunkInner).cmp(&(&*other.0 as *const ThunkInner)))
    }
}

impl PartialOrd for Thunk {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Debug for DataFlow {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let this = self.0.borrow();
        f.debug_struct("DataFlow")
            .field("threadid", &this.threadid)
            .field("thunks", &this.thunks)
            .field("seq", &this.seq)
            .finish()
    }
}