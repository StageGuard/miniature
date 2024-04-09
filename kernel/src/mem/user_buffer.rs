use alloc::sync::Arc;
use spinning_top::RwSpinlock;
use crate::context::Context;

// represents a memory region at userspace
#[repr(C)]
#[derive(Copy, Clone)]
pub struct UserBuffer {
    // the base address at the userspace of `context`
    base: *const u8,
    len: usize,
}

impl UserBuffer {
    pub fn new(base: u64, len: usize) -> Self {
        Self {
            base: base as *const u8,
            len
        }
    }
}