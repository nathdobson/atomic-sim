use std::cmp::Ordering;
use std::fmt::{Debug, Display, Formatter};
use std::fmt;
use std::iter::repeat;
use std::rc::Rc;

use llvm_ir::DebugLoc;

use crate::backtrace::Backtrace;
use crate::compile::function::CFunc;
use crate::data::{ComputeCtx, Thunk};
use crate::flow::FlowCtx;
use crate::interp::InterpFrame;
use crate::layout::Layout;
use crate::memory::Memory;
use crate::process::Process;
use crate::util::future::{FutureExt, LocalBoxFuture};
use crate::util::lazy::Lazy;
use crate::value::Value;

#[derive(Debug)]
pub struct Panic;

impl Display for Panic {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Panic")
    }
}

pub trait Func: Debug {
    fn name(&self) -> &str;
    fn call_imp<'a>(&'a self, flow: &'a FlowCtx, args: &'a [Thunk]) -> LocalBoxFuture<'a, Result<Thunk, Panic>>;
    fn debugloc(&self) -> Option<&DebugLoc> { None }
}

impl Func for CFunc {
    fn name(&self) -> &str {
        &self.src.name
    }

    fn debugloc(&self) -> Option<&DebugLoc> {
        self.src.debugloc.as_ref()
    }

    fn call_imp<'a>(&'a self, flow: &'a FlowCtx, args: &'a [Thunk]) -> LocalBoxFuture<'a, Result<Thunk, Panic>> {
        Box::pin(InterpFrame::call(flow, self, args.to_vec()))
    }
}

impl<F: Func> Func for Lazy<F> {
    fn name(&self) -> &str {
        self.force().unwrap().name()
    }

    fn debugloc(&self) -> Option<&DebugLoc> {
        self.force().unwrap().debugloc()
    }

    fn call_imp<'a>(&'a self, flow: &'a FlowCtx, args: &'a [Thunk]) -> LocalBoxFuture<'a, Result<Thunk, Panic>> {
        self.force().unwrap().call_imp(flow, args)
    }
}