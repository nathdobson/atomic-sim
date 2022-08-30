use std::{fmt, iter, mem, ops};
use std::cell::{Cell, Ref, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::fmt::{Debug, Display, Formatter};
use std::future::Future;
use std::mem::size_of;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use either::Either;
use llvm_ir::{BasicBlock, Constant, ConstantRef, Function, HasDebugLoc, Instruction, IntPredicate, Module, Name, Operand, Terminator, Type, TypeRef};
use llvm_ir::constant::Constant::Undef;
use llvm_ir::function::ParameterAttribute;
use llvm_ir::instruction::{Alloca, And, AShr, AtomicRMW, BitCast, Call, CmpXchg, ExtractValue, Fence, InlineAssembly, InsertValue, IntToPtr, Load, LShr, MemoryOrdering, Mul, Or, Phi, PtrToInt, RMWBinOp, SDiv, Select, SExt, Shl, SRem, Store, Sub, Trunc, UDiv, URem, Xor, ZExt};
use llvm_ir::instruction::Add;
use llvm_ir::instruction::ICmp;
use llvm_ir::types::NamedStructDef;

use crate::async_timer;
use crate::backtrace::Backtrace;
use crate::data::{DataFlow, Thunk};
use crate::flow::FlowCtx;
use crate::memory::Memory;
use crate::ordering::Ordering;
use crate::process::Process;
use crate::symbols::{Symbol, ThreadLocalKey};
use crate::util::future::{LocalBoxFuture, noop_waker, pending_once};
use crate::util::recursor::Recursor;
use crate::value::{add_u64_i64, Value};
use futures::channel::oneshot::{Receiver, channel};
use futures::future::Shared;
use futures::FutureExt;

#[derive(Debug)]
pub enum Blocker {
    Unknown,
    Pipeline,
    Cond,
    Mutex,
    Thunk(Thunk),
}

pub struct ThreadInner {
    process: Process,
    threadid: ThreadId,
    blocker: RefCell<Blocker>,
    data: DataFlow,
    thread_locals: RefCell<HashMap<ThreadLocalKey, u64>>,
    join_handle: Shared<Receiver<u64>>,
}

#[derive(Clone)]
pub struct Thread(Rc<ThreadInner>);

#[derive(Copy, Clone, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct ThreadId(pub usize);

impl Thread {
    pub fn new(process: Process, threadid: ThreadId, main: u64, params: &[Value]) -> (Self, impl Future<Output=()>) {
        let data = DataFlow::new(process.clone(), threadid);
        let main = process.definitions.reverse_lookup_fun(main).unwrap();
        let params = params.to_vec();
        let (join_signal, join_handle) = channel();
        let join_handle = join_handle.shared();
        let control = {
            let data = data.clone();
            let process = process.clone();
            let inner = async move {
                let mut deps = vec![];
                for value in params {
                    let value = value.clone();
                    deps.push(data.constant(Backtrace::empty(), value).await);
                }
                let recursor = Recursor::new();
                let flow = FlowCtx::new(process.clone(), data, Backtrace::empty(), recursor.clone());
                let main = main.call_imp(&flow, &deps);
                let main = recursor.spawn(main);
                recursor.await;
                let value = main.await.unwrap();
                join_signal.send(value.await.as_u64()).unwrap();
                println!("Finished decoding thread {:?}", threadid);
            };
            async_timer!("Thread::new::control", inner)
        };

        (Thread(Rc::new(ThreadInner {
            process,
            threadid,
            blocker: RefCell::new(Blocker::Unknown),
            data,
            thread_locals: RefCell::new(HashMap::new()),
            join_handle,
        })), control)
    }
    pub fn set_blocker(&self, blocker: Blocker) {
        *self.0.blocker.borrow_mut() = blocker;
    }
    pub async fn pending_once(&self, blocker: Blocker) {
        *self.0.blocker.borrow_mut() = blocker;
        pending_once().await;
        *self.0.blocker.borrow_mut() = Blocker::Unknown;
    }
    pub fn step_pure(&self, resolvable: &mut Vec<Thunk>) -> bool {
        self.0.data.step_pure(resolvable)
    }
    pub fn thread_local(&self, key: ThreadLocalKey) -> u64 {
        *self.0.thread_locals.borrow_mut().entry(key).or_insert_with(|| {
            let (layout, value) = self.0.process.definitions.thread_local_init(key);
            let ptr = self.0.process.memory.alloc(self.0.threadid, layout);
            self.0.process.memory.store_impl(self.0.threadid, ptr, &value, Ordering::None);
            ptr
        })
    }
    pub fn threadid(&self) -> ThreadId {
        self.0.threadid
    }
    pub async fn join(&self) -> Value {
        self.0.process.addr(self.0.join_handle.clone().await.unwrap())
    }
    pub fn alive(&self) -> bool {
        self.0.join_handle.clone().now_or_never().is_none() || self.0.data.len() > 0
    }
}

impl ThreadId {
    pub fn main() -> ThreadId {
        ThreadId(0)
    }
}

impl Debug for ThreadId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}


impl Debug for Thread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Thread")
            .field("threadid", &self.0.threadid)
            .field("blocker", &self.0.blocker)
            .field("data", &self.0.data)
            .finish()
    }
}
