use std::rc::Rc;
use crate::backtrace::{Backtrace, BacktraceFrame};
use crate::value::Value;
use crate::layout::Layout;
use std::fmt::{Debug, Formatter};
use std::fmt;
use crate::data::{DataFlow, Thunk};
use crate::process::{Process};
use llvm_ir::DebugLoc;
use smallvec::smallvec;
use crate::util::recursor::Recursor;
use crate::function::Panic;
use crate::compile::expr::CExpr;
use crate::thread::{Thread, ThreadId};
use crate::compile::class::Class;
use llvm_ir::instruction::MemoryOrdering;

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

    pub async fn invoke(&self, fun: &Value, fargs: &[&Value]) -> Result<Value, Panic> {
        let mut thunks = Vec::with_capacity(fargs.len());
        for farg in fargs.iter() {
            thunks.push(self.data.constant(self.backtrace.clone(), (*farg).clone()).await);
        }
        let fun = self.process.reverse_lookup_fun(&fun).unwrap();
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

    pub async fn alloc(&self, layout: Layout) -> Value {
        let threadid = self.data.threadid();
        let process = self.process.clone();
        self.data.thunk(self.backtrace.clone(), smallvec![], move |comp, _| {
            process.alloc(threadid, layout)
        }).await.await
    }

    pub async fn free(&self, ptr: &Value, layout: Layout) -> Value {
        let threadid = self.data.threadid();
        let process = self.process.clone();
        let ptr = ptr.clone();
        self.data.thunk(self.backtrace.clone(), smallvec![], move |comp, _| {
            process.free(threadid, &ptr);
            Value::from(())
        }).await.await
    }

    pub async fn load(&self, address: &Value, layout: Layout) -> Value {
        self.data.load(self.backtrace.clone(),
                       self.data.constant(self.backtrace.clone(),
                                          address.clone()).await, layout, None).await.await
    }

    pub async fn store(&self, address: &Value, value: &Value) {
        self.data.store(
            self.backtrace.clone(),
            self.data.constant(self.backtrace.clone(), address.clone()).await,
            Layout::from_bytes(value.as_bytes().len() as u64, 1),
            self.data.constant(self.backtrace.clone(), value.clone()).await,
            None).await.await;
    }

    pub async fn string(&self, string: &str) -> Value {
        let mut cstr = string.as_bytes().to_vec();
        cstr.push(0);
        let layout = Layout::from_bytes(cstr.len() as u64, 1);
        let res = self.alloc(layout).await;
        self.store(&res, &Value::from_bytes(&cstr, layout.bits())).await;
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

    pub async fn memcpy(&self, dst: &Value, src: &Value, len: u64) {
        if len > 0 {
            let value = self.load(src, Layout::from_bytes(len, 1)).await;
            self.store(dst, &value).await;
        }
    }
    pub async fn realloc(&self, old: &Value, old_layout: Layout, new_layout: Layout) -> Value {
        let new = self.alloc(new_layout).await;
        self.memcpy(&new, old, new_layout.bytes().min(old_layout.bytes())).await;
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
                self.process.value_from_address(self.process.thread(self.data.threadid()).unwrap().thread_local(*key)),
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
        success: MemoryOrdering,
        failure: MemoryOrdering,
    ) -> Thunk {
        use MemoryOrdering::*;
        let union = match (failure, success) {
            (SequentiallyConsistent, _) | (_, SequentiallyConsistent) =>
                SequentiallyConsistent,
            (AcquireRelease, _) | (_, AcquireRelease) | (Acquire, Release) | (Release, Acquire) =>
                AcquireRelease,
            (Acquire, _) | (_, Acquire) =>
                Acquire,
            (Release, _) | (_, Release) =>
                Release,
            (Monotonic, _) | (_, Monotonic) =>
                Monotonic,
            (Unordered, _) | (_, Unordered) =>
                Unordered,
            (NotAtomic, NotAtomic) => NotAtomic,
        };
        let deps = smallvec![address.clone(), expected, replacement];
        let types = self.process.types();
        let output = types.struc(vec![class.clone(), types.bool()], false);
        let layout = class.layout();
        self.data().thunk_impl(
            self.backtrace().clone(),
            Some((address.clone(), layout)),
            Some(union),
            deps, {
                move |comp, args| {
                    let current = comp.process.load_impl(comp.threadid, args[0], layout, Some(failure)).clone();
                    let changed = &current == args[1];
                    if changed {
                        comp.process.store_impl(comp.threadid, args[0], args[2], Some(success));
                    }
                    Value::aggregate(&output, vec![current, Value::from(changed)].into_iter())
                }
            }).await
    }

    pub async fn atomicrmw(
        &self,
        class: Class,
        address: Thunk,
        value: Thunk,
        ordering: MemoryOrdering,
        oper: impl FnOnce(&Value, &Value) -> Value + 'static,
    ) -> Thunk {
        let deps = smallvec![address.clone(), value];
        let layout = class.layout();
        self.data().thunk_impl(
            self.backtrace().clone(),
            Some((address.clone(), layout)),
            Some(ordering),
            deps, {
                move |comp, args| {
                    let current = comp.process.load_impl(comp.threadid, args[0], layout, Some(ordering)).clone();
                    let new = oper(&current, &args[1]);
                    comp.process.store_impl(comp.threadid, args[0], &new, Some(ordering));
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
}


impl Debug for FlowCtx {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for frame in self.backtrace.iter() {
            write!(f, "{:?}\n", self.process.reverse_lookup(&self.process.value_from_address(frame.ip())))?;
        }
        Ok(())
    }
}