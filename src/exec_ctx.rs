use crate::flow::Thread;
use crate::memory::Memory;
use crate::value::{Value, add_u64_i64};
use crate::layout::Layout;
use llvm_ir::instruction::Atomicity;
use crate::ctx::{Ctx, EvalCtx};
use llvm_ir::{Constant, TypeRef, Name, constant};

// pub struct ExecCtx<'exec, 'ctx> {
//     ctx: &'ctx Ctx<'ctx>,
//     thread: &'exec mut Thread<'ctx>,
//     memory: &'exec mut Memory<'ctx>,
// }

