use std::cmp::Ordering;
use crate::layout::Layout;
use crate::value::Value;
use crate::function::{Func, Panic};
use crate::data::{Thunk, ComputeCtx, ThunkDeps};
use std::rc::Rc;
use std::marker::PhantomData;
use crate::backtrace::BacktraceFrame;
use std::convert::{TryInto, TryFrom};
use crate::flow::FlowCtx;
use crate::layout::Packing;
use crate::util::future::{FutureExt, LocalBoxFuture};
use crate::thread::ThreadId;
use llvm_ir::instruction::MemoryOrdering;
use std::future::Future;
use llvm_ir::DebugLoc;
use std::fmt::{Formatter, Debug};
use std::fmt;
use crate::process::Process;

mod llvm;
mod pthread;
mod libc;
mod apple;
mod rust;
mod unwind;

#[derive(Debug)]
struct Addr(u64);

trait NativeInput {
    fn needs_value() -> bool { true }
    fn from_thunk(t: &Thunk) -> Self;
}

trait NativeInputs {
    fn needs_value(index: usize) -> Option<bool>;
    fn from_thunks(ts: &[Thunk]) -> Self;
}

impl NativeInput for Thunk {
    fn needs_value() -> bool { false }
    fn from_thunk(t: &Thunk) -> Self { t.clone() }
}

impl NativeInput for bool {
    fn from_thunk(t: &Thunk) -> Self { t.try_get().unwrap().unwrap_bool() }
}

impl NativeInput for u8 {
    fn from_thunk(t: &Thunk) -> Self { t.try_get().unwrap().unwrap_u8() }
}

impl NativeInput for i8 {
    fn from_thunk(t: &Thunk) -> Self { t.try_get().unwrap().unwrap_i8() }
}

impl NativeInput for u16 {
    fn from_thunk(t: &Thunk) -> Self { t.try_get().unwrap().unwrap_u16() }
}

impl NativeInput for i16 {
    fn from_thunk(t: &Thunk) -> Self { t.try_get().unwrap().unwrap_i16() }
}

impl NativeInput for u32 {
    fn from_thunk(t: &Thunk) -> Self { t.try_get().unwrap().unwrap_u32() }
}

impl NativeInput for i32 {
    fn from_thunk(t: &Thunk) -> Self { t.try_get().unwrap().unwrap_i32() }
}

impl NativeInput for u64 {
    fn from_thunk(t: &Thunk) -> Self { t.try_get().unwrap().unwrap_u64() }
}

impl NativeInput for i64 {
    fn from_thunk(t: &Thunk) -> Self { t.try_get().unwrap().unwrap_i64() }
}

impl NativeInput for u128 {
    fn from_thunk(t: &Thunk) -> Self { t.try_get().unwrap().unwrap_u128() }
}

impl NativeInput for i128 {
    fn from_thunk(t: &Thunk) -> Self { t.try_get().unwrap().unwrap_i128() }
}

impl NativeInput for Addr {
    fn from_thunk(t: &Thunk) -> Self { Addr(t.try_get().unwrap().as_u64()) }
}

impl NativeInput for Value {
    fn from_thunk(t: &Thunk) -> Self { t.try_get().unwrap().clone() }
}

impl NativeInputs for () {
    fn needs_value(index: usize) -> Option<bool> { None }
    fn from_thunks(_: &[Thunk]) -> Self { () }
}

impl<T> NativeInputs for (T, ) where T: NativeInput {
    fn needs_value(index: usize) -> Option<bool> { [T::needs_value()].get(index).cloned() }
    fn from_thunks(ts: &[Thunk]) -> Self { (T::from_thunk(&ts[0]), ) }
}

impl<T1, T2> NativeInputs for (T1, T2) where T1: NativeInput, T2: NativeInput {
    fn needs_value(index: usize) -> Option<bool> { [T1::needs_value(), T2::needs_value()].get(index).cloned() }
    fn from_thunks(ts: &[Thunk]) -> Self { (T1::from_thunk(&ts[0]), T2::from_thunk(&ts[1])) }
}

impl<T1, T2, T3> NativeInputs for (T1, T2, T3) where
    T1: NativeInput,
    T2: NativeInput,
    T3: NativeInput,
{
    fn needs_value(index: usize) -> Option<bool> {
        [
            T1::needs_value(),
            T2::needs_value(),
            T3::needs_value(),
        ].get(index).cloned()
    }
    fn from_thunks(ts: &[Thunk]) -> Self {
        (
            T1::from_thunk(&ts[0]),
            T2::from_thunk(&ts[1]),
            T3::from_thunk(&ts[2]),
        )
    }
}

impl<T1, T2, T3, T4> NativeInputs for (T1, T2, T3, T4) where
    T1: NativeInput,
    T2: NativeInput,
    T3: NativeInput,
    T4: NativeInput,
{
    fn needs_value(index: usize) -> Option<bool> {
        [
            T1::needs_value(),
            T2::needs_value(),
            T3::needs_value(),
            T4::needs_value(),
        ].get(index).cloned()
    }
    fn from_thunks(ts: &[Thunk]) -> Self {
        (
            T1::from_thunk(&ts[0]),
            T2::from_thunk(&ts[1]),
            T3::from_thunk(&ts[2]),
            T4::from_thunk(&ts[3]),
        )
    }
}

impl<T1, T2, T3, T4, T5> NativeInputs for (T1, T2, T3, T4, T5) where
    T1: NativeInput,
    T2: NativeInput,
    T3: NativeInput,
    T4: NativeInput,
    T5: NativeInput,
{
    fn needs_value(index: usize) -> Option<bool> {
        [
            T1::needs_value(),
            T2::needs_value(),
            T3::needs_value(),
            T4::needs_value(),
            T5::needs_value(),
        ].get(index).cloned()
    }
    fn from_thunks(ts: &[Thunk]) -> Self {
        (
            T1::from_thunk(&ts[0]),
            T2::from_thunk(&ts[1]),
            T3::from_thunk(&ts[2]),
            T4::from_thunk(&ts[3]),
            T5::from_thunk(&ts[4]),
        )
    }
}

impl<T1, T2, T3, T4, T5, T6> NativeInputs for (T1, T2, T3, T4, T5, T6) where
    T1: NativeInput,
    T2: NativeInput,
    T3: NativeInput,
    T4: NativeInput,
    T5: NativeInput,
    T6: NativeInput,
{
    fn needs_value(index: usize) -> Option<bool> {
        [
            T1::needs_value(),
            T2::needs_value(),
            T3::needs_value(),
            T4::needs_value(),
            T5::needs_value(),
            T6::needs_value(),
        ].get(index).cloned()
    }
    fn from_thunks(ts: &[Thunk]) -> Self {
        (
            T1::from_thunk(&ts[0]),
            T2::from_thunk(&ts[1]),
            T3::from_thunk(&ts[2]),
            T4::from_thunk(&ts[3]),
            T5::from_thunk(&ts[4]),
            T6::from_thunk(&ts[5]),
        )
    }
}

async fn from_thunks_impl<T>(args: &[Thunk]) -> Option<T> where T: NativeInputs {
    for (i, x) in args.iter().enumerate() {
        if T::needs_value(i)? {
            x.clone().await;
        }
    }
    Some(T::from_thunks(args))
}

trait NativeOutput {
    fn to_thunk(self, flow: &FlowCtx) -> Result<Result<Thunk, Value>, Panic>;
}

impl NativeOutput for Result<Thunk, Panic> {
    fn to_thunk(self, flow: &FlowCtx) -> Result<Result<Thunk, Value>, Panic> {
        Ok(Ok(self?))
    }
}

impl NativeOutput for Thunk {
    fn to_thunk(self, flow: &FlowCtx) -> Result<Result<Thunk, Value>, Panic> {
        Ok(Ok(self))
    }
}

impl NativeOutput for Addr {
    fn to_thunk(self, flow: &FlowCtx) -> Result<Result<Thunk, Value>, Panic> {
        Ok(Err(flow.process().value_from_address(self.0)))
    }
}

impl<T> NativeOutput for T where Value: From<T> {
    fn to_thunk(self, flow: &FlowCtx) -> Result<Result<Thunk, Value>, Panic> {
        Ok(Err(Value::from(self)))
    }
}

trait NativeValueOutput {
    fn to_value(self, process: &Process) -> Value;
}

impl NativeValueOutput for Addr {
    fn to_value(self, process: &Process) -> Value {
        process.value_from_address(self.0)
    }
}

impl<T> NativeValueOutput for T where Value: From<T> {
    fn to_value(self, process: &Process) -> Value {
        Value::from(self)
    }
}

#[derive(Debug)]
struct NativeBomb(&'static str);

impl Func for NativeBomb {
    fn name(&self) -> &str {
        self.0
    }

    fn call_imp<'a>(&'a self, flow: &'a FlowCtx, args: &'a [Thunk]) -> LocalBoxFuture<'a, Result<Thunk, Panic>> {
        todo!("{:?}", self.0)
    }

    fn debugloc(&self) -> Option<&DebugLoc> {
        None
    }
}

pub fn native_bomb(str: &'static str) -> Rc<dyn Func> {
    Rc::new(NativeBomb(str))
}

#[macro_export]
macro_rules! native_fn {
    ($f:ident) => {
        native_fn!(std::stringify!($f), $f)
    };
    ($name:expr, $f:ident) => {
        {
            use std::fmt::Debug;
            use crate::util::future::LocalBoxFuture;
            use crate::function::Panic;
            use std::fmt;
            use std::fmt::Formatter;
            use crate::data::Thunk;
            use crate::native::from_thunks_impl;
            use crate::native::NativeOutput;
            use crate::util::future::FutureExt;
            struct Impl;
            impl Func for Impl{
                fn name(&self) -> &str {
                    $name
                }
                fn call_imp<'a>(&'a self, flow: &'a FlowCtx, args: &'a [Thunk]) -> LocalBoxFuture<'a, Result<Thunk, Panic>> {
                    async move {
                        let argvs = from_thunks_impl(args).await.unwrap_or_else(|| panic!("Argument mismatch {:?}", $name));
                        let out = $f(flow, argvs);
                        match out.await.to_thunk(flow)?{
                            Ok(thunk) => Ok(thunk),
                            Err(value) => Ok(flow.constant(value).await),
                        }
                    }.boxed_local()
                }
            }
            impl Debug for Impl{
                fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                    write!(f, "{}", $name)
                }
            }
            Rc::new(Impl) as Rc<dyn 'static + Func>
        }
    };
}

struct NativeComp<F, I, O>
    where F: 'static + Fn(&ComputeCtx, I) -> O,
          I: NativeInputs,
          O: NativeValueOutput {
    name: &'static str,
    f: Rc<F>,
    phantom: PhantomData<(I, O)>,
}

impl<F, I, O> Func for NativeComp<F, I, O>
    where F: 'static + Fn(&ComputeCtx, I) -> O,
          I: NativeInputs,
          O: NativeValueOutput {
    fn name(&self) -> &str {
        self.name
    }
    fn call_imp<'a>(&'a self, flow: &'a FlowCtx, args: &'a [Thunk]) -> LocalBoxFuture<'a, Result<Thunk, Panic>> {
        async move {
            let f = self.f.clone();
            let args: ThunkDeps = args.iter().cloned().collect();
            let process = flow.process().clone();
            Ok(flow.data().thunk(
                flow.backtrace().clone(),
                args.clone(),
                move |comp, _| {
                    let argvs = I::from_thunks(&args);
                    f(comp, argvs).to_value(&process)
                }).await)
        }.boxed_local()
    }
}

impl<F, I, O> Debug for NativeComp<F, I, O>
    where F: 'static + Fn(&ComputeCtx, I) -> O,
          I: NativeInputs,
          O: NativeValueOutput {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", std::stringify!($f))
    }
}

fn native_comp_new<F, I:, O>(name: &'static str, f: F)
                             -> Rc<dyn Func>
    where F: 'static + Fn(&ComputeCtx, I) -> O,
          I: 'static + NativeInputs,
          O: 'static + NativeValueOutput {
    Rc::new(NativeComp {
        name,
        f: Rc::new(f),
        phantom: PhantomData,
    }) as Rc<dyn 'static + Func>
}

#[macro_export]
macro_rules! native_comp {
    ($f:ident)=>{
        native_comp!(std::stringify!($f), $f)
    };
    ($name:expr, $f:ident) => {
        {
            use $crate::native::NativeComp;
            use std::marker::PhantomData;
            use crate::native::native_comp_new;
            native_comp_new($name, $f)
        }
    }
}

pub fn builtins() -> Vec<Rc<dyn Func>> {
    let mut result: Vec<Rc<dyn Func>> = vec![];
    result.extend(pthread::builtins());
    result.extend(llvm::builtins());
    result.extend(libc::builtins());
    result.extend(apple::builtins());
    result.extend(rust::builtins());
    result.extend(unwind::builtins());
    result
}

