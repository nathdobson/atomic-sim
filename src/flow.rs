use std::rc::Rc;
use crate::backtrace::{Backtrace, BacktraceFrame};
use crate::value::Value;
use crate::layout::Layout;
use std::fmt::{Debug, Formatter};
use std::fmt;
use crate::data::DataFlow;
use crate::process::{Process};
use llvm_ir::DebugLoc;

#[derive(Clone)]
pub struct FlowCtx {
    process: Process,
    data: DataFlow,
    backtrace: Backtrace,
}

impl FlowCtx {
    pub fn new(process: Process,
               data: DataFlow,
               backtrace: Backtrace) -> Self {
        FlowCtx { process, data, backtrace }
    }

    pub async fn invoke(&self, fun: &Value, fargs: &[&Value]) -> Value {
        let mut thunks = Vec::with_capacity(fargs.len());
        for farg in fargs.iter() {
            thunks.push(self.data.constant(self.backtrace.clone(), (*farg).clone()).await);
        }
        self.process.reverse_lookup_fun(&fun)
            .call_imp(&self.with_frame(BacktraceFrame::new(0xFFFF_FFFF, DebugLoc {
                line: 0,
                col: None,
                filename: "<native>".to_string(),
                directory: None,
            })), &thunks).await.await
    }

    pub fn with_frame(&self, frame: BacktraceFrame) -> Self {
        FlowCtx {
            process: self.process.clone(),
            data: self.data.clone(),
            backtrace: self.backtrace.prepend(frame),
        }
    }

    pub async fn alloc(&self, layout: Layout) -> Value {
        let threadid = self.data.threadid();
        self.data.thunk(format!("alloc({:?})", layout), self.backtrace.clone(), vec![], move |comp, _| {
            comp.process.alloc(threadid, layout)
        }).await.await
    }

    pub async fn load(&self, address: &Value, layout: Layout) -> Value {
        self.data.load(self.backtrace.clone(),
                       self.data.constant(self.backtrace.clone(),
                                          address.clone()).await, layout, None).await.await
    }

    pub async fn store(&self, address: &Value, value: &Value) {
        self.data.store(self.backtrace.clone(), self.data.constant(self.backtrace.clone(), address.clone()).await,
                        self.data.constant(self.backtrace.clone(), value.clone()).await,
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
            let ptr = self.process.value_from_address(string.as_u64() + i);
            let v = self.load(&ptr, Layout::from_bytes(1, 1)).await;
            if v.unwrap_u8() as u32 == 0 {
                return self.process.value_from_address(i);
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

    pub fn data(&self) -> &DataFlow {
        &self.data
    }
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}


impl Debug for FlowCtx {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for frame in self.backtrace.iter() {
            write!(f, "{:?}\n", self.process.reverse_lookup(&self.process.value_from_address(frame.ip())))?;
        }
        Ok(())
    }
}