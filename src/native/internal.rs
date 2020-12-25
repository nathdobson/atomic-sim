use std::rc::Rc;

use crate::flow::FlowCtx;
use crate::function::Func;
use crate::native_fn;

async fn set_tracing(flow: &FlowCtx, (tracing, ): (bool, )) {
    flow.data().set_tracing(tracing)
}

pub fn builtins() -> Vec<Rc<dyn Func>> {
    vec![
        native_fn!(set_tracing),
    ]
}