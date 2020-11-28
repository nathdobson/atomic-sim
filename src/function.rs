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

pub trait Func<'ctx>: 'ctx {
    fn name(&self) -> &'ctx str;
    fn call_imp<'a>(&'a self, data: &'a DataFlow<'ctx>, args: Vec<Rc<Thunk<'ctx>>>) -> LocalBoxFuture<'a, Option<Rc<Thunk<'ctx>>>>;
}

impl<'ctx> Debug for dyn 'ctx + Func<'ctx> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{:?}", self.name())
    }
}