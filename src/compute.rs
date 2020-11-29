use crate::process::Process;
use crate::thread::ThreadId;
use crate::ctx::Ctx;
use std::rc::Rc;

pub struct ComputeCtx<'ctx, 'comp> {
    pub process: &'comp mut Process<'ctx>,
    pub threadid: ThreadId,
}

impl<'ctx, 'comp> ComputeCtx<'ctx, 'comp> {
    pub fn ctx(&self) -> &Rc<Ctx<'ctx>> {
        &self.process.ctx
    }
}