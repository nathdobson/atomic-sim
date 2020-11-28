use futures::pending;
use crate::ctx::{ThreadCtx, Ctx};
use crate::value::Value;
use crate::process::Process;
use std::fmt::{Formatter, Debug};
use std::{fmt, mem};
use crate::thread::{Thread};
use std::collections::{HashMap, BTreeMap};
use std::rc::Rc;
use std::cell::{RefCell, Ref};
use std::borrow::BorrowMut;

const PIPELINE_SIZE: usize = 1;

pub struct DataFlowInner<'ctx> {
    threadid: usize,
    seq: usize,
    thunks: BTreeMap<usize, Rc<Thunk<'ctx>>>,
}

pub struct DataFlow<'ctx> {
    ctx: Rc<Ctx<'ctx>>,
    inner: RefCell<DataFlowInner<'ctx>>,
}

pub struct ComputeArgs<'ctx, 'comp> {
    pub process: &'comp mut Process<'ctx>,
    pub tctx: ThreadCtx,
    pub args: &'comp [&'comp Value],
    pub load: Option<&'comp Value>,
    pub store: &'comp mut Option<Value>,
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
    Pending(Box<dyn 'ctx + for<'a> FnOnce(ComputeArgs<'ctx, 'a>) -> Value>),
    Ready(Value),
    Sandbag,
}

pub struct Thunk<'ctx> {
    pub tctx: ThreadCtx,
    pub seq: usize,
    pub deps: Vec<Rc<Thunk<'ctx>>>,
    pub address: Option<Rc<Thunk<'ctx>>>,
    pub load: Option<LoadOrdering>,
    pub store: Option<(StoreOrdering, StoreOrdering)>,
    pub value: RefCell<ThunkState<'ctx>>,
}


impl<'ctx> Thunk<'ctx> {
    pub async fn get(&self) -> &Value {
        loop {
            if let Some(v) = self.try_get() {
                return v;
            } else {
                pending!();
            }
        }
    }
    pub fn try_get(&self) -> Option<&Value> {
        let r = self.value.borrow();
        match &*r {
            ThunkState::Pending(_) => return None,
            ThunkState::Ready(_) => {}
            ThunkState::Sandbag => unreachable!()
        }
        let r = Ref::leak(r);
        match r {
            ThunkState::Pending(_) => unreachable!(),
            ThunkState::Ready(value) => Some(value),
            ThunkState::Sandbag => unreachable!(),
        }
    }
    pub fn step(&self, process: &mut Process<'ctx>, tctx: ThreadCtx) -> bool {
        let mut args = self.deps.iter().map(|d| d.try_get().unwrap()).collect::<Vec<_>>();
        match &*self.value.borrow() {
            ThunkState::Ready(_) => panic!("Already ready"),
            ThunkState::Pending(_) => {}
            ThunkState::Sandbag => unreachable!(),
        }
        let mut r = self.value.borrow_mut();
        match mem::replace(&mut *r, ThunkState::Sandbag) {
            ThunkState::Pending(compute) => {
                *r = ThunkState::Ready(compute(ComputeArgs {
                    process,
                    tctx,
                    args: args.as_slice(),
                    load: None,
                    store: &mut None,
                }));
            }
            ThunkState::Ready(_) => panic!("Stepping ready thunk."),
            ThunkState::Sandbag => unreachable!(),
        }
        true
    }
}

impl<'ctx> DataFlow<'ctx> {
    pub fn new(ctx: Rc<Ctx<'ctx>>, threadid: usize) -> Self {
        DataFlow {
            ctx,
            inner: RefCell::new(DataFlowInner { threadid, seq: 0, thunks: BTreeMap::new() }),
        }
    }
    pub async fn add_thunk(&self, deps: Vec<Rc<Thunk<'ctx>>>, compute: impl 'ctx + for<'a> FnOnce(ComputeArgs<'ctx, 'a>) -> Value) -> Rc<Thunk<'ctx>> {
        while self.inner.borrow().thunks.len() >= PIPELINE_SIZE {
            pending!();
        }
        let mut this = self.inner.borrow_mut();
        let seq = this.seq;
        this.seq += 1;
        let thunk: Rc<Thunk<'ctx>> = Rc::new(Thunk {
            tctx: ThreadCtx { threadid: this.threadid },
            seq,
            deps,
            address: None,
            load: None,
            store: None,
            value: RefCell::new(ThunkState::Pending(Box::new(compute))),
        });
        this.thunks.insert(seq, thunk.clone());
        thunk
    }
    pub fn len(&self) -> usize {
        self.inner.borrow().thunks.len()
    }
    pub fn step(&self, tctx: ThreadCtx) -> impl FnOnce(&mut Process<'ctx>) -> bool {
        let thunk = self.inner.borrow_mut().thunks.pop_first();
        move |process| {
            if let Some((_, thunk)) = thunk {
                thunk.step(process, tctx);
                true
            } else {
                false
            }
        }
    }
    pub fn ctx(&self) -> &Rc<Ctx<'ctx>> {
        &self.ctx
    }
}

impl<'ctx, 'comp> ComputeArgs<'ctx, 'comp> {
    pub fn ctx(&self) -> &Ctx<'ctx> {
        &*self.process.ctx
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
        f.debug_list().entries(self.0.iter().map(|thunk| thunk.seq)).finish()
    }
}


impl<'ctx> Debug for Thunk<'ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{} = {:?} [{:?}]", self.tctx.threadid, self.seq, self.value.borrow(), self.deps.iter().map(|dep| dep.seq).collect::<Vec<_>>())
    }
}

impl<'ctx> Debug for ThunkState<'ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ThunkState::Pending(_) => write!(f, "ThunkState::Pending"),
            ThunkState::Ready(value) => write!(f, "ThunkState::Ready({:?})", value),
            ThunkState::Sandbag => write!(f, "ThunkState::Pending"),
        }
    }
}