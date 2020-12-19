use std::rc::Rc;
use crate::function::{Func};
use crate::layout::Layout;
use crate::value::Value;
use crate::util::future::FutureExt;
use crate::native::{native_bomb, Addr};
use crate::data::ComputeCtx;
use crate::flow::FlowCtx;
use crate::native_fn;
use crate::native_comp;

fn __rust_alloc(comp: &ComputeCtx, (len, align): (Addr, Addr)) -> Value {
    comp.process.alloc(comp.threadid, Layout::from_bytes(len.0, align.0))
}

async fn __rust_realloc(flow: &FlowCtx, (ptr, old_size, align, new_size): (Value, Addr, Addr, Addr)) -> Value {
    flow.realloc(&ptr,
                 Layout::from_bytes(old_size.0, align.0),
                 Layout::from_bytes(new_size.0, align.0)).await
}

async fn __rust_dealloc(flow: &FlowCtx, (ptr, size, align): (Value, Addr, Addr)) -> Value {
    flow.free(&ptr, Layout::from_bytes(size.0, align.0)).await;
    Value::from(())
}

pub fn builtins() -> Vec<Rc<dyn Func>> {
    let mut result: Vec<Rc<dyn Func>> = vec![];
    result.append(&mut vec![
        native_comp!(__rust_alloc),
        native_fn!(__rust_realloc),
        native_fn!(__rust_dealloc),
    ]);
    for bomb in &[
        "__rust_alloc_zeroed",
    ] {
        result.push(native_bomb(*bomb));
    }
    result
}