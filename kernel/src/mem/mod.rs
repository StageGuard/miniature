use core::ptr;
use lazy_static::lazy_static;
use spin::Mutex;
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::PageTable;
use shared::print_panic::PrintPanic;
use shared::uni_processor::UPSafeCell;

pub mod heap;
pub mod frame_allocator;
pub mod aligned_box;
mod unique;
pub mod user_buffer;
pub mod user_addr_space;
pub mod load_elf;

pub const PAGE_SIZE: usize = 4096;

pub static KERNEL_PHYS_ADDRSP_P4_INDEX: usize = 256;

lazy_static! {
    static ref KERNEL_PML4_PAGE_TABLE: UPSafeCell<Mutex<Option<&'static PageTable>>> = unsafe {
        UPSafeCell::new(Mutex::new(None))
    };
}

pub fn set_kernel_pml4_page_table(addr: u64) {
    let refmut = KERNEL_PML4_PAGE_TABLE.inner_exclusive_mut();
    let mut locked = refmut.lock();

    let mut pt = unsafe { &mut *(addr as *mut PageTable) };
    pt[KERNEL_PHYS_ADDRSP_P4_INDEX] = pt[0].clone(); // map phys addr space to higher half

    *locked = Some(pt);
    drop(locked);
    drop(refmut);

    let refmut = KERNEL_PML4_PAGE_TABLE.inner_exclusive_mut();
    let locked = refmut.lock().or_panic("failed to get KERNEL_PML4_PAGE_TABLE, it is none");
    assert_eq!(locked as *const _ as u64, Cr3::read().0.start_address().as_u64())
}

pub fn get_kernel_pml4_page_table_addr() -> u64 {
    let refmut = KERNEL_PML4_PAGE_TABLE.inner_exclusive_mut();
    let locked = refmut.lock().or_panic("failed to get KERNEL_PML4_PAGE_TABLE, it is none");
    locked as *const _ as u64
}