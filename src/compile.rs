use llvm_ir::{Function, BasicBlock, Name};
use std::collections::HashMap;
use crate::ctx::{EvalCtx, Ctx};
use crate::function::Func;
use crate::data::{DataFlow, Thunk};
use std::rc::Rc;
use futures::future::LocalBoxFuture;
use crate::frame::Frame;

#[derive(Debug)]
pub struct CompiledFunc<'ctx> {
    pub src: &'ctx Function,
    pub ectx: EvalCtx,
}

impl<'ctx> CompiledFunc<'ctx> {
    pub fn new(ectx: EvalCtx, fun: &'ctx Function) -> Self {
        CompiledFunc {
            src: &fun,
            ectx: ectx,
        }
    }
}

impl<'ctx> Func<'ctx> for CompiledFunc<'ctx> {
    fn name(&self) -> &'ctx str {
        self.src.name.as_str()
    }

    fn call_imp<'a>(&'a self, data: &'a DataFlow<'ctx>, args: Vec<Rc<Thunk<'ctx>>>) -> LocalBoxFuture<'a, Option<Rc<Thunk<'ctx>>>> {
        Box::pin(Frame::call(self.ectx, data, self.src, args))
        // Box::pin(async {
        //     let result = Frame {
        //         ctx: self.ctx.clone(),
        //         fun: self.ctx.functions.get(self.src.n).unwrap(),
        //         origin: None,
        //         temps,
        //         allocs: vec![],
        //         result: dest,
        //     }.decode(data).await;
        //     self.add_thunk(data, dest, result.into_iter().collect(), |args| {
        //         args.args.iter().next().cloned().cloned().unwrap_or(Value::from(()))
        //     }).await;
        // })
    }
}
