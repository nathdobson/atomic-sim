use crate::memory::Memory;
use crate::value::Value;
use crate::ctx::{Ctx, ThreadCtx};
use std::fmt::{Debug, Formatter};
use std::fmt;
use crate::layout::Layout;
use std::iter::repeat;
use crate::process::Process;
use std::hint::unreachable_unchecked;
use std::cmp::Ordering;
use crate::data::{ComputeArgs, DataFlow, Thunk};
use std::rc::Rc;
use futures::future::LocalBoxFuture;
use crate::backtrace::Backtrace;

pub struct ExecArgs<'ctx, 'exec> {
    pub ctx: &'exec Rc<Ctx<'ctx>>,
    pub data: &'exec DataFlow<'ctx>,
    pub backtrace: &'exec Backtrace<'ctx>,
    pub args: Vec<Thunk<'ctx>>,
}

pub trait Func<'ctx>: 'ctx {
    fn name(&self) -> &'ctx str;
    fn call_imp<'a>(&'a self, args: ExecArgs<'ctx, 'a>) -> LocalBoxFuture<'a, Thunk<'ctx>>;
}

impl<'ctx> Debug for dyn 'ctx + Func<'ctx> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{:?}", self.name())
    }
}