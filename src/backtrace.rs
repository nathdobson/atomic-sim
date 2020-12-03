use std::rc::Rc;
use std::fmt::{Debug, Formatter};
use std::fmt;
use crate::value::Value;
use std::marker::PhantomData;
use crate::ctx::Ctx;

#[derive(Clone)]
pub enum Backtrace<'ctx> {
    Nil,
    Cons {
        frame: BacktraceFrame<'ctx>,
        next: Rc<Backtrace<'ctx>>,
    },
}

#[derive(Clone, Debug, Default)]
pub struct BacktraceFrame<'ctx> {
    //pub name: &'ctx str,
    pub ip: u64,
    pub phantom: PhantomData<&'ctx ()>,
}

pub struct Iter<'ctx, 'bt> {
    backtrace: &'bt Backtrace<'ctx>,
}

impl<'ctx> Backtrace<'ctx> {
    pub fn empty() -> Self {
        Backtrace::Nil
    }
    pub fn prepend(&self, frame: BacktraceFrame<'ctx>) -> Self {
        Backtrace::Cons {
            frame,
            next: Rc::new(self.clone()),
        }
    }
    pub fn iter(&self) -> Iter<'ctx, '_> {
        Iter { backtrace: self }
    }
    pub fn full_debug(&self, ctx: &Ctx<'ctx>) -> impl Debug + 'ctx {
        self.iter().map(|f| ctx.reverse_lookup(&ctx.value_from_address(f.ip))).collect::<Vec<_>>()
    }
}

impl<'ctx, 'bt> Iterator for Iter<'ctx, 'bt> {
    type Item = &'bt BacktraceFrame<'ctx>;

    fn next(&mut self) -> Option<Self::Item> {
        match &self.backtrace {
            Backtrace::Nil => None,
            Backtrace::Cons { frame, next } => {
                self.backtrace = next;
                Some(frame)
            }
        }
    }
}

impl<'ctx> Debug for Backtrace<'ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}