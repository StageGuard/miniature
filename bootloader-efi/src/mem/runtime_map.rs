use x86_64::{registers::segmentation::{Segment, CS, DS, ES, SS}, structures::{gdt::{Descriptor, GlobalDescriptorTable}, paging::{FrameAllocator, Mapper, OffsetPageTable, Page, PageTableIndex, PhysFrame, Size4KiB}}, PhysAddr, VirtAddr};
use x86_64::structures::paging::page_table::PageTableFlags as PTFlags;

use crate::{mem::tracked_mapper::TrackedMapper, panic::PrintPanic};

// map kernel stack
pub fn alloc_and_map_kernel_stack(
    stack_size: usize,
    kernel_pml4_table: &mut TrackedMapper<OffsetPageTable>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>
) -> VirtAddr {
    // additional size for guarding
    let total_size = stack_size + 4096;

    let available_p4pti = kernel_pml4_table.find_free_space_and_mark(total_size, true)
        .or_panic("cannot get available pml4 entry for kernel stack, maybe it run out");
    let kernel_stack_start_page_1gb = Page::from_page_table_indices_1gib(available_p4pti.0,  PageTableIndex::new(0));

    let kernel_stack_start_page = Page::<Size4KiB>::containing_address(kernel_stack_start_page_1gb.start_address());
    let kernel_stack_end_page = Page::<Size4KiB>::containing_address(kernel_stack_start_page.start_address() + total_size - 1u64);

    for page in Page::range_inclusive(kernel_stack_start_page, kernel_stack_end_page) {
        let frame = frame_allocator
            .allocate_frame()
            .or_panic("cannot allocate new physics frame for kernel stack");

        unsafe {
            kernel_pml4_table
                .map_to(page, frame, PTFlags::PRESENT | PTFlags::WRITABLE, frame_allocator)
                .or_panic("cannot map new allocated physics frame to kernel stack page.")
                .flush();
        }
    }

    kernel_stack_start_page.start_address() + 4096u64
}

// create and map gdt
pub fn alloc_and_map_gdt(
    kernel_pml4_table: &mut TrackedMapper<OffsetPageTable>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>
) -> VirtAddr {
    let gdt_phys_frame = frame_allocator
        .allocate_frame()
        .or_panic("cannot allocate new physics frame for global descriptor table.");

    // uefi boootloader runtime 阶段的虚拟地址和物理地址是无偏移映射的
    let gdt = unsafe {
        let ptr: *mut GlobalDescriptorTable = gdt_phys_frame.start_address().as_u64() as *mut GlobalDescriptorTable;
        *ptr = GlobalDescriptorTable::new();
        &mut *ptr
    };

    let code_selector = gdt.add_entry(Descriptor::kernel_code_segment());
    let data_selector = gdt.add_entry(Descriptor::kernel_data_segment());
    gdt.load();

    unsafe {
        CS::set_reg(code_selector);
        DS::set_reg(data_selector);
        ES::set_reg(data_selector);
        SS::set_reg(data_selector);
    }

    let gdt_identical_page = Page::containing_address(VirtAddr::new(gdt_phys_frame.start_address().as_u64()));

    unsafe {
        kernel_pml4_table
            .map_to(gdt_identical_page, gdt_phys_frame, PTFlags::PRESENT, frame_allocator)
            .or_panic("cannot identity map new allocated global descriptor table.")
            .flush();

        gdt_identical_page.start_address()
    }
}

pub fn map_framebuffer(
    framebuffer: &[u8],
    kernel_pml4_table: &mut TrackedMapper<OffsetPageTable>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>
) -> VirtAddr {
    let available_p4pti = kernel_pml4_table.find_free_space_and_mark(framebuffer.len(), true)
        .or_panic("cannot get available pml4 entry for framebuffer, maybe it run out");
    let framebuffer_start_page_1gb = Page::from_page_table_indices_1gib(available_p4pti.0,  PageTableIndex::new(0));

    let framebuffer_start_page = Page::<Size4KiB>::containing_address(framebuffer_start_page_1gb.start_address());
    let framebuffer_phys_addr = PhysAddr::new(&framebuffer[0] as *const _ as u64);

    let framebuffer_start_phys_frame = PhysFrame::<Size4KiB>::containing_address(framebuffer_phys_addr);
    let framebuffer_end_phys_frame = PhysFrame::<Size4KiB>::containing_address(framebuffer_phys_addr + framebuffer.len() - 1u64);

    for (idx, frame) in PhysFrame::range_inclusive(framebuffer_start_phys_frame, framebuffer_end_phys_frame).enumerate() {
        unsafe {
            kernel_pml4_table
                .map_to(
                    framebuffer_start_page + idx as u64, 
                    frame, 
                    PTFlags::PRESENT | PTFlags::WRITABLE, 
                    frame_allocator
                )
                .or_panic("cannot map new allocated physics frame to kernel stack page.")
                .flush();
        }
    }

    framebuffer_start_page.start_address() + (framebuffer_phys_addr - framebuffer_phys_addr.align_down(4096u64))
}

