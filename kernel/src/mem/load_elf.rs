use alloc::sync::Arc;
use log::{debug, info, warn};
use x86_64::{align_up, structures::paging::{mapper::{MappedFrame, TranslateResult}, page::PageRangeInclusive, FrameAllocator, Mapper, OffsetPageTable, Page, PageTableIndex, PhysFrame, Size4KiB, Translate}, PhysAddr, VirtAddr};
use x86_64::structures::paging::page_table::PageTableFlags as PTFlags;
use xmas_elf::{dynamic, header::{self, Type as EType}, program::{self, SegmentData, Type as ShType}, sections::Rela, ElfFile};
use core::{cmp, iter::Step, mem::size_of, ptr};
use spin::RwLockWriteGuard;

use shared::{arg::TlsTemplate, print_panic::PrintPanic};
use crate::infohart;
use crate::mem::frame_allocator::frame_alloc;
use crate::mem::PAGE_SIZE;
use crate::mem::user_addr_space::{RwLockUserAddrSpace, UserAddrSpace};

/// load elf to userspace, return entry point
pub unsafe fn elf_copy_to_addrsp(
    elf: &[u8],
    addrsp: Arc<RwLockUserAddrSpace>
) -> VirtAddr {
    let elf_file = ElfFile::new(elf).or_panic("failed to parse elf");
    let elf_bytes_phys_addr = PhysAddr::new(&elf[0] as *const _ as u64);
    info!("mapping elf, size: {}", elf.len());

    let mut addrsp_guard = addrsp.acquire_write();

    for program_header in elf_file.program_iter() {
        program::sanity_check(program_header, &elf_file).or_panic("kernel progran sanity check failed");
    }
    header::sanity_check(&elf_file).or_panic("kernel header sanity check failed");

    // get kernel virtual address offset
    let elf_pt2_type = elf_file.header.pt2.type_().as_type();
    // kernel elf 定义的起始虚拟地址 和 需要用到的虚拟地址空间大小
    let (elf_start_virt_addr, elf_virt_addr_space_size) = match elf_pt2_type {
        EType::Executable | EType::SharedObject => {
            let mut min_virt_addr = u64::MAX;
            let mut max_virt_addr = u64::MIN;

            elf_file
                .program_iter()
                .filter(|h| matches!(h.get_type(), Ok(ShType::Load)))
                .for_each(|ph| {
                    let ph_right = ph.virtual_addr() + ph.mem_size();

                    if ph_right > max_virt_addr { max_virt_addr = ph_right }
                    if ph.virtual_addr() < min_virt_addr { min_virt_addr = ph.virtual_addr() }
                });
            (min_virt_addr, (max_virt_addr - min_virt_addr) as usize)
        }
        _ => { panic!("elf has type {:?} which cannot be processed.", elf_pt2_type) }
    };

    let mut tls_template: Option<TlsTemplate> = None;

    // load kernel segments to virtual memory
    // TODO: seg 处理有顺序：LOAD，DYNAMIC，GNU_RELRO
    // TODO: 现在是在一个迭代器都处理，假设迭代器元素的顺序都正确。
    for ph in elf_file.program_iter() {
        if ph.mem_size() <= 0 {
            continue;
        }

        let seg_bytes_start_addr = elf_bytes_phys_addr + ph.offset();

        let seg_start_virt_addr = VirtAddr::new(ph.virtual_addr());
        // 段 bss 在实际虚拟内存结束位置，bss 可能追加在 fs 后面
        let seg_mem_end_virt_addr = seg_start_virt_addr + ph.mem_size();
        // 段 fs 在实际虚拟内存结束位置
        let seg_file_end_virt_addr = seg_start_virt_addr + ph.file_size();

        // elf 段数据在物理地址所在的物理帧，end inclusive
        let seg_bytes_start_phys_frame = PhysFrame::<Size4KiB>::containing_address(seg_bytes_start_addr);
        let seg_bytes_end_phys_frame = PhysFrame::<Size4KiB>::containing_address(seg_bytes_start_addr + ph.file_size() - 1u64);

        // 段实际虚拟内存位置对应的页，end inclusive
        let seg_start_page = Page::<Size4KiB>::containing_address(seg_start_virt_addr);
        let seg_end_page = Page::<Size4KiB>::containing_address(seg_mem_end_virt_addr - 1u64);

        let sh_type = ph.get_type().unwrap_or(ShType::Null);

        match sh_type {
            ShType::Load => { // Loadable segment
            infohart!("loading LOAD segment from phys addr 0x{:x} to virt addr 0x{:x}, file_size = {}, mem_size = {}",
                seg_bytes_start_addr.as_u64(), ph.virtual_addr(), ph.file_size(), ph.mem_size()
            );

                let seg_flags = {
                    let mut f = PTFlags::PRESENT;
                    if !ph.flags().is_execute() { f |= PTFlags::NO_EXECUTE; }
                    if ph.flags().is_write() { f |= PTFlags::WRITABLE; }
                    f
                };

                // setup mapping to the fs part of segment and kernel pml4 table.
                for original_frame in PhysFrame::range_inclusive(seg_bytes_start_phys_frame, seg_bytes_end_phys_frame) {
                    let seg_page = seg_start_page + (original_frame - seg_bytes_start_phys_frame);

                    let new_frame = frame_alloc()
                        .or_panic("failed to allocate new phys frame for bss segment.");

                    ptr::copy(
                        original_frame.start_address().as_u64() as *const u8,
                        new_frame.start_address().as_u64() as *mut u8,
                        PAGE_SIZE
                    );

                    addrsp_guard.raw_map_to(seg_page, new_frame, seg_flags);
                    addrsp_guard.push_tracked_frame(new_frame);
                }

                // 段没有 .bss 部分

                if ph.mem_size() <= ph.file_size() { continue; }

                // 段有 .bss 部分，需要 zero-fill
                let seg_bss_start_virt_addr = seg_file_end_virt_addr;
                let seg_bss_end_virt_addr = seg_mem_end_virt_addr;

                infohart!("clearing .bss segments from virt 0x{:x} to 0x{:x}", seg_bss_start_virt_addr, seg_bss_end_virt_addr);

                // .bss 部分需要跟在 fs 后并且填充 0
                // 在物理内存中，段 fs 结束所在的物理页帧可能包含其他东西，而不仅仅是段 fs
                // 如果有那不能直接写成 0，需要分配一个新的页帧，把东西复制过去。
                // 然后把 fs 结束的虚拟地址映射到这个新的页，这样就不会修改原先的页帧了。

                let file_end_relative_addr = seg_bss_start_virt_addr.as_u64() & 0xfff;
                // 检测一下 fs 结束地址是不是页对齐的
                if file_end_relative_addr != 0 {
                    debug!("start virt addr of .bss segments is not aligned, offset = {}", file_end_relative_addr);
                    // 如果不是对齐的，我们需要特殊处理 bss 段的第一个页
                    // 分配一个新的物理页，把这一页复制过去，然后再 zero-fill 新复制的页的 bss 段
                    let last_page = Page::<Size4KiB>::containing_address(seg_bss_start_virt_addr - 1u64);
                    let new_frame = copy_page_and_remap(last_page, &mut addrsp_guard)
                        .or_panic("failed to remap the page of of fs end LOAD segment.");

                    let new_frame_phys_addr = new_frame.start_address().as_u64() as *mut u8;
                    ptr::write_bytes(
                        new_frame_phys_addr.add(file_end_relative_addr as usize),
                        0u8,
                        4096 - file_end_relative_addr as usize
                    )
                }

                // 其他 bss 段
                // 分配新的物理页帧然后映射
                let seg_bss_start_page = Page::<Size4KiB>::containing_address(
                    VirtAddr::new(align_up(seg_bss_start_virt_addr.as_u64(), PAGE_SIZE as u64))
                );
                let seg_bss_end_page = Page::<Size4KiB>::containing_address(seg_bss_end_virt_addr - 1u64);

                for bss_page in Page::range_inclusive(seg_bss_start_page, seg_bss_end_page) {
                    let frame = frame_alloc()
                        .or_panic("failed to allocate new phys frame for bss segment.");

                    let frame_ptr = frame.start_address().as_u64() as *mut u8;
                    ptr::write_bytes(frame_ptr, 0, 4096);
                    addrsp_guard.raw_map_to(bss_page, frame, seg_flags);
                    addrsp_guard.push_tracked_frame(frame)
                }
            }
            ShType::Dynamic => { // dynamic link data
                let data = ph.get_data(&elf_file)
                    .or_panic("failed to load kernel elf dynamic data");
                let data = if let SegmentData::Dynamic64(data) = data {
                    data
                } else {
                    panic!("not dynamic 64 data")
                };

                // Relocation entries with addends
                let mut rela = None;
                let mut rela_size = None;
                let mut rela_ent = None;

                for rel in data {
                    let tag = rel.get_tag().or_panic("failed get tag of dynamic data");
                    match tag {
                        dynamic::Tag::Rela => {
                            let ptr = rel.get_ptr().or_panic("failed to get rela ptr of dynamic data");
                            let prev = rela.replace(ptr);
                            if prev.is_some() {
                                panic!("Dynamic section contains more than one Rela entry");
                            }
                        }
                        dynamic::Tag::RelaSize => {
                            let val = rel.get_val().or_panic("failed to get rela size of dynamic data");
                            let prev = rela_size.replace(val);
                            if prev.is_some() {
                                panic!("Dynamic section contains more than one RelaSize entry");
                            }
                        }
                        dynamic::Tag::RelaEnt => {
                            let val = rel.get_val().or_panic("failed to get rela entry of dynamic data");
                            let prev = rela_ent.replace(val);
                            if prev.is_some() {
                                panic!("Dynamic section contains more than one RelaEnt entry");
                            }
                        }
                        _ => {}
                    }
                }
                // rela 在 elf 文件的指针位置偏移
                let rela_ptr_offset = match rela {
                    Some(ptr) => ptr,
                    None => {
                        if rela_size.is_some() || rela_ent.is_some() {
                            warn!("Rela entry is missing but RelaSize or RelaEnt have been provided");
                        }
                        continue;
                    }
                };

                let total_size = rela_size.or_panic("RelaSize entry is missing");
                let entry_size = rela_ent.or_panic("RelaEnt entry is missing");

                infohart!("loading DYNAMIC segment: RELA = 0x{:x}, RELASIZE = {}, RELAENT = {}", rela.unwrap(), total_size, entry_size);

                if entry_size as usize != size_of::<Rela<u64>>() {
                    panic!("unsupported dynamic relative entry size: {entry_size}");
                }

                let rela_count = total_size / entry_size;

                for entry_idx in 0..rela_count {
                    // rela entry 在 kernel bytes 的索引偏移
                    let entry_ptr_phys_addr_idx = (rela_ptr_offset - elf_start_virt_addr) + entry_idx * size_of::<Rela<u64>>() as u64;
                    let rela = &*(&elf[entry_ptr_phys_addr_idx as usize] as *const _ as *const Rela<u64>);

                    if rela.get_symbol_table_index() != 0 {
                        panic!("relocation using symbol table is not supported")
                    }

                    // https://intezer.com/blog/malware-analysis/executable-and-linkable-format-101-part-3-relocations/
                    match rela.get_type() {
                        8 => { // R_X86_64_RELATIVE: B + A
                            // TODO: check rela offset is at virtual space of LOAD segments

                            let offset = VirtAddr::new(rela.get_offset());
                            let attend = VirtAddr::new(rela.get_addend());

                            copy_pages_and_write(offset, &attend.as_u64().to_ne_bytes(), &mut addrsp_guard);
                        }
                        _ => {
                            panic!("reallocation with type {} is not supported", rela.get_type())
                        }
                    }
                }
                // // Apply the relocations.
                // for idx in 0..(total_size / entry_size) {
                //     let rela = self.read_relocation(offset, idx);
                //     self.apply_relocation(rela, elf_file)?;
                // }
            }
            ShType::GnuRelro => {
                infohart!("loading GNURELRO segment: start_page: {:?}, end_page: {:?}", seg_start_page, seg_end_page);

                update_page_flag(&mut addrsp_guard, Page::range_inclusive(seg_start_page, seg_end_page), !PTFlags::WRITABLE);
            }
            ShType::Tls => {
                tls_template.replace(TlsTemplate {
                    start_virt_addr: seg_start_virt_addr.as_u64(),
                    mem_size: ph.mem_size() as usize,
                    file_size: ph.file_size() as usize
                });
            }
            _ => {}
        }
    }

    // remove PTEFlags:BIT_9
    for ph in elf_file.program_iter() {
        if ph.mem_size() <= 0 {
            continue;
        }
        if ph.get_type().unwrap_or(ShType::Null) != ShType::Load {
            continue;
        }

        let seg_start_virt_addr = VirtAddr::new(ph.virtual_addr());
        let seg_mem_end_virt_addr = seg_start_virt_addr + ph.mem_size();
        let seg_start_page = Page::<Size4KiB>::containing_address(seg_start_virt_addr);
        let seg_end_page = Page::<Size4KiB>::containing_address(seg_mem_end_virt_addr - 1u64);

        update_page_flag(&mut addrsp_guard, Page::range_inclusive(seg_start_page, seg_end_page), !PTFlags::BIT_9);
    }

    VirtAddr::new(elf_file.header.pt2.entry_point())
}

/// copy underlying phys frame of a page to new allocated frame and remap page to the new one
/// # Safety
/// `page` should be a page mapped by a Load segment.
unsafe fn copy_page_and_remap(
    page: Page,
    addrsp: &mut RwLockWriteGuard<UserAddrSpace>,
) -> Option<PhysFrame> {
    let (curr_frame, flags) = match addrsp.raw_translate(page.start_address()) {
        TranslateResult::Mapped { frame, offset: _, flags, } => {
            if let MappedFrame::Size4KiB(frame) = frame { (frame, flags) } else { return None }
        },
        _ => return None
    };

    if flags.contains(PTFlags::BIT_9) {
        return Some(curr_frame)
    }

    // allocate new frame
    let new_frame = match frame_alloc() {
        Some(f) => f,
        None => return None
    };
    addrsp.push_tracked_frame(new_frame.clone());

    // copy no overlappiong
    let curr_frame_ptr = curr_frame.start_address().as_u64() as *const u8;
    let new_frame_ptr = new_frame.start_address().as_u64() as *mut u8;
    ptr::copy_nonoverlapping(curr_frame_ptr, new_frame_ptr, 4096usize);

    // remap this page
    addrsp.raw_unmap(page);
    addrsp.raw_map_to(page, new_frame, flags | PTFlags::BIT_9);

    Some(new_frame)
}

/// 复制 addr 到 addr + buf_len 所在的 page 到新分配的物理页帧，
/// 写入 buf 到新的物理页帧，再重新映射 page 到新的 buf
/// # SAFETY
/// `addr` should refer to a page mapped by a Load segment.
unsafe fn copy_pages_and_write(
    addr: VirtAddr,
    buf: &[u8],
    addrsp: &mut RwLockWriteGuard<UserAddrSpace>
) {
    // We can't know for sure that contiguous virtual address are contiguous
    // in physical memory, so we iterate of the pages spanning the
    // addresses, translate them to frames and copy the data.

    let end_inclusive_addr = Step::forward_checked(addr, buf.len() - 1)
        .expect("the end address should be in the virtual address space");
    let start_page = Page::<Size4KiB>::containing_address(addr);
    let end_inclusive_page = Page::<Size4KiB>::containing_address(end_inclusive_addr);

    for page in start_page..=end_inclusive_page {
        // Translate the virtual page to the physical frame.
        let phys_addr = unsafe {
            copy_page_and_remap(page, addrsp)
                .or_panic("failed to remap the page of the LOAD segnemt the addr points to")
        };

        // Figure out which address range we want to copy from the frame.

        // This page covers these addresses.
        let page_start = page.start_address();
        let page_end_inclusive = page.start_address() + 4095u64;

        // We want to copy from the following address in this frame.
        let start_copy_address = cmp::max(addr, page_start);
        let end_inclusive_copy_address = cmp::min(end_inclusive_addr, page_end_inclusive);

        // These are the offsets into the frame we want to copy from.
        let start_offset_in_frame = (start_copy_address - page_start) as usize;
        let end_inclusive_offset_in_frame = (end_inclusive_copy_address - page_start) as usize;

        // Calculate how many bytes we want to copy from this frame.
        let copy_len = end_inclusive_offset_in_frame - start_offset_in_frame + 1;

        // Calculate the physical addresses.
        let start_phys_addr = phys_addr.start_address() + start_offset_in_frame;

        // These are the offsets from the start address. These correspond
        // to the destination indices in `buf`.
        let start_offset_in_buf = Step::steps_between(&addr, &start_copy_address).unwrap();

        // Calculate the source slice.
        // Utilize that frames are identity mapped.
        let dest_ptr = start_phys_addr.as_u64() as *mut u8;
        let dest = unsafe {
            // SAFETY: We know that this memory is valid because we got it
            // as a result from a translation. There are not other
            // references to it.
            &mut *core::ptr::slice_from_raw_parts_mut(dest_ptr, copy_len)
        };

        // Calculate the destination pointer.
        let src = &buf[start_offset_in_buf..][..copy_len];

        // Do the actual copy.
        dest.copy_from_slice(src);
    }
}

unsafe fn update_page_flag(
    addrsp: &mut RwLockWriteGuard<UserAddrSpace>,
    range_inclusive: PageRangeInclusive<Size4KiB>,
    flag: PTFlags
) {
    for page in range_inclusive {
        let translated = addrsp.raw_translate(page.start_address());
        let flags = if let TranslateResult::Mapped {
            frame,
            offset,
            flags
        } = translated {
            flags
        } else {
            panic!("page is not mapped while parsing segment GNURELTRO")
        };

        addrsp.raw_update_flags(page, flags & flag);
    }
}