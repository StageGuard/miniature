use core::{mem::size_of, slice};

use log::info;
use uefi::table::boot::MemoryDescriptor;
use x86_64::{align_up, registers::segmentation::{Segment, CS, DS, ES, SS}, structures::{gdt::{Descriptor, GlobalDescriptorTable}, paging::{FrameAllocator, Mapper, OffsetPageTable, Page, PageTableIndex, PhysFrame, Size2MiB, Size4KiB}}, PhysAddr, VirtAddr};
use x86_64::structures::paging::page_table::PageTableFlags as PTFlags;

use crate::{context::KernelArg, mem::tracked_mapper::TrackedMapper, panic::PrintPanic};

use super::{frame_allocator::LinearIncFrameAllocator, MemoryRegion};

// map kernel stack
pub fn alloc_and_map_kernel_stack(
    stack_size: usize,
    kernel_pml4_table: &mut TrackedMapper<OffsetPageTable>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>
) -> VirtAddr {
    // additional size for guarding
    let total_size = stack_size + 4096;

    let available_p4pti = kernel_pml4_table.find_free_space_and_mark(total_size, true)
        .or_panic("failed to get available pml4 entry for kernel stack, maybe it run out");
    let kernel_stack_start_page_1gb = Page::from_page_table_indices_1gib(available_p4pti.0,  PageTableIndex::new(0));

    let kernel_stack_start_page = Page::<Size4KiB>::containing_address(kernel_stack_start_page_1gb.start_address());
    let kernel_stack_end_page = Page::<Size4KiB>::containing_address(kernel_stack_start_page.start_address() + total_size - 1u64);

    for page in Page::range_inclusive(kernel_stack_start_page, kernel_stack_end_page) {
        let frame = frame_allocator
            .allocate_frame()
            .or_panic("failed to allocate new physics frame for kernel stack");

        unsafe {
            kernel_pml4_table
                .map_to(page, frame, PTFlags::PRESENT | PTFlags::WRITABLE, frame_allocator)
                .or_panic("failed to map new allocated physics frame to kernel stack page.")
                .flush();
        }
    }

    kernel_stack_start_page.start_address() + 4096u64
}

// create context switch function map indentically
pub fn map_context_switch_identically(
    context_switch: *const fn(),
    kernel_pml4_table: &mut TrackedMapper<OffsetPageTable>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>
) -> VirtAddr {
    let fn_phys_frame: PhysFrame<Size4KiB> = PhysFrame::containing_address(PhysAddr::new(context_switch as u64));
    
    for frame in PhysFrame::range_inclusive(fn_phys_frame, fn_phys_frame + 1) {
        unsafe {
            kernel_pml4_table
                .identity_map(frame, PTFlags::PRESENT, frame_allocator)
                .or_panic("failed to identity map kernel pml4 table.")
                .flush();
        }
    }
    
    VirtAddr::new(context_switch as u64)
}

// create and map gdt
pub fn alloc_and_map_gdt_identically(
    kernel_pml4_table: &mut TrackedMapper<OffsetPageTable>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>
) -> VirtAddr {
    let gdt_phys_frame = frame_allocator
        .allocate_frame()
        .or_panic("failed to allocate new physics frame for global descriptor table.");

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
            .or_panic("failed to identity map new allocated global descriptor table.")
            .flush();
    }
    gdt_identical_page.start_address()
}

pub fn map_framebuffer(
    framebuffer: &[u8],
    kernel_pml4_table: &mut TrackedMapper<OffsetPageTable>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>
) -> VirtAddr {
    let available_p4pti = kernel_pml4_table.find_free_space_and_mark(framebuffer.len(), true)
        .or_panic("failed to get available pml4 entry for framebuffer, maybe it run out");
    let framebuffer_start_page_1gb = Page::from_page_table_indices_1gib(available_p4pti.0,  PageTableIndex::new(0));

    let framebuffer_start_page = Page::<Size4KiB>::containing_address(framebuffer_start_page_1gb.start_address());
    let framebuffer_phys_addr = PhysAddr::new(&framebuffer[0] as *const _ as u64);

    // bootloader runtime 阶段物理内存和虚拟内存是恒等映射
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
                .or_panic("failed to map new allocated physics frame to kernel stack page.")
                .flush();
        }
    }

    framebuffer_start_page.start_address() + (framebuffer_phys_addr - framebuffer_phys_addr.align_down(4096u64))
}

pub fn map_physics_memory(
    max_phys_addr: PhysAddr,
    kernel_pml4_table: &mut TrackedMapper<OffsetPageTable>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>
) -> VirtAddr {
    // bootloader runtime 阶段物理内存和虚拟内存是恒等映射
    // 用 4kb size 会让下面迭代器迭代过多次
    let start_phys_frame = PhysFrame::<Size2MiB>::containing_address(PhysAddr::new(0));
    let end_phys_frame = PhysFrame::<Size2MiB>::containing_address(max_phys_addr - 1u64);

    info!("physics address space size: {}", max_phys_addr.as_u64());

    let available_p4pti = kernel_pml4_table.find_free_space_and_mark(max_phys_addr.as_u64() as usize, true)
        .or_panic("failed to get available pml4 entry for full physics address space, maybe it run out");
    let phys_start_page = Page::from_page_table_indices_1gib(available_p4pti.0,  PageTableIndex::new(0));

    for frame in PhysFrame::range_inclusive(start_phys_frame, end_phys_frame) {
        let page = Page::<Size2MiB>::containing_address(phys_start_page.start_address() + frame.start_address().as_u64());

        unsafe {
            kernel_pml4_table.map_to(page, frame, PTFlags::PRESENT | PTFlags::WRITABLE, frame_allocator)
                .or_panic("failed to map physics address space to kernel page.")
                .ignore()
        }
    }
    // 这里不用把 frame 关联到 kernel pml4 页表

    phys_start_page.start_address()
}

/// 映射 KernelArg 到内核 PML4 页表
/// 
/// kernel_arg 是可变的，因为在映射的过程中会分配物理帧，而这些新分配的也要写入到 KernelArg.unav_phys_mem_regions 中
/// 
/// TODO: 这样做非常不好，需要优化
pub fn map_kernel_arg<I: ExactSizeIterator<Item = MemoryDescriptor> + Clone>(
    kernel_arg: &mut KernelArg,
    kernel_pml4_table: &mut TrackedMapper<OffsetPageTable>,
    frame_allocator: &mut LinearIncFrameAllocator<I, MemoryDescriptor>
) -> VirtAddr {
    const KERNEL_ARG_LEN: usize = size_of::<KernelArg>();

    let available_p4pti = kernel_pml4_table.find_free_space_and_mark(KERNEL_ARG_LEN, true)
        .or_panic("failed to get available pml4 entry for KernelArg, maybe it run out");
    let kernel_arg_start_page = Page::containing_address(
        Page::from_page_table_indices_1gib(available_p4pti.0, PageTableIndex::new(0)).start_address()
    );

    // 存储 KernelArg 需要的物理页帧
    const NEEDED_PHYS_FRAMES_COUNT: usize = align_up(KERNEL_ARG_LEN as u64, 4096) as usize / 4096;
    let needed_phys_frames: [PhysFrame; NEEDED_PHYS_FRAMES_COUNT] = [
        frame_allocator.allocate_frame().or_panic("failed to allocate frame for kernel arg"); 
        NEEDED_PHYS_FRAMES_COUNT
    ];
    let mut curr_needed_phys_frames_idx = 0;

    // 上面使用 FrameAllocator.allocate_frame 了，需要再记录一下
    kernel_arg.unav_phys_mem_regions[kernel_arg.unav_phys_mem_regions_len] = frame_allocator.allocated_region();
    kernel_arg.unav_phys_mem_regions_len += 1;

    // fast path to write
    if KERNEL_ARG_LEN <= 4096 {
        let frame = &needed_phys_frames[curr_needed_phys_frames_idx];

        unsafe {
            (frame.start_address().as_u64() as *mut KernelArg).write(*kernel_arg);

            kernel_pml4_table
                .map_to(kernel_arg_start_page, *frame, PTFlags::PRESENT, frame_allocator)
                .or_panic("failed to identity map new allocated kernel arg.")
                .flush();
        }

        curr_needed_phys_frames_idx += 1;
    } else {
        let kernel_arg_bytes = unsafe { slice::from_raw_parts(kernel_arg as *const _ as *const u8, KERNEL_ARG_LEN) };

        for slice_index in (0..KERNEL_ARG_LEN).step_by(4096) {
            let slice_end = (slice_index + 4096).min(KERNEL_ARG_LEN);
            let frame = &needed_phys_frames[curr_needed_phys_frames_idx];

            let dest_frame_slice = unsafe {
                slice::from_raw_parts_mut(frame.start_address().as_u64() as *mut u8, slice_end - slice_index)
            };
            dest_frame_slice.copy_from_slice(&kernel_arg_bytes[slice_index..slice_end]);

            unsafe {
                kernel_pml4_table
                    .map_to(kernel_arg_start_page + (slice_index / 4096) as u64, *frame, PTFlags::PRESENT, frame_allocator)
                    .or_panic("failed to identity map new allocated kernel arg.")
                    .flush();
            }

            curr_needed_phys_frames_idx += 1;
        }
    }

    kernel_arg_start_page.start_address()
}