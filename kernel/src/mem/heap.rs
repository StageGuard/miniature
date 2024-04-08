use core::alloc::GlobalAlloc;

use buddy_alloc::{BuddyAllocParam, FastAllocParam, NonThreadsafeAlloc};
use lazy_static::lazy_static;
use shared::uni_processor::UPSafeCell;
use spin::Mutex;

const RT_HEAP_SIZE: usize = 0x10_8000;
static mut RT_HEAP_SPACE: [u8; RT_HEAP_SIZE] = [0; RT_HEAP_SIZE];

lazy_static! {
    static ref RUNTIME_HEAP_ALLOC: UPSafeCell<LockedGlobalAlloc> = unsafe {
        let fast_param = FastAllocParam::new(RT_HEAP_SPACE.as_ptr(), 0x10_0000);
        let buddy_param = BuddyAllocParam::new(RT_HEAP_SPACE[0x10_0000..].as_ptr(), 0x8000, 32);
        UPSafeCell::new(LockedGlobalAlloc::new(NonThreadsafeAlloc::new(fast_param, buddy_param)))
    };
}

struct LockedGlobalAlloc(Mutex<NonThreadsafeAlloc>);

impl LockedGlobalAlloc {
    fn new(alloc: NonThreadsafeAlloc) -> Self {
        Self(Mutex::new(alloc))
    }
}

unsafe impl Sync for LockedGlobalAlloc {}
unsafe impl Send for LockedGlobalAlloc {}

unsafe impl GlobalAlloc for LockedGlobalAlloc {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let alloc = self.0.lock();
        alloc.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        let alloc = self.0.lock();
        alloc.dealloc(ptr, layout)
    }
}

#[derive(Debug)]
pub struct OutOfMemory;

// delegate static global alloc
pub(super) struct _DelegateAlloc;

unsafe impl GlobalAlloc for _DelegateAlloc {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let allocator = RUNTIME_HEAP_ALLOC.inner_exclusive_mut();
        allocator.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        let allocator = RUNTIME_HEAP_ALLOC.inner_exclusive_mut();
        allocator.dealloc(ptr, layout)
    }
}

#[global_allocator]
pub static HEAP_ALLOC: _DelegateAlloc = _DelegateAlloc;

#[test_case]
fn test_heap_allocator() {
    use alloc::{boxed::Box, rc::Rc, vec::Vec};
    use alloc::vec;
    use crate::qemu_println;
    use core::fmt::Write;

    // allocate a number on the heap
    let heap_value = Box::new(41);
    qemu_println!("heap_value at {:p}", heap_value);

    // create a dynamically sized vector
    let mut vec = Vec::new();
    for i in 0..500 {
        vec.push(i);
    }
    qemu_println!("vec at {:p}", vec.as_slice());

    // create a reference counted vector -> will be freed when count reaches 0
    let reference_counted = Rc::new(vec![1, 2, 3]);
    let cloned_reference = reference_counted.clone();
    qemu_println!("current reference count is {}", Rc::strong_count(&cloned_reference));
    core::mem::drop(reference_counted);
    qemu_println!("reference count is {} now", Rc::strong_count(&cloned_reference));
}