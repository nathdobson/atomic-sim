use llvm_ir::{Module, Instruction, Terminator, Function, Name, BasicBlock, Operand, ConstantRef, Constant, TypeRef, Type, HasDebugLoc, IntPredicate};
use llvm_ir::instruction::{Call, InlineAssembly, Phi, Alloca, Store, Load, Xor, SExt, BitCast, InsertValue, ZExt, AtomicRMW, Trunc, Select, PtrToInt, Sub, Or, And, IntToPtr, UDiv, SDiv, URem, SRem, Shl, LShr, AShr, ExtractValue, Mul, CmpXchg, Fence, MemoryOrdering, RMWBinOp};
use std::collections::{HashMap, BTreeMap};
use std::rc::Rc;
use std::fmt::{Debug, Formatter, Display};
use std::{fmt, iter, mem, ops};
use either::Either;
use std::cell::{Cell, RefCell, Ref};
use llvm_ir::instruction::Add;
use llvm_ir::instruction::ICmp;
use llvm_ir::constant::Constant::Undef;
use llvm_ir::types::NamedStructDef;
use llvm_ir::function::ParameterAttribute;
use std::mem::size_of;
use crate::value::{Value, add_u64_i64};
use crate::memory::{Memory};
use std::future::Future;
use std::task::{Context, Poll};
use std::pin::Pin;
use crate::data::{Thunk, DataFlow};
use crate::symbols::{Symbol, ThreadLocalKey};
use crate::backtrace::Backtrace;
use crate::flow::FlowCtx;
use crate::process::{Process};
use crate::util::future::{LocalBoxFuture, noop_waker, pending_once};
use crate::async_timer;
use crate::util::recursor::Recursor;
use crate::ordering::Ordering;

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
}

#[derive(Clone)]
pub struct Thread(Rc<ThreadInner>);

#[derive(Copy, Clone, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct ThreadId(pub usize);

impl Thread {
    pub fn new(process: Process, threadid: ThreadId, main: &Value, params: &[Value]) -> (Self, impl Future<Output=()>) {
        let data = DataFlow::new(process.clone(), threadid);
        let main = process.reverse_lookup_fun(&main).unwrap();
        let params = params.to_vec();
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
                main.await.unwrap();
                println!("Finished decoding thread");
            };
            async_timer!("Thread::new::control", inner)
        };
        (Thread(Rc::new(ThreadInner {
            process,
            threadid,
            blocker: RefCell::new(Blocker::Unknown),
            data,
            thread_locals: RefCell::new(HashMap::new()),
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
            let (layout, value) = self.0.process.thread_local_init(key);
            let ptr = self.0.process.alloc(self.0.threadid, layout);
            self.0.process.store_impl(self.0.threadid, &ptr, &value, Ordering::None);
            ptr.as_u64()
        })
    }
    pub fn threadid(&self) -> ThreadId {
        self.0.threadid
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
