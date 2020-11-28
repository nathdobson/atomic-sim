use llvm_ir::{Module, Instruction, Terminator, Function, Name, BasicBlock, Operand, ConstantRef, Constant, TypeRef, Type, HasDebugLoc, IntPredicate};
use llvm_ir::instruction::{Call, InlineAssembly, Phi, Alloca, Store, Load, Xor, SExt, BitCast, InsertValue, ZExt, AtomicRMW, Trunc, Select, PtrToInt, Sub, Or, And, IntToPtr, UDiv, SDiv, URem, SRem, Shl, LShr, AShr, ExtractValue, Mul, CmpXchg, Fence, MemoryOrdering, RMWBinOp};
use std::collections::{HashMap, BTreeMap};
use std::rc::Rc;
use std::fmt::{Debug, Formatter, Display};
use std::{fmt, iter, mem, ops};
use std::borrow::BorrowMut;
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
use crate::ctx::{Ctx, EvalCtx, ThreadCtx};
use crate::process::Process;
use std::future::Future;
use std::task::{Context, Poll};
use std::pin::Pin;
use crate::data::{DataFlow, ComputeArgs};
use futures::{pending, FutureExt};
use crate::data::Thunk;
use futures::future::LocalBoxFuture;
use futures::task::noop_waker_ref;
use crate::frame::Frame;
use crate::symbols::Symbol;

pub struct Thread<'ctx> {
    threadid: usize,
    control: LocalBoxFuture<'ctx, ()>,
    data: Rc<DataFlow<'ctx>>,
}


impl<'ctx> Thread<'ctx> {
    pub fn new(ctx: Rc<Ctx<'ctx>>, main: Symbol<'ctx>, threadid: usize, params: &[Value]) -> Self {
        let data = Rc::new(DataFlow::new(ctx.clone(), threadid));
        let main = ctx.functions.get(&main).unwrap().clone();
        let params = params.to_vec();
        let control = Box::pin({
            let data = data.clone();
            async move {
                let mut deps = vec![];
                for value in params {
                    let value = value.clone();
                    deps.push(data.add_thunk(vec![], |args| { value }).await);
                }
                main.call_imp(&*data, deps).await;
            }
        });
        Thread { threadid, control, data }
    }
    pub fn step(&mut self) -> impl FnOnce(&mut Process<'ctx>) -> bool {
        let step_control = self.control.as_mut().poll(&mut Context::from_waker(noop_waker_ref())).is_pending();
        let cont = (self.data.step(ThreadCtx { threadid: self.threadid }));
        move |process| {
            let step_data = cont(process);
            step_control || step_data
        }
    }
}

