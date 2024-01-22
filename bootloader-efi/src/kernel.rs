use log::info;
use x86_64::{structures::paging::{PageTable, FrameAllocator, Size4KiB, Page, PageTableIndex, OffsetPageTable, PhysFrame}, VirtAddr, PhysAddr};
use xmas_elf::{ElfFile, program::{self, Type}, header};

use crate::{mem::tracked_mapper::TrackedMapper, panic::PrintPanic};


pub fn load_kernel_to_virt_mem(
    kernel: &[u8],
    pml4_table: &mut TrackedMapper<OffsetPageTable>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>
) -> Result<(), &'static str> {
    let kernel_elf = ElfFile::new(kernel)?;
    let kernel_start = PhysAddr::new(&kernel[0] as *const _ as u64);

    for program_header in kernel_elf.program_iter() {
        program::sanity_check(program_header, &kernel_elf)?;
    }

    // get kernel virtual address offset

    let kernel_type = kernel_elf.header.pt2.type_().as_type();
    let kernel_start_virt_addr = match kernel_type {
        header::Type::Executable => VirtAddr::new(0),
        header::Type::SharedObject => {
            let mut min_virt_addr = u64::MAX;
            let mut max_virt_addr = u64::MIN;
            let mut max_align = u64::MIN;

            kernel_elf
                .program_iter()
                .filter(|h| matches!(h.get_type(), Ok(Type::Load)))
                .for_each(|ph| {
                    let ph_right = ph.virtual_addr() + ph.mem_size();

                    if ph_right > max_virt_addr { max_virt_addr = ph_right }
                    if ph.virtual_addr() < min_virt_addr { min_virt_addr = ph.virtual_addr() }
                    if ph.align() > max_align { max_align = ph.align() }
                });
            
            // currently is pml4[511]
            let available_p4pti = pml4_table
                .find_free_space_and_mark((max_virt_addr - min_virt_addr) as usize, true)
                .or_panic("cannot get available pml4 entry, maybe it run out");

            // aligned
            let start_virt_addr = Page::from_page_table_indices_1gib(
                available_p4pti.0, 
                PageTableIndex::new(0)
            ).start_address();

            start_virt_addr - min_virt_addr
        }
        _ => { panic!("kernel has type {:?} which cannot be processed.", kernel_type) }
    };

    info!("kernel_start_virt_addr: {:?}", kernel_start_virt_addr);

    // pml4 标记 program header 定义段已使用
    kernel_elf.program_iter().for_each(|ph| {
        if ph.mem_size() > 0 {
            let ph_start_va = kernel_start_virt_addr + ph.virtual_addr();
            let ph_end_va = ph_start_va + ph.mem_size();
            pml4_table.mark_range_as_used(ph_start_va..ph_end_va);
        }
    });
    
    header::sanity_check(&kernel_elf);

    

    Ok(())
}

//load kernel segments to virtual memory
fn load_kernel_segments(
    kernel_elf: &ElfFile,
    kernel_phys_addr_start: PhysAddr,
    kernel_virt_addr_start: VirtAddr,
) -> Result<(), &'static str> {
    for ph in kernel_elf.program_iter() {
        let p_type = ph.get_type()?;
 
        if p_type == Type::Load {
             // 这个段在物理内存的位置，在 boot 阶段通过 load_file_sfs 读到物理内存的。
             // 我们现在要把它映射到内核
             let seg_start_phys_addr = kernel_phys_addr_start + ph.offset();
             let seg_start_phys_frame = PhysFrame::containing_address(seg_start_phys_addr);
        }
    }

    Ok(())
}