use std::rc::Rc;

use crate::data::ComputeCtx;
use crate::flow::FlowCtx;
use crate::function::Func;
use crate::layout::Layout;
use crate::native::{Addr, native_bomb};
use crate::native_comp;
use crate::native_fn;
use crate::util::future::FutureExt;
use crate::value::Value;

fn __rust_alloc(comp: &ComputeCtx, (Addr(len), Addr(align)): (Addr, Addr)) -> Addr {
    Addr(comp.process.memory.alloc(comp.threadid, Layout::from_bytes(len, align)))
}

async fn __rust_realloc(flow: &FlowCtx, (Addr(ptr), Addr(old_size), Addr(align), Addr(new_size)): (Addr, Addr, Addr, Addr)) -> Addr {
    Addr(flow.realloc(ptr,
                 Layout::from_bytes(old_size, align),
                 Layout::from_bytes(new_size, align)).await)
}

async fn __rust_dealloc(flow: &FlowCtx, (Addr(ptr), Addr(size), Addr(align)): (Addr, Addr, Addr)) {
    flow.free(ptr, Layout::from_bytes(size, align)).await;
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