use std::rc::Rc;
use crate::ctx::Ctx;
use crate::data::DataFlow;
use crate::backtrace::{Backtrace, BacktraceFrame};
use crate::value::Value;
use crate::layout::Layout;
use std::fmt::{Debug, Formatter};
use std::fmt;

#[derive(Clone)]
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
            thunks.push(self.data.constant(self.backtrace.clone(), (*farg).clone()).await);
        }
        self.ctx.reverse_lookup_fun(&fun)
            .call_imp(&self.with_frame(BacktraceFrame { ip: 0xFFFF_FFFF, ..BacktraceFrame::default() }), &thunks).await.await
    }

    pub fn with_frame(&self, frame: BacktraceFrame<'ctx>) -> Self {
        Self {
            ctx: self.ctx,
            data: self.data,
            backtrace: self.backtrace.prepend(frame),
        }
    }

    pub async fn alloc(&self, layout: Layout) -> Value {
        self.data.thunk(format!("alloc({:?})", layout), self.backtrace.clone(), vec![], move |comp, _| {
            comp.process.alloc(comp.threadid, layout)
        }).await.await
    }

    pub async fn load(&self, address: &Value, layout: Layout) -> Value {
        self.data.load(self.backtrace.clone(),
                       self.data.constant(self.backtrace.clone(),
                                          address.clone()).await, layout, None).await.await
    }

    pub async fn store(&self, address: &Value, value: &Value) {
        self.data.store(self.backtrace.clone(),self.data.constant(self.backtrace.clone(),address.clone()).await,
                        self.data.constant(self.backtrace.clone(),value.clone()).await,
                        None).await.await;
    }

    pub async fn string(&self, string: &str) -> Value {
        let mut cstr = string.as_bytes().to_vec();
        cstr.push(0);
        let layout = Layout::from_bytes(cstr.len() as u64, 1);
        let res = self.alloc(layout).await;
        self.store(&res, &Value::from_bytes(&cstr, layout)).await;
        res
    }

    pub async fn strlen(&self, string: &Value) -> Value {
        for i in 0.. {
            let ptr = self.ctx().value_from_address(string.as_u64() + i);
            let v = self.load(&ptr, Layout::from_bytes(1, 1)).await;
            if v.unwrap_u8() as u32 == 0 {
                return self.ctx().value_from_address(i);
            }
        }
        panic!()
    }

    pub async fn get_string(&self, string: &Value) -> String {
        let len = self.strlen(string).await;
        String::from_utf8(
            self.load(string,
                      Layout::from_bytes(len.as_u64(), 1)).await.as_bytes().to_vec()).unwrap()
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


impl<'ctx, 'flow> Debug for FlowCtx<'ctx, 'flow> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for frame in self.backtrace.iter() {
            write!(f, "{:?}\n", self.ctx.reverse_lookup(&self.ctx.value_from_address(frame.ip)))?;
        }
        Ok(())
    }
}