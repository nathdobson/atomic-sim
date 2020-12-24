use llvm_ir::instruction::{MemoryOrdering, SynchronizationScope};

#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum Ordering {
    None,
    Relaxed,
    Acquire,
    Release,
    AcqRel,
    SeqCst,
}

impl Ordering {
    pub fn union(self, other: Ordering) -> Self {
        use Ordering::*;
        match (self, other) {
            (SeqCst, _) | (_, SeqCst) => SeqCst,
            (AcqRel, _) | (_, AcqRel) | (Acquire, Release) | (Release, Acquire) => AcqRel,
            (Acquire, _) | (_, Acquire) => Acquire,
            (Release, _) | (_, Release) => Release,
            (Relaxed, _) | (_, Relaxed) => Relaxed,
            (None, None) => None,
        }
    }
}

impl From<MemoryOrdering> for Ordering {
    fn from(x: MemoryOrdering) -> Self {
        match x {
            MemoryOrdering::Unordered => panic!(),
            MemoryOrdering::Monotonic => Ordering::Relaxed,
            MemoryOrdering::Acquire => Ordering::Acquire,
            MemoryOrdering::Release => Ordering::Release,
            MemoryOrdering::AcquireRelease => Ordering::AcqRel,
            MemoryOrdering::SequentiallyConsistent => Ordering::SeqCst,
            MemoryOrdering::NotAtomic => Ordering::None,
        }
    }
}

impl From<&Option<llvm_ir::instruction::Atomicity>> for Ordering {
    fn from(atomicity: &Option<llvm_ir::instruction::Atomicity>) -> Self {
        if let Some(atomicity)=atomicity{
            Self::from(atomicity)
        }else{
            Ordering::None
        }
    }
}

impl From<&llvm_ir::instruction::Atomicity> for Ordering {
    fn from(atomicity: &llvm_ir::instruction::Atomicity) -> Self {
        assert_eq!(atomicity.synch_scope, SynchronizationScope::System);
        Self::from(atomicity.mem_ordering)
    }
}