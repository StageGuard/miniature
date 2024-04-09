use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::alloc::AllocError;
use core::hint::spin_loop;
use core::iter::repeat;
use core::mem;
use core::ops::{Deref, DerefMut};
use spin::{RwLock, RwLockReadGuard, RwLockUpgradableGuard, RwLockWriteGuard};
use spin::lock_api::RwLockUpgradableReadGuard;
use spinning_top::RwSpinlock;
use x86_64::{PhysAddr, VirtAddr};
use x86_64::structures::paging::{FrameAllocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame, Size4KiB};
use x86_64::structures::paging::mapper::CleanUp;
use shared::print_panic::PrintPanic;
use crate::context::Context;
use crate::mem::frame_allocator::{frame_alloc, frame_dealloc};
use crate::mem::PAGE_SIZE;
use crate::mem::user_buffer::UserBuffer;

pub struct ArcRwLockUserAddrSpace {
    context: Arc<RwSpinlock<Context>>,
    inner: Arc<RwLock<UserAddrSpace>>
}

pub struct UserAddrSpace {
    page_table: OffsetPageTable<'static>,
    // 地址空间页表用到的子页表物理页帧
    pte_frames: Vec<PhysFrame>,
    // track buffers which length > PAGE_SIZE
    tracked_large_buffers: Vec<PhysFrame>,
    large_buffer_pointer: usize,
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

impl ArcRwLockUserAddrSpace {
    pub unsafe fn new(context: &Arc<RwSpinlock<Context>>, base: usize) -> Self {
        Self {
            context: Arc::clone(context),
            inner: Arc::new(RwLock::new(UserAddrSpace::new(base)))
        }
    }

    fn acquire_read(&self) -> RwLockReadGuard<'_, UserAddrSpace> {
        loop {
            match self.inner.try_read() {
                Some(g) => return g,
                None => { spin_loop() }
            }
        }
    }

    fn acquire_upg_read(&self) -> RwLockUpgradableGuard<'_, UserAddrSpace> {
        loop {
            match self.inner.try_upgradeable_read() {
                Some(g) => return g,
                None => { spin_loop() }
            }
        }
    }

    fn acquire_write(&self) -> RwLockWriteGuard<'_, UserAddrSpace> {
        loop {
            match self.inner.try_write() {
                Some(g) => return g,
                None => { spin_loop() }
            }
        }
    }
}

impl UserAddrSpace {
    pub unsafe fn new(base: usize) -> Self {
        assert_eq!(base % PAGE_SIZE, 0, "base address of userspace address space must be 4k aligned.");

        let pml4_frame = frame_alloc()
            .or_panic("failed to allocate new frame for user addr space page table");

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
            large_buffer_pointer: 0,
            tracked_medium_buffers: vec![medium_init_frame],
            medium_buffer_pointer: 0,
            tracked_small_buffers: vec![small_init_frame],
            small_buffer_pointer: 0,
            consumed_page_count: 2, // index 0 and 1 is used
            base_address: base,
        }
    }

    pub fn alloc(&mut self, size: usize) -> Arc<UserBuffer> {
        match size {
            ..=64 => unsafe {
                if size + self.small_buffer_pointer > PAGE_SIZE {
                    let new_frame = TrackedPhysFrame {
                        frame: frame_alloc().or_panic("failed to allocate new frame for small buffer of user addr space"),
                        index: self.consumed_page_count
                    };
                    let virt_addr = VirtAddr::new((self.base_address + new_frame.index * PAGE_SIZE) as u64);

                    self.page_table.map_to(
                        Page::containing_address(virt_addr),
                        new_frame.frame.clone(),
                        PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
                        &mut *(self as *const Self as u64 as *mut Self) // TODO: leak borrow convention，好孩子不要这样，最好是为 pte_frames 实现 FrameAllocator
                    )
                        .or_panic("failed to map newly allocated small buffer")
                        .ignore();

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
                        index: self.consumed_page_count
                    };
                    let virt_addr = VirtAddr::new((self.base_address + new_frame.index * PAGE_SIZE) as u64);

                    self.page_table.map_to(
                        Page::containing_address(virt_addr),
                        new_frame.frame.clone(),
                        PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
                        &mut *(self as *const Self as u64 as *mut Self) // leak borrow convention
                    )
                        .or_panic("failed to map newly allocated small buffer")
                        .ignore();

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
                let virt_addr = VirtAddr::new((self.base_address + self.consumed_page_count * PAGE_SIZE) as u64);
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
                        .ignore();

                    self.tracked_large_buffers.push(frame)
                }

                self.consumed_page_count += required_pages;
                Arc::new(UserBuffer::new(virt_addr.as_u64(), size))
            }
        }
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