fn page_size() -> usize {
    4096
}

// maximum 8GiB
const MAX_ADDRESS: u64 = 0x2_0000_0000u64;

pub mod boot {
    use core::mem::size_of;
    use core::ptr;
    use core::slice;
    use uefi::table::{Boot, SystemTable};
    use uefi::table::boot::{AllocateType, MemoryType};
    use super::{MAX_ADDRESS, page_size};
    use crate::print_panic::PrintPanic;

    /// can be only used at boot stage.
    pub fn allocate_zeroed_page_aligned(system_table: &SystemTable<Boot>, size: usize) -> *mut u8 {
        let page_size = page_size();
        let pages = (size + page_size - 1) / page_size;

        let bs = system_table.boot_services();
        let ptr = bs
            .allocate_pages(AllocateType::MaxAddress(MAX_ADDRESS), MemoryType::BOOT_SERVICES_DATA, pages)
            .or_panic("cannot allocate pages") as *mut u8;

        // make pages zero-filled
        assert!(!ptr.is_null());
        unsafe { ptr::write_bytes(ptr, 0, pages * page_size) };

        ptr
    }

    /// can be only used at boot stage.
    pub unsafe fn paging_allocate<T : Sized>(system_table: &SystemTable<Boot>) -> Option<&'static mut [T]> {
        let ptr = allocate_zeroed_page_aligned(system_table, page_size());

        if !ptr.is_null() {
            Some(slice::from_raw_parts_mut(ptr as *mut T, page_size() / size_of::<T>()))
        } else {
            None
        }
    }
}

pub mod runtime {
    use x86_64::{structures::paging::{PageTable, FrameAllocator, OffsetPageTable}, VirtAddr, registers::control::{Cr3, Cr3Flags}};

    use crate::{mem::frame_allocator::RTFrameAllocator, print_panic::PrintPanic};


    /// map current level4 page table (boot stage) to runtime stage page table
    pub fn map_boot_stage_page_table<'a>(allocator: &'a mut RTFrameAllocator) -> OffsetPageTable<'a> {
        // UEFI identity-maps all memory, so the offset between physical and virtual addresses is 0
        let phys_offset = VirtAddr::new(0);

        let frame = allocator.allocate_frame().or_panic("failed to allocate new physics frame");

        let current_page_table: &PageTable = unsafe { 
            &*(phys_offset + Cr3::read().0.start_address().as_u64()).as_ptr()
        };
        let new_page_table: &mut PageTable = unsafe {
            let ptr: *mut PageTable = *(phys_offset + frame.start_address().as_u64()).as_mut_ptr();

            *ptr = PageTable::new();
            &mut *ptr
        };

        // clone the first entry of page table
        new_page_table[0] = current_page_table[0].clone();

        unsafe {
            Cr3::write(frame, Cr3Flags::empty());
            OffsetPageTable::new(&mut *new_page_table, phys_offset)
        }
    }

    // create new page table
    pub fn create_page_table<'a>(allocator: &'a mut RTFrameAllocator, phys_offset: VirtAddr) -> OffsetPageTable<'a> {
        let frame = allocator.allocate_frame().or_panic("failed to allocate new physics frame");

        let page_table: *mut PageTable = unsafe {
            let ptr: *mut PageTable = *(phys_offset + frame.start_address().as_u64()).as_mut_ptr();

            *ptr = PageTable::new();
            &mut *ptr
        };

        unsafe { OffsetPageTable::new(&mut *page_table, phys_offset) }
    }
}