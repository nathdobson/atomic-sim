use crate::value::Value;
use std::fmt::{Formatter, Debug};
use std::{fmt, mem, iter, thread};
use std::collections::{HashMap, BTreeMap, HashSet, BTreeSet};
use std::rc::Rc;
use std::cell::{RefCell, Ref};
use llvm_ir::instruction::{Atomicity, MemoryOrdering};
use crate::layout::Layout;
use std::future::Future;
use std::pin::Pin;
use std::any::type_name_of_val;
use std::hash::{Hash, Hasher};
use std::cmp::Ordering;
use defer::defer;
use crate::backtrace::Backtrace;
use crate::process::Process;
use crate::thread::{Thread, ThreadId};
use std::ops::Deref;
use std::task::{Poll, Context};
use crate::util::future::pending_once;
use smallvec::alloc::collections::VecDeque;
use crate::timer;
use smallvec::SmallVec;
use smallvec::smallvec;
use crate::util::rangemap::RangeMap;
use crate::util::freelist::{FreeList, Frc};

const PIPELINE_SIZE: usize = 1000;

pub struct DataFlowInner {
    process: Process,
    threadid: ThreadId,
    seq: usize,
    freelist: FreeList<ThunkInner>,
    thunks: VecDeque<Thunk>,
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
    ordering: Option<MemoryOrdering>,
    backtrace: Backtrace,
    value: RefCell<ThunkState>,
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
            ordering: None,
            backtrace: Backtrace::empty(),
            value: RefCell::new(ThunkState::Ready(value)),
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
    pub fn step<'a>(&'a self, comp: &ComputeCtx) -> bool {
        let args = self.0.deps.iter().map(|d| d.try_get().unwrap()).collect::<Vec<_>>();
        match &*self.0.value.borrow() {
            ThunkState::Ready(_) => panic!("Already ready"),
            ThunkState::Pending(_) => {}
            _ => unreachable!(),
        }
        let mut r = self.0.value.borrow_mut();
        match mem::replace(&mut *r, ThunkState::Sandbag) {
            ThunkState::Pending(compute) => {
                let v = compute(comp, args.as_slice());
                *r = ThunkState::Ready(v);
                mem::drop(r);
            }
            ThunkState::Ready(_) => panic!("Stepping ready thunk."),
            _ => unreachable!(),
        }
        true
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
            thunks: VecDeque::new(),
            tracing: false,
        })))
    }
    pub async fn thunk_impl(
        &self,
        backtrace: Backtrace,
        address: Option<(Thunk, Layout)>,
        ordering: Option<MemoryOrdering>,
        deps: ThunkDeps,
        compute: impl 'static + FnOnce(&ComputeCtx, &[&Value]) -> Value) -> Thunk {
        while self.0.borrow().thunks.len() >= PIPELINE_SIZE {
            pending_once().await;
        }
        let mut this = self.0.borrow_mut();
        let seq = this.seq;
        this.seq += 1;
        if this.tracing {
            println!("{:?} {:?} {:?}", this.threadid, this.seq, backtrace.iter().next().unwrap());
        }
        let thunk: Thunk = Thunk(this.freelist.alloc(ThunkInner {
            process: this.process.clone(),
            threadid: this.threadid,
            seq,
            deps,
            address,
            ordering,
            backtrace,
            value: RefCell::new(ThunkState::Pending(Box::new(compute))),
            tracing: this.tracing,
        }));
        this.thunks.push_back(thunk.clone());
        thunk
    }
    pub async fn thunk(
        &self,
        backtrace: Backtrace,
        deps: ThunkDeps,
        compute: impl 'static + FnOnce(&ComputeCtx, &[&Value]) -> Value) -> Thunk {
        self.thunk_impl(backtrace, None, None, deps, compute).await
    }
    pub async fn constant(&self, backtrace: Backtrace, v: Value) -> Thunk {
        self.thunk(backtrace, smallvec![], |_, _| v).await
    }
    pub async fn store<'a>(&'a self, backtrace: Backtrace, address: Thunk, layout: Layout, value: Thunk, atomicity: Option<Atomicity>) -> Thunk {
        let process = self.0.borrow().process.clone();
        let threadid = self.0.borrow().threadid;
        let ordering = atomicity.clone().map(|x| x.mem_ordering);
        self.thunk_impl(
            backtrace,
            Some((address.clone(), layout)),
            atomicity.clone().map(|x| x.mem_ordering),
            smallvec![address, value],
            move |comp, args| {
                process.store_impl(threadid, args[0], args[1], ordering);
                Value::from(())
            }).await
    }
    pub async fn load<'a>(&'a self, backtrace: Backtrace, address: Thunk, layout: Layout, atomicity: Option<Atomicity>) -> Thunk {
        let process = self.0.borrow().process.clone();
        let threadid = self.0.borrow().threadid;
        let ordering = atomicity.clone().map(|x| x.mem_ordering);
        self.thunk_impl(
            backtrace,
            Some((address.clone(), layout)),
            ordering,
            smallvec![address],
            move |comp, args| {
                process.load_impl(threadid, args[0], layout, ordering)
            }).await
    }
    pub fn len(&self) -> usize {
        self.0.borrow().thunks.len()
    }
    pub fn seq(&self) -> usize {
        self.0.borrow().seq
    }
    pub fn pop(&self) -> Option<Thunk> {
        let mut this = self.0.borrow_mut();
        if true {
            return this.thunks.pop_front();
        }
        let mut pending_addresses = RangeMap::<u64, ()>::new();
        let mut pop_index = None;
        for (index, thunk) in this.thunks.iter().enumerate() {
            let available = thunk.0.deps.iter().all(|x| x.try_get().is_some());
            if let Some(address) = &thunk.0.address {
                let layout = address.1;
                let range = if let Some(address) = address.0.try_get() {
                    let address = address.as_u64();
                    address..address + layout.bytes()
                } else {
                    0..u64::max_value()
                };
                let known_first_at_address = pending_addresses.range(range.clone()).next().is_none();
                pending_addresses.insert(range, ());
                if known_first_at_address && available {
                    pop_index = Some(index);
                }
            } else {
                if available {
                    pop_index = Some(index);
                    break;
                }
            }
        }
        if let Some(pop_index) = pop_index {
            this.thunks.remove(pop_index)
        } else {
            assert!(this.thunks.is_empty());
            None
        }
    }
    pub fn step(&self) -> bool {
        timer!("DataFlow::step");
        let threadid = self.0.borrow().threadid;
        let thunk = self.pop();
        if let Some(thunk) = thunk {
            let thunk2 = thunk.clone();
            let _d = defer(move || if thread::panicking() {
                println!("Panic while forcing {:#?}", thunk2.full_debug());
            });
            thunk.step(&ComputeCtx { process: self.0.borrow().process.clone(), threadid });
            true
        } else {
            false
        }
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
        write!(f, "{:?}_{} = {:?} [{:?}] {:?}",
               self.0.threadid,
               self.0.seq,
               self.0.value.borrow(),
               self.0.deps.iter().map(|dep| dep.0.seq).collect::<Vec<_>>(),
               self.0.backtrace.iter().next(),
        )
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
            Poll::Ready(v.clone())
        } else {
            Poll::Pending
        }
    }
}

impl Hash for Thunk {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.seq.hash(state);
    }
}

impl Eq for Thunk {}

impl PartialEq for Thunk {
    fn eq(&self, other: &Self) -> bool {
        self.0.seq == other.0.seq
    }
}

impl Ord for Thunk {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.seq.cmp(&other.0.seq)
    }
}

impl PartialOrd for Thunk {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.seq.partial_cmp(&other.0.seq)
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