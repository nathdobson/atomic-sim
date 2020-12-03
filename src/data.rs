use futures::pending;
use crate::ctx::{Ctx};
use crate::value::Value;
use crate::process::Process;
use std::fmt::{Formatter, Debug};
use std::{fmt, mem, iter, thread};
use crate::thread::{Thread, ThreadId};
use std::collections::{HashMap, BTreeMap, HashSet, BTreeSet};
use std::rc::Rc;
use std::cell::{RefCell, Ref};
use std::borrow::BorrowMut;
use llvm_ir::instruction::Atomicity;
use crate::layout::Layout;
use std::future::Future;
use futures::task::{Context, Poll};
use std::pin::Pin;
use crate::compute::ComputeCtx;
use std::any::type_name_of_val;
use std::hash::{Hash, Hasher};
use std::cmp::Ordering;
use crate::backtrace::Backtrace;
use defer::defer;

const PIPELINE_SIZE: usize = 1;

pub struct DataFlowInner<'ctx> {
    threadid: ThreadId,
    seq: usize,
    thunks: BTreeMap<usize, Thunk<'ctx>>,
}

pub struct DataFlow<'ctx> {
    inner: RefCell<DataFlowInner<'ctx>>,
}

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

pub enum ThunkState<'ctx> {
    Pending(Box<dyn 'ctx + for<'a> FnOnce(ComputeCtx<'ctx, 'a>, &'a [&'a Value]) -> Value>),
    Load { layout: Layout },
    Store,
    Modify(Box<dyn 'ctx + for<'a> FnOnce(ComputeCtx<'ctx, 'a>, &'a Value) -> (Value, Value)>),
    Ready(Value),
    Sandbag,
}

pub struct ThunkInner<'ctx> {
    pub threadid: ThreadId,
    pub seq: usize,
    pub deps: Vec<Thunk<'ctx>>,
    pub address: Option<Thunk<'ctx>>,
    pub name: String,
    pub backtrace: Backtrace<'ctx>,
    pub value: RefCell<ThunkState<'ctx>>,
}

#[derive(Clone)]
pub struct Thunk<'ctx>(Rc<ThunkInner<'ctx>>);

impl<'ctx> Thunk<'ctx> {
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
    pub fn step<'a>(&'a self, comp: ComputeCtx<'ctx, 'a>) -> bool {
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
    pub fn all_deps(&self) -> Vec<&Thunk<'ctx>> {
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
    pub fn backtrace(&self) -> &Backtrace<'ctx> {
        &self.0.backtrace
    }
    pub fn full_debug(&self, ctx: &Ctx<'ctx>) -> (Vec<&Thunk<'ctx>>, impl Debug + 'ctx) {
        (self.all_deps(), self.backtrace().full_debug(ctx))
    }
}

impl<'ctx> DataFlow<'ctx> {
    pub fn new(threadid: ThreadId) -> Self {
        DataFlow {
            inner: RefCell::new(DataFlowInner { threadid, seq: 0, thunks: BTreeMap::new() }),
        }
    }
    pub async fn thunk(&self, name: String, backtrace: Backtrace<'ctx>, deps: Vec<Thunk<'ctx>>, compute: impl 'ctx + for<'a> FnOnce(ComputeCtx<'ctx, 'a>, &'a [&'a Value]) -> Value) -> Thunk<'ctx> {
        while self.inner.borrow().thunks.len() >= PIPELINE_SIZE {
            pending!();
        }
        let mut this = self.inner.borrow_mut();
        let seq = this.seq;
        this.seq += 1;
        let thunk: Thunk<'ctx> = Thunk(Rc::new(ThunkInner {
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
    pub async fn constant(&self, backtrace: Backtrace<'ctx>, v: Value) -> Thunk<'ctx> {
        self.thunk("constant".to_string(), backtrace, vec![], |_, _| v).await
    }
    pub async fn store<'a>(&'a self, backtrace: Backtrace<'ctx>, address: Thunk<'ctx>, value: Thunk<'ctx>, atomicity: Option<&'ctx Atomicity>) -> Thunk<'ctx> {
        self.thunk("store".to_string(), backtrace, vec![address, value], move |comp, args| {
            comp.process.store(comp.threadid, args[0], args[1], atomicity);
            Value::from(())
        }).await
    }
    pub async fn load<'a>(&'a self, backtrace: Backtrace<'ctx>, address: Thunk<'ctx>, layout: Layout, atomicity: Option<&'ctx Atomicity>) -> Thunk<'ctx> {
        self.thunk("load".to_string(), backtrace, vec![address], move |comp, args| {
            comp.process.load(comp.threadid, args[0], layout, atomicity)
        }).await
    }
    pub fn len(&self) -> usize {
        self.inner.borrow().thunks.len()
    }
    pub fn seq(&self) -> usize {
        self.inner.borrow().seq
    }
    pub fn step(&self, threadid: ThreadId) -> impl FnOnce(&mut Process<'ctx>) -> bool {
        let thunk = self.inner.borrow_mut().thunks.pop_first();
        move |process| {
            if let Some((_, thunk)) = thunk {
                let ctx = process.ctx.clone();
                let thunk2 = thunk.clone();
                let _d = defer(move || if thread::panicking() {
                    println!("Panic while forcing {:#?}", thunk2.full_debug(&ctx));
                });
                thunk.step(ComputeCtx { process: process, threadid });
                true
            } else {
                false
            }
        }
    }
}

struct DebugFlat<T: Debug>(T);

impl<T: Debug> Debug for DebugFlat<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", &self.0)
    }
}

impl<'ctx> Debug for Thread<'ctx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stack")
            .finish()
    }
}


struct DebugDeps<'a>(&'a [Rc<Thunk<'a>>]);

impl<'a> Debug for DebugDeps<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.0.iter().map(|thunk| thunk.0.seq)).finish()
    }
}


impl<'ctx> Debug for Thunk<'ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}_{} = {} {:?} [{:?}]", self.0.threadid, self.0.seq, self.0.name, self.0.value.borrow(), self.0.deps.iter().map(|dep| dep.0.seq).collect::<Vec<_>>())
    }
}

impl<'ctx> Debug for ThunkState<'ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ThunkState::Pending(_) => write!(f, "Pending"),
            ThunkState::Ready(value) => write!(f, "{:?}", value),
            ThunkState::Sandbag => write!(f, "Sandbag"),
            _ => todo!(),
        }
    }
}

impl<'ctx> Future for Thunk<'ctx> {
    type Output = Value;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(v) = self.try_get() {
            Poll::Ready(v.clone())
        } else {
            Poll::Pending
        }
    }
}

impl<'ctx> Hash for Thunk<'ctx> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.seq.hash(state);
    }
}

impl<'ctx> Eq for Thunk<'ctx> {}

impl<'ctx> PartialEq for Thunk<'ctx> {
    fn eq(&self, other: &Self) -> bool {
        self.0.seq == other.0.seq
    }
}

impl<'ctx> Ord for Thunk<'ctx> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.seq.cmp(&other.0.seq)
    }
}

impl<'ctx> PartialOrd for Thunk<'ctx> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.seq.partial_cmp(&other.0.seq)
    }
}

