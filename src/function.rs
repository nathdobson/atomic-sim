use crate::memory::Memory;
use crate::value::Value;
use crate::ctx::{Ctx};
use std::fmt::{Debug, Formatter};
use std::fmt;
use crate::layout::Layout;
use std::iter::repeat;
use crate::process::Process;
use std::hint::unreachable_unchecked;
use std::cmp::Ordering;
use crate::data::{DataFlow, Thunk};
use std::rc::Rc;
use futures::future::LocalBoxFuture;
use crate::backtrace::Backtrace;
use crate::flow::FlowCtx;
use llvm_ir::DebugLoc;

pub trait Func<'ctx>: 'ctx {
    fn name(&self) -> &'ctx str;
    fn call_imp<'flow>(&'flow self, flow: &'flow FlowCtx<'ctx, 'flow>, args: &'flow [Thunk<'ctx>]) -> LocalBoxFuture<'flow, Thunk<'ctx>>;
    fn debugloc(&self) -> Option<&'ctx DebugLoc> { None }
}

impl<'ctx> Debug for dyn 'ctx + Func<'ctx> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{:?}", self.name())
    }
}