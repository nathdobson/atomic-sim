use crate::function::{Func};
use std::rc::Rc;
use crate::util::future::FutureExt;
use crate::value::Value;
use crate::layout::Layout;
use crate::native::{native_bomb, Addr};
use crate::data::ComputeCtx;
use crate::flow::FlowCtx;
use crate::native_comp;
use crate::native_fn;

fn _tlv_atexit(comp: &ComputeCtx, (func, objAddr): (Addr, Addr)) {}

async fn _NSGetExecutablePath(flow: &FlowCtx, (Addr(buf), Addr(len_ptr)): (Addr, Addr)) -> u32 {
    let len = flow.load(len_ptr, Layout::of_int(32)).await;
    let filename = b"unknown.rs\0";
    if len.as_u64() < filename.len() as u64 {
        return 0;
    }
    flow.store(buf, &Value::from_bytes_exact(filename)).await;
    flow.store(len_ptr, &Value::from(filename.len() as u32)).await;
    0
}

pub fn builtins() -> Vec<Rc<dyn Func>> {
    let mut result: Vec<Rc<dyn Func>> = vec![];
    result.append(&mut vec![
        native_comp!(_tlv_atexit),
        native_fn!(_NSGetExecutablePath),
    ]);
    for bomb in &[
        "_NSGetArgc",
        "_NSGetArgv",
        "_NSGetEnviron",
        "__error",
    ] {
        result.push(native_bomb(*bomb))
    }
    result
}