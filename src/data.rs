use futures::pending;
use crate::ctx::{Ctx};
use crate::value::Value;
use crate::process::Process;
use std::fmt::{Formatter, Debug};
use std::{fmt, mem};
use crate::thread::{Thread, ThreadId};
use std::collections::{HashMap, BTreeMap};
use std::rc::Rc;
use std::cell::{RefCell, Ref};
use std::borrow::BorrowMut;
use llvm_ir::instruction::Atomicity;
use crate::layout::Layout;
use std::future::Future;
use futures::task::{Context, Poll};
use std::pin::Pin;
use crate::compute::ComputeCtx;

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
    pub value: RefCell<ThunkState<'ctx>>,
}

#[derive(Clone)]
pub struct Thunk<'ctx>(Rc<ThunkInner<'ctx>>);

impl<'ctx> Thunk<'ctx> {
    // pub async fn get(&self) -> &Value {
    //     loop {
    //         if let Some(v) = self.try_get() {
    //             return v;
    //         } else {
    //             pending!();
    //         }
    //     }
    // }
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
                *r = ThunkState::Ready(compute(comp, args.as_slice()));
            }
            ThunkState::Ready(_) => panic!("Stepping ready thunk."),
            _ => unreachable!(),
        }
        true
    }
}

impl<'ctx> DataFlow<'ctx> {
    pub fn new(threadid: ThreadId) -> Self {
        DataFlow {
            inner: RefCell::new(DataFlowInner { threadid, seq: 0, thunks: BTreeMap::new() }),
        }
    }
    pub async fn thunk(&self, deps: Vec<Thunk<'ctx>>, compute: impl 'ctx + for<'a> FnOnce(ComputeCtx<'ctx, 'a>, &'a [&'a Value]) -> Value) -> Thunk<'ctx> {
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
            value: RefCell::new(ThunkState::Pending(Box::new(compute))),
        }));
        this.thunks.insert(seq, thunk.clone());
        thunk
    }
    pub async fn constant(&self, v: Value) -> Thunk<'ctx> {
        self.thunk(vec![], |_, _| v).await
    }
    pub async fn store<'a>(&'a self, address: Thunk<'ctx>, value: Thunk<'ctx>, atomicity: Option<&'ctx Atomicity>) -> Thunk<'ctx> {
        self.thunk(vec![address, value], move |comp, args| {
            comp.process.store(comp.threadid, args[0], args[1], atomicity);
            Value::from(())
        }).await
    }
    pub async fn load<'a>(&'a self, address: Thunk<'ctx>, layout: Layout, atomicity: Option<&'ctx Atomicity>) -> Thunk<'ctx> {
        self.thunk(vec![address], move |comp, args| {
            comp.process.load(comp.threadid, args[0], layout, atomicity)
        }).await
    }
    pub fn len(&self) -> usize {
        self.inner.borrow().thunks.len()
    }
    pub fn step(&self, threadid: ThreadId) -> impl FnOnce(&mut Process<'ctx>) -> bool {
        let thunk = self.inner.borrow_mut().thunks.pop_first();
        move |process| {
            if let Some((_, thunk)) = thunk {
                thunk.step(ComputeCtx{ process: process, threadid });
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
        write!(f, "{:?}_{} = {:?} [{:?}]", self.0.threadid, self.0.seq, self.0.value.borrow(), self.0.deps.iter().map(|dep| dep.0.seq).collect::<Vec<_>>())
    }
}

impl<'ctx> Debug for ThunkState<'ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ThunkState::Pending(_) => write!(f, "ThunkState::Pending"),
            ThunkState::Ready(value) => write!(f, "ThunkState::Ready({:?})", value),
            ThunkState::Sandbag => write!(f, "ThunkState::Pending"),
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
