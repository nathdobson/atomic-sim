use std::rc::Rc;
use crate::ctx::Ctx;
use crate::data::DataFlow;
use crate::backtrace::{Backtrace, BacktraceFrame};
use crate::value::Value;
use crate::layout::Layout;

pub struct FlowCtx<'ctx, 'flow> {
    ctx: &'flow Rc<Ctx<'ctx>>,
    data: &'flow DataFlow<'ctx>,
    backtrace: Backtrace<'ctx>,
}

impl<'ctx, 'flow> FlowCtx<'ctx, 'flow> {
    pub fn new(ctx: &'flow Rc<Ctx<'ctx>>,
               data: &'flow DataFlow<'ctx>,
               backtrace: Backtrace<'ctx>) -> Self {
        FlowCtx { ctx, data, backtrace }
    }

    pub async fn invoke(&self, fun: &'flow Value, fargs: &[&Value]) -> Value {
        let mut thunks = Vec::with_capacity(fargs.len());
        for farg in fargs.iter() {
            thunks.push(self.data.constant((*farg).clone()).await);
        }
        self.ctx.reverse_lookup_fun(&fun)
            .call_imp(&self.with_frame(BacktraceFrame { name: "<native>" }), &thunks).await.await
    }

    pub fn with_frame(&self, frame: BacktraceFrame<'ctx>) -> Self {
        Self {
            ctx: self.ctx,
            data: self.data,
            backtrace: self.backtrace.prepend(frame),
        }
    }

    pub async fn load(&self, address: &Value, layout: Layout) -> Value {
        self.data.load(self.data.constant(address.clone()).await, layout, None).await.await
    }

    pub async fn store(&self, address: &Value, value: &Value) {
        self.data.store(self.data.constant(address.clone()).await,
                        self.data.constant(value.clone()).await,
                        None).await.await;
    }

    pub fn ctx(&self) -> &'flow Rc<Ctx<'ctx>> {
        self.ctx
    }
    pub fn data(&self) -> &'flow DataFlow<'ctx> {
        self.data
    }
    pub fn backtrace(&self) -> &Backtrace<'ctx> {
        &self.backtrace
    }
}
