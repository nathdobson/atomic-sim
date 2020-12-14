use std::alloc::{GlobalAlloc, System};
use smallvec::alloc::alloc::Layout;
use std::ptr::null_mut;

const BLOCK_SIZE: usize = 1 << 20;
const BLOCK_ALIGN: usize = 4096;

#[thread_local]
static mut ARENA: Arena = Arena {
    start: null_mut(),
    len: 0,
};

struct Arena {
    start: *mut u8,
    len: usize,
}

pub struct Allocator;

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        assert!(layout.align() <= BLOCK_ALIGN);
        let padding = ARENA.start.align_offset(layout.align());
        if padding + layout.size() <= ARENA.len {
            let result = ARENA.start.offset(padding as isize);
            ARENA.start = result.offset(layout.size() as isize);
            ARENA.len -= padding + layout.size();
            result
        } else if layout.size() < BLOCK_SIZE {
            let result = System.alloc(Layout::from_size_align(BLOCK_SIZE, BLOCK_ALIGN).unwrap());
            ARENA.start = result.offset(layout.size() as isize);
            ARENA.len = BLOCK_SIZE - layout.size();
            result
        } else {
            System.alloc(layout)
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {}
}