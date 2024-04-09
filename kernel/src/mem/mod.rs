use core::ptr;
use lazy_static::lazy_static;
use spin::Mutex;
use x86_64::structures::paging::PageTable;
use shared::uni_processor::UPSafeCell;

pub mod heap;
pub mod frame_allocator;
pub mod aligned_box;
mod unique;
pub mod user_buffer;
pub mod user_addr_space;

pub const PAGE_SIZE: usize = 4096;

lazy_static! {
    static ref KERNEL_PML4_PAGE_TABLE: UPSafeCell<Mutex<Option<&'static PageTable>>> = unsafe {
        UPSafeCell::new(Mutex::new(None))
    };
}

pub fn set_kernel_pml4_page_table(addr: u64) {
    let refmut = KERNEL_PML4_PAGE_TABLE.inner_exclusive_mut();
    let mut locked = refmut.lock();
    *locked = Some(unsafe { &*(addr as *const PageTable) })
}