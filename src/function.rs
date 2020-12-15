use crate::memory::Memory;
use crate::value::Value;
use std::fmt::{Debug, Formatter};
use std::fmt;
use crate::layout::Layout;
use std::iter::repeat;
use crate::process::Process;
use std::cmp::Ordering;
use crate::data::{Thunk, ComputeCtx};
use std::rc::Rc;
use crate::backtrace::Backtrace;
use crate::flow::FlowCtx;
use llvm_ir::DebugLoc;
use crate::compile::CFunc;
use crate::interp::InterpFrame;
use crate::future::{LocalBoxFuture, FutureExt};
use crate::lazy::Lazy;

pub trait Func: Debug {
    fn name(&self) -> &str;
    fn call_imp<'a>(&'a self, flow: &'a FlowCtx, args: &'a [Thunk]) -> LocalBoxFuture<'a, Thunk>;
    fn debugloc(&self) -> Option<&DebugLoc> { None }
}

struct NativeComp {
    name: String,
    imp: Rc<dyn 'static + Fn(&ComputeCtx, &[&Value]) -> Value>,
}

struct NativeExec {
    name: String,
    imp: Rc<dyn 'static + for<'flow> Fn(&'flow FlowCtx, &'flow [Value]) -> LocalBoxFuture<'flow, Value>>,
}

#[derive(Debug)]
pub struct NativeBomb {
    pub name: String,
}

impl Func for CFunc {
    fn name(&self) -> &str {
        &self.src.name
    }

    fn debugloc(&self) -> Option<&DebugLoc> {
        self.src.debugloc.as_ref()
    }

    fn call_imp<'a>(&'a self, flow: &'a FlowCtx, args: &'a [Thunk]) -> LocalBoxFuture<'a, Thunk> {
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

    fn call_imp<'a>(&'a self, flow: &'a FlowCtx, args: &'a [Thunk]) -> LocalBoxFuture<'a, Thunk> {
        self.force().unwrap().call_imp(flow, args)
    }
}

impl Func for NativeComp {
    fn name(&self) -> &str {
        &self.name
    }


    fn call_imp<'flow>(&'flow self, flow: &'flow FlowCtx, args: &'flow [Thunk]) -> LocalBoxFuture<'flow, Thunk> {
        let imp = self.imp.clone();
        Box::pin(flow.data().thunk(flow.backtrace().clone(), args.iter().cloned().collect(),
                                   move |comp: &ComputeCtx, args| {
                                       imp(comp, args)
                                   }))
    }
}

impl Func for NativeBomb {
    fn name(&self) -> &str {
        &self.name
    }

    fn call_imp<'a>(&'a self, flow: &'a FlowCtx, args: &'a [Thunk]) -> LocalBoxFuture<'a, Thunk> {
        todo!("{:?}", self.name)
    }

    fn debugloc(&self) -> Option<&DebugLoc> {
        None
    }
}

impl Func for NativeExec {
    fn name(&self) -> &str {
        &self.name
    }


    fn call_imp<'flow>(&'flow self, flow: &'flow FlowCtx, args: &'flow [Thunk]) -> LocalBoxFuture<'flow, Thunk> {
        Box::pin(async move {
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                values.push(arg.clone().await);
            }
            flow.data().constant(flow.backtrace().clone(), (self.imp)(flow, &values).await).await
        })
    }

    fn debugloc(&self) -> Option<&DebugLoc> {
        unimplemented!()
    }
}

pub fn native_comp_new<const N: usize>(
    name: &str,
    imp: impl 'static + Fn(&ComputeCtx, [Value; N]) -> Value)
    -> Rc<dyn 'static + Func>
    where [Value; N]: Default {
    let name2 = name.to_string();
    Rc::new(NativeComp {
        name: name.to_string(),
        imp: Rc::new(move |comp, args| {
            assert_eq!(args.len(), N, "{:?}", name2);
            let mut array = <[Value; N]>::default();
            for (p, a) in array.iter_mut().zip(args.iter()) {
                *p = (*a).clone();
            }
            imp(comp, array)
        }),
    })
}

pub fn native_exec_new<const N: usize, F>(
    name: &str,
    imp: F)
    -> Rc<dyn Func>
    where [Value; N]: Default,
          F: 'static + for<'flow> Fn(&'flow FlowCtx, [Value; N]) -> LocalBoxFuture<'flow, Value>
{
    let imp = Rc::new(imp);
    Rc::new(NativeExec {
        name: name.to_string(),
        imp: Rc::new(move |flow, args| {
            let imp = imp.clone();
            async move {
                assert_eq!(args.len(), N);
                let mut array = <[Value; N]>::default();
                array.clone_from_slice(args);
                imp(flow, array).await
            }
        }.boxed_local()),
    })
}

impl Debug for NativeComp {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl Debug for NativeExec {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}