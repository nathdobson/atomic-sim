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
use crate::symbols::Symbol;
use crate::backtrace::Backtrace;
use crate::flow::FlowCtx;
use crate::process::{Process};
use crate::future::{LocalBoxFuture, noop_waker};
use crate::async_timer;
use crate::recursor::Recursor;

pub struct ThreadInner {
    threadid: ThreadId,
    control: RefCell<Option<LocalBoxFuture<'static, ()>>>,
    data: DataFlow,
}

#[derive(Clone)]
pub struct Thread(Rc<ThreadInner>);

#[derive(Copy, Clone, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct ThreadId(pub usize);

impl Thread {
    pub fn new(process: Process, threadid: ThreadId, main: Symbol, params: &[Value]) -> Self {
        let data = DataFlow::new(process.clone(), threadid);
        let main = process.lookup_symbol(&main).clone();
        let main = process.reverse_lookup_fun(&main);
        let params = params.to_vec();
        let control: RefCell<Option<Pin<Box<dyn Future<Output=()>>>>> = RefCell::new(Some(Box::pin({
            let data = data.clone();
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
                main.await;
            };
            async_timer!("Thread::new::control", inner)
        })));
        Thread(Rc::new(ThreadInner { threadid, control, data }))
    }
    pub fn step(&self) -> bool {
        let step_control = {
            let mut control = self.0.control.borrow_mut();
            if let Some(fut) = &mut *control {
                if fut.as_mut().poll(&mut Context::from_waker(&noop_waker())).is_ready() {
                    *control = None;
                }
                true
            } else {
                false
            }
        };
        let step_data = self.0.data.step();
        step_control || step_data
    }
}

impl Debug for ThreadId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
