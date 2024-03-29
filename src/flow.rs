use std::fmt::{Debug, Formatter};
use std::fmt;
use std::rc::Rc;

use llvm_ir::DebugLoc;
use llvm_ir::instruction::MemoryOrdering;
use smallvec::smallvec;

use crate::backtrace::{Backtrace, BacktraceFrame};
use crate::compile::class::Class;
use crate::compile::expr::CExpr;
use crate::data::{DataFlow, Thunk};
use crate::function::Panic;
use crate::layout::Layout;
use crate::ordering::Ordering;
use crate::process::Process;
use crate::thread::{Thread, ThreadId};
use crate::util::recursor::Recursor;
use crate::value::Value;

#[derive(Clone)]
pub struct FlowCtx {
    process: Process,
    data: DataFlow,
    backtrace: Backtrace,
    recursor: Recursor,
}

impl FlowCtx {
    pub fn new(process: Process,
               data: DataFlow,
               backtrace: Backtrace,
               recursor: Recursor,
    ) -> Self {
        FlowCtx { process, data, backtrace, recursor }
    }

    pub async fn invoke(&self, fun: u64, fargs: &[&Value]) -> Result<Value, Panic> {
        let mut thunks = Vec::with_capacity(fargs.len());
        for farg in fargs.iter() {
            thunks.push(self.data.constant(self.backtrace.clone(), (*farg).clone()).await);
        }
        let fun = self.process.definitions.reverse_lookup_fun(fun).unwrap();
        Ok(fun.call_imp(self, &thunks).await?.await)
    }

    pub fn with_frame(&self, frame: BacktraceFrame) -> Self {
        FlowCtx {
            process: self.process.clone(),
            data: self.data.clone(),
            backtrace: self.backtrace.prepend(frame),
            recursor: self.recursor.clone(),
        }
    }

    pub async fn alloc(&self, layout: Layout) -> u64 {
        let threadid = self.data.threadid();
        let process = self.process.clone();
        self.data.thunk(self.backtrace.clone(), smallvec![], move |comp, _| {
            process.addr(process.memory.alloc(threadid, layout))
        }).await.await.as_u64()
    }

    pub async fn free(&self, ptr: u64, layout: Layout) {
        let threadid = self.data.threadid();
        let process = self.process.clone();
        let ptr = ptr.clone();
        self.data.thunk(self.backtrace.clone(), smallvec![], move |comp, _| {
            process.memory.free(threadid, ptr);
            Value::from(())
        }).await.await;
    }

    pub async fn load(&self, address: u64, layout: Layout) -> Value {
        self.data.load(self.backtrace.clone(),
                       self.data.constant(
                           self.backtrace.clone(),
                           self.process.addr(address)).await,
                       layout, Ordering::None).await.await
    }

    pub async fn store(&self, address: u64, value: &Value) {
        self.data.store(
            self.backtrace.clone(),
            self.data.constant(
                self.backtrace.clone(),
                self.process.addr(address)).await,
            Layout::from_bytes(value.as_bytes().len() as u64, 1),
            self.data.constant(self.backtrace.clone(), value.clone()).await,
            Ordering::None).await.await;
    }

    pub async fn string(&self, string: &str) -> u64 {
        let mut cstr = string.as_bytes().to_vec();
        cstr.push(0);
        let layout = Layout::from_bytes(cstr.len() as u64, 1);
        let res = self.alloc(layout).await;
        self.store(res, &Value::from_bytes(&cstr, layout.bits())).await;
        res
    }

    pub async fn strlen(&self, string: u64) -> u64 {
        for i in 0.. {
            let ptr = string + i;
            let v = self.load(ptr, Layout::from_bytes(1, 1)).await;
            if v.unwrap_u8() as u32 == 0 {
                return i;
            }
        }
        panic!()
    }

    pub async fn get_string(&self, string: u64) -> String {
        let len = self.strlen(string).await;
        String::from_utf8(
            self.load(string,
                      Layout::from_bytes(len, 1)).await.as_bytes().to_vec()).unwrap()
    }

    pub async fn memcpy(&self, dst: u64, src: u64, len: u64) {
        if len > 0 {
            let value = self.load(src, Layout::from_bytes(len, 1)).await;
            self.store(dst, &value).await;
        }
    }
    pub async fn realloc(&self, old: u64, old_layout: Layout, new_layout: Layout) -> u64 {
        let new = self.alloc(new_layout).await;
        self.memcpy(new, old, new_layout.bytes().min(old_layout.bytes())).await;
        self.free(old, old_layout).await;
        new
    }

    pub async fn constant(&self, constant: Value) -> Thunk {
        self.data.constant(self.backtrace.clone(), constant).await
    }
    pub fn evaluate(&self, expr: &CExpr) -> Value {
        match expr {
            CExpr::Const { class, value } => value.clone(),
            CExpr::ThreadLocal { class, key } =>
                self.process.addr(self.process.thread(self.data.threadid()).unwrap().thread_local(*key)),
            CExpr::Apply { oper, args } => {
                let x = args.iter().map(|arg| self.evaluate(arg)).collect::<Vec<_>>();
                oper.call_operation(&x.iter().collect::<Vec<_>>())
            }
        }
    }

    pub async fn cmpxchg(
        &self,
        class: &Class,
        address: Thunk,
        expected: Thunk,
        replacement: Thunk,
        success: Ordering,
        failure: Ordering,
    ) -> Thunk {
        let deps = smallvec![address.clone(), expected, replacement];
        let types = self.process.types.clone();
        let output = types.struc(vec![class.clone(), types.bool()], false);
        let layout = class.layout();
        self.data().thunk_impl(
            self.backtrace().clone(),
            Some((address.clone(), layout)),
            success.union(failure),
            deps, {
                move |comp, args| {
                    let current = comp.process.memory.load_impl(comp.threadid, args[0].as_u64(), layout, failure).clone();
                    let changed = &current == args[1];
                    if changed {
                        comp.process.memory.store_impl(comp.threadid, args[0].as_u64(), args[2], success);
                    }
                    Value::aggregate(&output, vec![current, Value::from(changed)].into_iter())
                }
            }).await
    }

    pub async fn cmpxchg_u64(&self,
                             address: u64,
                             expected: u64,
                             replacement: u64,
                             success: Ordering,
                             failure: Ordering) -> (u64, bool) {
        let c64 = self.process().types.int(64);
        let c1 = self.process().types.int(1);
        let c64_1 = self.process().types.struc(vec![c64.clone(), c1], false);
        let address = self.constant(self.process.addr(address)).await;
        let expected = self.constant(Value::from(expected)).await;
        let replacement = self.constant(Value::from(replacement)).await;
        let output = self.cmpxchg(&c64,
                                  address.clone(),
                                  expected.clone(),
                                  replacement.clone(),
                                  success,
                                  failure).await.await;
        (output.extract(&c64_1, 0).unwrap_u64(),
         output.extract(&c64_1, 1).unwrap_bool())
    }

    pub async fn atomicrmw(
        &self,
        class: Class,
        address: Thunk,
        value: Thunk,
        atomicity: Ordering,
        oper: impl FnOnce(&Value, &Value) -> Value + 'static,
    ) -> Thunk {
        let deps = smallvec![address.clone(), value];
        let layout = class.layout();
        self.data().thunk_impl(
            self.backtrace().clone(),
            Some((address.clone(), layout)),
            atomicity,
            deps, {
                move |comp, args| {
                    let current = comp.process.memory.load_impl(comp.threadid, args[0].as_u64(), layout, atomicity).clone();
                    let new = oper(&current, &args[1]);
                    comp.process.memory.store_impl(comp.threadid, args[0].as_u64(), &new, atomicity);
                    current
                }
            }).await
    }

    pub fn process(&self) -> &Process { &self.process }
    pub fn data(&self) -> &DataFlow {
        &self.data
    }
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
    pub fn recursor(&self) -> &Recursor { &self.recursor }
    pub fn threadid(&self) -> ThreadId { self.data.threadid() }
}


impl Debug for FlowCtx {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for frame in self.backtrace.iter() {
            write!(f, "{:?}\n", self.process.symbols.reverse_lookup(frame.ip()))?;
        }
        Ok(())
    }
}