use crate::memory::Memory;
use crate::value::Value;
use std::fmt::{Debug, Formatter};
use std::fmt;
use crate::layout::Layout;
use std::iter::repeat;
use crate::process::Process;
use std::cmp::Ordering;
use crate::data::{Thunk};
use std::rc::Rc;
use futures::future::LocalBoxFuture;
use crate::backtrace::Backtrace;
use crate::flow::FlowCtx;
use llvm_ir::DebugLoc;

pub trait Func: Debug {
    fn name(&self) -> &str;
    fn call_imp<'a>(&'a self, flow: &'a FlowCtx, args: &'a [Thunk]) -> LocalBoxFuture<'a, Thunk>;
    fn debugloc(&self) -> Option<&DebugLoc> { None }
}