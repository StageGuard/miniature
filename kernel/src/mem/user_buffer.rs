use alloc::sync::Arc;
use alloc::vec::Vec;
use libvdso::error::{ENOMEM, ESRCH, KError, KResult};
use crate::context::list::context_storage;

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

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn ptr(&self) -> *const u8 {
        self.base
    }

    pub fn resolve_by_current(self: Arc<Self>) -> KResult<Vec<&'static [u8]>> {
        let contexts = context_storage();
        let context = match contexts.current() {
            Some(lock) => lock,
            None => return Err(KError::new(ESRCH))
        };
        let guard = context.write_arc();
        let addrsp = match guard.addrsp {
            Some(ref r) => r,
            None => return Err(KError::new(ENOMEM))
        };

        let addrsp = addrsp.acquire_read();
        addrsp.resolve(Arc::clone(&self))
    }
}