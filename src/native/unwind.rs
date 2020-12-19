use crate::function::{Func, Panic};
use std::rc::Rc;
use crate::util::future::FutureExt;
use crate::value::Value;
use crate::flow::FlowCtx;
use crate::native::Addr;
use crate::data::Thunk;
use crate::native_fn;

async fn _Unwind_RaiseException(flow: &FlowCtx, (object, ): (Addr, )) -> Result<Thunk, Panic> {
    Err(Panic)
}

async fn _Unwind_Backtrace(flow: &FlowCtx, (trace, trace_argument): (Value, Value)) -> Result<Thunk, Panic> {
    for bt in flow.backtrace().iter() {
        let bctx = flow.process().value_from_address(bt.ip() + 1);
        let ret = flow.invoke(&trace, &[&bctx, &trace_argument]).await?;
    }
    Ok(flow.constant(Value::from(0u32)).await)
}

async fn _Unwind_GetTextRelBase(flow: &FlowCtx, (_, ): (Addr, )) -> Addr { Addr(0) }

async fn _Unwind_GetDataRelBase(flow: &FlowCtx, (_, ): (Addr, )) -> Addr { Addr(0) }

async fn _Unwind_GetLanguageSpecificData(flow: &FlowCtx, (_, _): (Addr, Addr)) -> Addr { Addr(0) }

async fn _Unwind_GetIPInfo(flow: &FlowCtx, (_, _): (Addr, Addr)) -> Addr { Addr(0) }

async fn _Unwind_GetRegionStart(flow: &FlowCtx, (_, _): (Addr, Addr)) -> Addr { Addr(0) }

async fn _Unwind_SetGR(flow: &FlowCtx, (_, _, _): (Addr, u32, u32)) {}

async fn _Unwind_SetIP(flow: &FlowCtx, (_, _): (Addr, Addr)) {}

async fn _Unwind_GetIP(flow: &FlowCtx, (bctx, ): (Addr, )) -> Addr {
    bctx
}

async fn __rdos_backtrace_create_state(
    flow: &FlowCtx,
    (filename, threaded, error, data): (Value, Value, Value, Value)) -> Addr {
    Addr(0xCAFEBABEu64)
}

async fn __rdos_backtrace_syminfo(
    flow: &FlowCtx,
    (state, addr, cb, error, data): (Value, Value, Value, Value, Value)) -> u32 {
    let zero = flow.process().null();
    let one = flow.process().value_from_address(1);
    let name = flow.string(&format!("{:?}", flow.process().reverse_lookup(&addr))).await;
    flow.invoke(&cb, &[&data, &addr, &name, &addr, &one]).await.unwrap();
    0
}

async fn __rdos_backtrace_pcinfo(
    flow: &FlowCtx,
    (state, pc, cb, error, data): (Value, Value, Value, Value, Value)) -> u32 {
    let null = flow.process().null();
    let symbol = flow.process().reverse_lookup(&pc);
    let fun = flow.process().reverse_lookup_fun(&pc).unwrap();
    let symbol_name = flow.string(&format!("{:?}", symbol)).await;
    let debugloc = fun.debugloc();
    let (filename, line) = if let Some(debugloc) = debugloc {
        (debugloc.filename.as_str(), debugloc.line)
    } else {
        ("", 0)
    };
    let filename = flow.string(filename).await;
    flow.invoke(&cb, &[&data, &pc, &filename, &Value::from(line), &symbol_name]).await.unwrap();
    0
}

pub fn builtins() -> Vec<Rc<dyn Func>> {
    let mut result: Vec<Rc<dyn Func>> = vec![];
    result.append(&mut vec![
        native_fn!(_Unwind_RaiseException),
        native_fn!(_Unwind_Backtrace),
        native_fn!(_Unwind_GetTextRelBase),
        native_fn!(_Unwind_GetDataRelBase),
        native_fn!(_Unwind_GetLanguageSpecificData),
        native_fn!(_Unwind_GetIPInfo),
        native_fn!(_Unwind_GetRegionStart),
        native_fn!(_Unwind_SetGR),
        native_fn!(_Unwind_SetIP),
        native_fn!(_Unwind_GetIP),
        native_fn!(__rdos_backtrace_create_state),
        native_fn!(__rdos_backtrace_syminfo),
        native_fn!(__rdos_backtrace_pcinfo),
    ]);
    result
}