use core::alloc::GlobalAlloc;

use buddy_alloc::{NonThreadsafeAlloc, FastAllocParam, BuddyAllocParam};
use lazy_static::lazy_static;
use spin::mutex::Mutex;
use crate::sync::upsafe_cell::UPSafeCell;

const RT_HEAP_SIZE: usize = 0x10_8000;
static mut RT_HEAP_SPACE: [u8; RT_HEAP_SIZE] = [0; RT_HEAP_SIZE];

enum CurrentAllocator {
    Boot,
    Runtime
}

lazy_static! {
    static ref BOOT_ALLOC: uefi::allocator::Allocator = uefi::allocator::Allocator;
    static ref RUNTIME_HEAP_ALLOC: UPSafeCell<LockedRTAllocator> = unsafe { 
        let fast_param = FastAllocParam::new(RT_HEAP_SPACE.as_ptr(), 0x10_0000);
        let buddy_param = BuddyAllocParam::new(RT_HEAP_SPACE[0x10_0000..].as_ptr(), 0x8000, 32);
        UPSafeCell::new(LockedRTAllocator::new(NonThreadsafeAlloc::new(fast_param, buddy_param)))
     };
    static ref CURRENT_ALLOCATOR: UPSafeCell<Mutex<CurrentAllocator>> = unsafe { UPSafeCell::new(Mutex::new(CurrentAllocator::Boot)) };
}

struct LockedRTAllocator(Mutex<NonThreadsafeAlloc>);

impl LockedRTAllocator {
    fn new(alloc: NonThreadsafeAlloc) -> Self {
        Self(Mutex::new(alloc))
    }
}

unsafe impl Sync for LockedRTAllocator {}
unsafe impl Send for LockedRTAllocator {}

unsafe impl GlobalAlloc for LockedRTAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let alloc = self.0.lock();
        alloc.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        let alloc = self.0.lock();
        alloc.dealloc(ptr, layout)
    }
}

struct CombinedAllocator;

unsafe impl GlobalAlloc for CombinedAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let mutex = CURRENT_ALLOCATOR.borrow_mut();
        let alloc = mutex.lock();

        match *alloc {
            CurrentAllocator::Boot => BOOT_ALLOC.alloc(layout),
            CurrentAllocator::Runtime => RUNTIME_HEAP_ALLOC.borrow_mut().alloc(layout)
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        let mutex = CURRENT_ALLOCATOR.borrow_mut();
        let alloc = mutex.lock();
        
        match *alloc {
            CurrentAllocator::Boot => BOOT_ALLOC.dealloc(ptr, layout),
            CurrentAllocator::Runtime => RUNTIME_HEAP_ALLOC.borrow_mut().dealloc(ptr, layout)
        }
    }
}

#[global_allocator]
static ALLOCATOR: CombinedAllocator = CombinedAllocator;


pub fn switch_to_runtime_global_allocator() {
    let mutex = CURRENT_ALLOCATOR.borrow_mut();
    let mut alloc = mutex.lock();

    *alloc = CurrentAllocator::Runtime;
}