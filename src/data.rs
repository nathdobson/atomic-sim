use futures::pending;
use crate::value::Value;
use std::fmt::{Formatter, Debug};
use std::{fmt, mem, iter, thread};
use std::collections::{HashMap, BTreeMap, HashSet, BTreeSet};
use std::rc::Rc;
use std::cell::{RefCell, Ref};
use llvm_ir::instruction::Atomicity;
use crate::layout::Layout;
use std::future::Future;
use futures::task::{Context, Poll};
use std::pin::Pin;
use std::any::type_name_of_val;
use std::hash::{Hash, Hasher};
use std::cmp::Ordering;
use defer::defer;
use crate::compute::ComputeCtx;
use crate::backtrace::Backtrace;
use crate::process::Process;
use crate::thread::{Thread, ThreadId};
use std::ops::Deref;

const PIPELINE_SIZE: usize = 1;

pub struct DataFlowInner {
    process: Process,
    threadid: ThreadId,
    seq: usize,
    thunks: BTreeMap<usize, Thunk>,
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

pub enum ThunkState {
    Pending(Box<dyn FnOnce(&ComputeCtx, &[&Value]) -> Value>),
    Load { layout: Layout },
    Store,
    Modify(Box<dyn FnOnce(&ComputeCtx, &Value) -> (Value, Value)>),
    Ready(Value),
    Sandbag,
}

pub struct ThunkInner {
    process: Process,
    threadid: ThreadId,
    seq: usize,
    deps: Vec<Thunk>,
    address: Option<Thunk>,
    name: String,
    backtrace: Backtrace,
    value: RefCell<ThunkState>,
}

#[derive(Clone)]
pub struct Thunk(Rc<ThunkInner>);

impl Thunk {
    pub fn try_get(&self) -> Option<&Value> {
        let r = self.0.value.borrow();
        match &*r {
            ThunkState::Pending(_) => return None,
            ThunkState::Ready(_) => {}
            ThunkState::Sandbag => unreachable!(),
            _ => todo!()
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
                //println!("{:?} = {:?} ({:?})[{:?}]", self.0.seq, *r, args, self.0.deps.iter().map(|x| x.0.seq));
                println!("{:?}", self);
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
        DataFlow(Rc::new(RefCell::new(DataFlowInner { process, threadid, seq: 0, thunks: BTreeMap::new() })))
    }
    pub async fn thunk(&self, name: String, backtrace: Backtrace, deps: Vec<Thunk>, compute: impl 'static + FnOnce(&ComputeCtx, &[&Value]) -> Value) -> Thunk {
        while self.0.borrow().thunks.len() >= PIPELINE_SIZE {
            pending!();
        }
        let mut this = self.0.borrow_mut();
        let seq = this.seq;
        this.seq += 1;
        let thunk: Thunk = Thunk(Rc::new(ThunkInner {
            process: this.process.clone(),
            threadid: this.threadid,
            seq,
            deps,
            address: None,
            name,
            backtrace,
            value: RefCell::new(ThunkState::Pending(Box::new(compute))),
        }));
        this.thunks.insert(seq, thunk.clone());
        thunk
    }
    pub async fn constant(&self, backtrace: Backtrace, v: Value) -> Thunk {
        self.thunk("constant".to_string(), backtrace, vec![], |_, _| v).await
    }
    pub async fn store<'a>(&'a self, backtrace: Backtrace, address: Thunk, value: Thunk, atomicity: Option<Atomicity>) -> Thunk {
        let threadid = self.0.borrow().threadid;
        self.thunk("store".to_string(), backtrace, vec![address, value], move |comp, args| {
            comp.process.store(threadid, args[0], args[1], atomicity);
            Value::from(())
        }).await
    }
    pub async fn load<'a>(&'a self, backtrace: Backtrace, address: Thunk, layout: Layout, atomicity: Option<Atomicity>) -> Thunk {
        let threadid = self.0.borrow().threadid;
        self.thunk("load".to_string(), backtrace, vec![address], move |comp, args| {
            comp.process.load(threadid, args[0], layout, atomicity)
        }).await
    }
    pub fn len(&self) -> usize {
        self.0.borrow().thunks.len()
    }
    pub fn seq(&self) -> usize {
        self.0.borrow().seq
    }
    pub fn step(&self) -> bool {
        let thunk = self.0.borrow_mut().thunks.pop_first();
        if let Some((_, thunk)) = thunk {
            let thunk2 = thunk.clone();
            let _d = defer(move || if thread::panicking() {
                println!("Panic while forcing {:#?}", thunk2.full_debug());
            });
            thunk.step(&ComputeCtx { process: self.0.borrow().process.clone() });
            true
        } else {
            false
        }
    }
    pub fn threadid(&self) -> ThreadId { self.0.borrow().threadid }
}

struct DebugFlat<T: Debug>(T);

impl<T: Debug> Debug for DebugFlat<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", &self.0)
    }
}

impl Debug for Thread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stack")
            .finish()
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
        write!(f, "{:?}_{} = {} {:?} [{:?}]", self.0.threadid, self.0.seq, self.0.name, self.0.value.borrow(), self.0.deps.iter().map(|dep| dep.0.seq).collect::<Vec<_>>())
    }
}

impl Debug for ThunkState {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ThunkState::Pending(_) => write!(f, "Pending"),
            ThunkState::Ready(value) => write!(f, "{:?}", value),
            ThunkState::Sandbag => write!(f, "Sandbag"),
            _ => todo!(),
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

