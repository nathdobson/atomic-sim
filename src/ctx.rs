use std::fmt::Debug;
use std::fmt;
use llvm_ir::{Module, Function, Name, BasicBlock, TypeRef, Type, Constant, constant, Operand};
use std::collections::HashMap;
use llvm_ir::types::NamedStructDef;
use crate::value::{Value, add_u64_i64};
use crate::layout::{Layout, AggrLayout, Packing, align_to};
use crate::memory::{Memory};
use crate::function::Func;
use crate::symbols::{SymbolTable, Symbol};
use crate::interp::InterpFunc;
use crate::process::Process;
use std::rc::Rc;
use llvm_ir::module::GlobalAlias;
use std::borrow::Cow;

pub struct Ctx<'ctx> {
}

#[derive(Copy, Clone, Debug, Default)]
pub struct EvalCtx {
    pub module: Option<usize>,
}

impl<'ctx> Ctx<'ctx> {

}

impl<'ctx> Debug for Ctx<'ctx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ctx")
            .field("module", &DebugModule(&self.modules))
            //.field("functions", &self.functions)
            .finish()
    }
}
