use std::rc::Rc;
use std::fmt::{Debug, Formatter};
use std::fmt;
use crate::value::Value;
use std::marker::PhantomData;
use crate::process::Process;
use llvm_ir::DebugLoc;

enum BacktraceInner {
    Nil,
    Cons {
        frame: BacktraceFrame,
        next: Backtrace,
    },
}

#[derive(Clone)]
pub struct Backtrace(Rc<BacktraceInner>);

#[derive(Debug)]
struct BacktraceFrameInner {
    pub ip: u64,
    pub loc: DebugLoc,
}

#[derive(Clone)]
pub struct BacktraceFrame(Rc<BacktraceFrameInner>);

pub struct Iter<'bt> {
    backtrace: &'bt Backtrace,
}

impl Backtrace {
    pub fn empty() -> Self {
        Backtrace(Rc::new(BacktraceInner::Nil))
    }
    pub fn prepend(&self, frame: BacktraceFrame) -> Self {
        Backtrace(Rc::new(BacktraceInner::Cons {
            frame,
            next: self.clone(),
        }))
    }
    pub fn iter(&self) -> Iter<'_> {
        Iter { backtrace: self }
    }
}

impl BacktraceFrame {
    pub fn new(ip: u64, loc: DebugLoc) -> Self {
        BacktraceFrame(Rc::new(BacktraceFrameInner { ip, loc }))
    }
    pub fn ip(&self) -> u64 {
        self.0.ip
    }
    pub fn loc(&self) -> &DebugLoc {
        &self.0.loc
    }
}
//
// impl Default for BacktraceFrame {
//     fn default() -> Self {
//         BacktraceFrame(Rc::new(BacktraceFrameInner {
//             ip: 0xFFFF_FFFF,
//             loc: DebugLoc {
//                 line: 0,
//                 col: None,
//                 filename: "<native>".to_string(),
//                 directory: None,
//             },
//         }))
//     }
// }

impl<'ctx, 'bt> Iterator for Iter<'bt> {
    type Item = &'bt BacktraceFrame;

    fn next(&mut self) -> Option<Self::Item> {
        match &*self.backtrace.0 {
            BacktraceInner::Nil => None,
            BacktraceInner::Cons { frame, next } => {
                self.backtrace = next;
                Some(frame)
            }
        }
    }
}

impl Debug for Backtrace {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl Debug for BacktraceFrame {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("BacktraceFrame")
            .field("ip", &self.0.ip)
            .field("loc", &self.0.loc)
            .finish()
    }
}