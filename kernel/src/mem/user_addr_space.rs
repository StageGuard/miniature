use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::spin_loop;
use core::slice;
use bitflags::Flags;
use spin::{RwLock, RwLockReadGuard, RwLockUpgradableGuard, RwLockWriteGuard};
use spinning_top::RwSpinlock;
use x86_64::{PhysAddr, VirtAddr};
use x86_64::registers::control::{Cr3, Cr3Flags};
use x86_64::structures::paging::{FrameAllocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame, Size1GiB, Size4KiB, Translate};
use x86_64::structures::paging::mapper::TranslateResult;
use libvdso::error::{EFAULT, KError, KResult};
use shared::{BOOTSTRAP_BYTES_P4, FRAMEBUFFER_P4, KERNEL_BYTES_P4, KERNEL_STACK_P4, PHYS_MEM_P4};
use shared::print_panic::PrintPanic;
use crate::arch_spec::copy_to;
use crate::context::Context;
use crate::mem::frame_allocator::{frame_alloc, frame_dealloc};
use crate::mem::{get_kernel_pml4_page_table_addr, PAGE_SIZE};
use crate::mem::user_buffer::UserBuffer;

pub struct RwLockUserAddrSpace {
    context: Arc<RwSpinlock<Context>>,
    inner: Arc<RwLock<UserAddrSpace>>
}

pub struct UserAddrSpace {
    page_table: OffsetPageTable<'static>,
    // 地址空间页表用到的子页表物理页帧
    pte_frames: Vec<PhysFrame>,
    // track buffers which length > PAGE_SIZE
    tracked_large_buffers: Vec<PhysFrame>,
    // track buffers which length < 512 and length > 64
    tracked_medium_buffers: Vec<TrackedPhysFrame>,
    medium_buffer_pointer: usize,
    // store small buffers smaller than 64 bytes
    tracked_small_buffers: Vec<TrackedPhysFrame>,
    // small buffer 物理页帧的当前指针，用来定位分配新内存区域的地址
    small_buffer_pointer: usize,
    // to locate virtual address of newly allocated buffer in the address space
    // 为每次分配的新内存区域定位虚拟内存地址
    consumed_page_count: usize,
    // 用户地址空间基地址，在这之前的东西是未定义的
    base_address: usize,
}

impl RwLockUserAddrSpace {
    pub unsafe fn new(context: &Arc<RwSpinlock<Context>>, base: usize) -> Arc<Self> {
        let mut addrsp = UserAddrSpace::new(base);
        addrsp.setup_kernel();

        Arc::new(Self {
            context: Arc::clone(context),
            inner: Arc::new(RwLock::new(addrsp))
        })
    }

    pub fn acquire_read<'a>(self: &'a Arc<Self>) -> RwLockReadGuard<'a, UserAddrSpace> {
        loop {
            match self.inner.try_read() {
                Some(g) => return g,
                None => { spin_loop() }
            }
        }
    }

    pub fn acquire_upg_read<'a>(self: &'a Arc<Self>) -> RwLockUpgradableGuard<'a, UserAddrSpace> {
        loop {
            match self.inner.try_upgradeable_read() {
                Some(g) => return g,
                None => { spin_loop() }
            }
        }
    }

    pub fn acquire_write<'a>(self: &'a Arc<Self>) -> RwLockWriteGuard<'a, UserAddrSpace> {
        loop {
            match self.inner.try_write() {
                Some(g  ) => return g,
                None => { spin_loop() }
            }
        }
    }
}

impl UserAddrSpace {
    pub unsafe fn new(base: usize) -> Self {
        assert_eq!(base % PAGE_SIZE, 0, "base address of userspace address space must be 4k aligned.");

        let pml4_frame = frame_alloc().or_panic("failed to allocate new frame for user addr space page table");

        let ptr = pml4_frame.start_address().as_u64() as *mut PageTable;
        ptr.write(PageTable::new());

        let mut offset_page_table = OffsetPageTable::new(&mut *ptr, VirtAddr::new(0));

        let small_init_frame = TrackedPhysFrame {
            frame: frame_alloc().or_panic("failed to allocate new frame for user addr space page buffers"),
            index: 0
        };
        let medium_init_frame = TrackedPhysFrame {
            frame: frame_alloc().or_panic("failed to allocate new frame for user addr space page buffers"),
            index: 1
        };

        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;

        let mut pte_frames = vec![];
        let mut temp_allocator = TempAllocator(&mut pte_frames);

        offset_page_table.map_to(
            Page::containing_address(VirtAddr::new((base + 0 * PAGE_SIZE) as u64)),
            small_init_frame.frame,
            flags,
            &mut temp_allocator
        )
            .or_panic("failed to map small tracked page.")
            .flush();
        offset_page_table.map_to(
            Page::containing_address(VirtAddr::new((base + 1 * PAGE_SIZE) as u64)),
            medium_init_frame.frame,
            flags,
            &mut temp_allocator
        )
            .or_panic("failed to map medium tracked page.")
            .flush();

        Self {
            page_table: offset_page_table,
            pte_frames,
            tracked_large_buffers: vec![],
            tracked_medium_buffers: vec![medium_init_frame],
            medium_buffer_pointer: 0,
            tracked_small_buffers: vec![small_init_frame],
            small_buffer_pointer: 0,
            consumed_page_count: 2, // index 0 and 1 is used
            base_address: base,
        }
    }

    pub unsafe fn setup_kernel(&mut self) {
        // map kernel pml4 page table identically
        let mut pt = self.page_table.level_4_table();
        let kernel_pml4_pt = &*(get_kernel_pml4_page_table_addr() as *const PageTable);

        pt[KERNEL_BYTES_P4 as usize] = kernel_pml4_pt[KERNEL_BYTES_P4 as usize].clone();
        pt[BOOTSTRAP_BYTES_P4 as usize] = kernel_pml4_pt[BOOTSTRAP_BYTES_P4 as usize].clone();
        pt[KERNEL_STACK_P4 as usize] = kernel_pml4_pt[KERNEL_STACK_P4 as usize].clone();
        pt[FRAMEBUFFER_P4 as usize] = kernel_pml4_pt[FRAMEBUFFER_P4 as usize].clone();
        pt[PHYS_MEM_P4 as usize] = kernel_pml4_pt[PHYS_MEM_P4 as usize].clone();
    }

    pub fn alloc(&mut self, size: usize) -> Arc<UserBuffer> {
        match size {
            ..=64 => unsafe {
                if size + self.small_buffer_pointer > PAGE_SIZE {
                    let new_frame = TrackedPhysFrame {
                        frame: frame_alloc().or_panic("failed to allocate new frame for small buffer of user addr space"),
                        index: self.next_page_unused()
                    };
                    let virt_addr = VirtAddr::new((self.base_address + new_frame.index * PAGE_SIZE) as u64);

                    self.page_table.map_to(
                        Page::containing_address(virt_addr),
                        new_frame.frame.clone(),
                        PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
                        // SAFETY: FrameAllocator as self only modifies `self.pte_frames`
                        //         so it is safe to leak it
                        // TODO: leak borrow convention，好孩子不要这样，最好是为 pte_frames 实现 FrameAllocator
                        &mut *(self as *const Self as u64 as *mut Self)
                    )
                        .or_panic("failed to map newly allocated small buffer")
                        .flush();

                    self.tracked_small_buffers.push(new_frame);
                    self.consumed_page_count += 1;
                    self.small_buffer_pointer = size;

                    Arc::new(UserBuffer::new(virt_addr.as_u64(), size))
                } else {
                    let last_frame = self.tracked_small_buffers.last()
                        .or_panic("failed to get last tracked small buffer");
                    let virt_addr = VirtAddr::new((self.base_address + last_frame.index * PAGE_SIZE) as u64);

                    self.small_buffer_pointer += size;
                    Arc::new(UserBuffer::new(virt_addr.as_u64(), size))
                }
            }
            65..=512 => unsafe {
                if size + self.medium_buffer_pointer > PAGE_SIZE {
                    let new_frame = TrackedPhysFrame {
                        frame: frame_alloc().or_panic("failed to allocate new frame for medium buffer of user addr space"),
                        index: self.next_page_unused()
                    };
                    let virt_addr = VirtAddr::new((self.base_address + new_frame.index * PAGE_SIZE) as u64);

                    self.page_table.map_to(
                        Page::containing_address(virt_addr),
                        new_frame.frame.clone(),
                        PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
                        &mut *(self as *const Self as u64 as *mut Self) // leak borrow convention
                    )
                        .or_panic("failed to map newly allocated small buffer")
                        .flush();

                    self.tracked_medium_buffers.push(new_frame);
                    self.consumed_page_count += 1;
                    self.medium_buffer_pointer = size;

                    Arc::new(UserBuffer::new(virt_addr.as_u64(), size))
                } else {
                    let last_frame = self.tracked_medium_buffers.last()
                        .or_panic("failed to get last tracked small buffer");
                    let virt_addr = VirtAddr::new((self.base_address + last_frame.index * PAGE_SIZE) as u64);

                    self.medium_buffer_pointer += size;
                    Arc::new(UserBuffer::new(virt_addr.as_u64(), size))
                }
            }
            _ => unsafe {
                let required_pages = size.div_ceil(PAGE_SIZE);
                let virt_addr = VirtAddr::new((self.base_address + self.next_page_unused() * PAGE_SIZE) as u64);
                let start_page = Page::<Size4KiB>::containing_address(virt_addr);

                for page in Page::range(start_page, start_page + required_pages as u64) {
                    let frame = frame_alloc().or_panic("failed to allocate new frame for large buffer of user addr space");

                    self.page_table.map_to(
                        page,
                        frame.clone(),
                        PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
                        &mut *(self as *const Self as u64 as *mut Self) // leak borrow convention
                    )
                        .or_panic("failed to map newly allocated small buffer")
                        .flush();

                    self.tracked_large_buffers.push(frame)
                }

                self.consumed_page_count += required_pages;
                Arc::new(UserBuffer::new(virt_addr.as_u64(), size))
            }
        }
    }

    // resolve userspace buffer to kernel space
    pub fn resolve(&self, buffer: Arc<UserBuffer>) -> KResult<Vec<&'static [u8]>> {
        if buffer.len() <= 512 { // alloc 不会把小于 512 的内存区域分页
            let virt_addr = VirtAddr::new(buffer.ptr() as u64);
            let page = Page::<Size4KiB>::containing_address(virt_addr);

            let translated = self.page_table.translate_page(page).map_err(|_| KError::new(EFAULT))?;
            let phys_addr = translated.start_address().as_u64() + (virt_addr.as_u64() - page.start_address().as_u64());

            return Ok(vec![unsafe { slice::from_raw_parts(phys_addr as *const u8, buffer.len()) }]);
        }

        let mut result = Vec::new();
        let mut resolved_len = 0;
        let mut base_virt_addr = buffer.ptr();

        while resolved_len < buffer.len() {
            let virt_addr = VirtAddr::new(unsafe { base_virt_addr.add(resolved_len) } as u64);
            let page = Page::<Size4KiB>::containing_address(virt_addr);

            let translated = self.page_table.translate_page(page).map_err(|_| KError::new(EFAULT))?;
            let phys_addr = translated.start_address().as_u64() + (virt_addr - page.start_address());

            let len_till_page_end = (page + 1).start_address() - virt_addr;
            let remain_bytes_len = (buffer.len() - resolved_len) as u64;
            if len_till_page_end < remain_bytes_len { // 只有部分 buffer
                result.push(unsafe { slice::from_raw_parts(phys_addr as *const u8, len_till_page_end as usize) });
                resolved_len += len_till_page_end as usize;
            } else {
                result.push(unsafe { slice::from_raw_parts(phys_addr as *const u8, remain_bytes_len as usize) });
                resolved_len += remain_bytes_len as usize;
            }
        }

        Ok(result)
    }

    pub fn alloc_and_copy_from(&mut self, src: &[u8]) -> KResult<Arc<UserBuffer>> {
        let allocated = self.alloc(src.len());
        let mut resolved = self.resolve(Arc::clone(&allocated))?;

        assert_eq!(resolved.iter().map(|slice| slice.len()).sum::<usize>(), src.len(), "resolved len is not equal to src");

        let mut start = 0;
        for slice in resolved.into_iter() {
            // TODO: check copy to
            unsafe { copy_to(
                slice[0] as *mut u8 as usize,
                src[start..][0] as *mut u8 as usize,
                slice.len()
            ); }
        }

        return Ok(allocated)
    }

    pub fn next_page_unused(&mut self) -> usize {
        loop {
            let virt_addr = VirtAddr::new((self.base_address + self.consumed_page_count * PAGE_SIZE) as u64);
            let used = self.page_table.translate_addr(virt_addr).map(|_| true).unwrap_or(false);
            if !used {
                return self.consumed_page_count
            }
            self.consumed_page_count += 1;
        }
    }

    // get reference of the underlying page table
    pub unsafe fn page_table<'a>(&'a mut self) -> &'a mut PageTable {
        self.page_table.level_4_table()
    }

    // preform raw map
    pub unsafe fn raw_map_to(&mut self, page: Page, frame: PhysFrame, flags: PageTableFlags) {
        self.page_table.map_to(
            page,
            frame,
            flags,
            &mut *(self as *const Self as u64 as *mut Self)
        )
            .or_panic("failed to perform raw map_to")
            .flush();
    }

    pub unsafe fn raw_unmap(&mut self, page: Page) {
        let (p1_entry, flusher) = self.page_table.unmap(page).or_panic("failed to perform raw unmap");
        frame_dealloc(p1_entry);
        flusher.flush();
    }

    pub unsafe fn raw_translate(&mut self, virt_addr: VirtAddr) -> TranslateResult {
        self.page_table.translate(virt_addr)
    }

    pub unsafe fn raw_update_flags(&mut self, page: Page, flags: PageTableFlags) {
        self.page_table.update_flags(page, flags)
            .or_panic("failed to perform raw update flags")
            .flush()
    }

    pub unsafe fn push_tracked_frame(&mut self, frame: PhysFrame) {
        self.tracked_large_buffers.push(frame)
    }

    pub unsafe fn validate(&mut self) {
        let pg_phys_addr = self.page_table.level_4_table() as *const _ as u64;
        Cr3::write(PhysFrame::containing_address(PhysAddr::new(pg_phys_addr)), Cr3Flags::empty())
    }
}

/**
 *  tracked physics frame at userspace address space
 */
struct TrackedPhysFrame {
    frame: PhysFrame,
    // page index starting from base address
    index: usize
}

unsafe impl FrameAllocator<Size4KiB> for UserAddrSpace {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        let frame = frame_alloc()
            .expect("failed to allocate new pte for addr space page table");
        self.pte_frames.push(frame.clone());
        Some(frame)
    }
}

struct TempAllocator<'a>(&'a mut Vec<PhysFrame>);

unsafe impl FrameAllocator<Size4KiB> for TempAllocator<'_> {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        let frame = frame_alloc()
            .expect("failed to allocate new pte for addr space page table");
        self.0.push(frame.clone());
        Some(frame)
    }
}

impl Drop for UserAddrSpace {
    fn drop(&mut self) {
        for frame in self.tracked_small_buffers.iter() {
            frame_dealloc(frame.frame)
        }
        for frame in self.tracked_medium_buffers.iter() {
            frame_dealloc(frame.frame)
        }
        for frame in self.tracked_large_buffers.iter() {
            frame_dealloc(*frame)
        }

        for frame in self.pte_frames.iter() {
            frame_dealloc(*frame)
        }

        frame_dealloc(PhysFrame::containing_address(
            PhysAddr::new(self.page_table.level_4_table() as *const _ as u64)
        ));
    }
}