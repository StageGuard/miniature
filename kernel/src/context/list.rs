use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use core::cell::{Cell, RefCell};
use core::hint::spin_loop;
use core::mem::{offset_of, size_of};
use core::ops::{Add, Index, RangeBounds};
use core::ptr;
use core::ptr::{slice_from_raw_parts, slice_from_raw_parts_mut};
use core::slice::from_raw_parts;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use lazy_static::lazy_static;
use log::info;
use spin::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use spinning_top::RwSpinlock;
use x86_64::structures::paging::{Page, PageTableFlags, PhysFrame, Size4KiB};
use x86_64::{PhysAddr, VirtAddr};
use x86_64::structures::paging::mapper::TranslateResult;
use shared::print_panic::PrintPanic;
use shared::uni_processor::UPSafeCell;
use crate::context::{Context, ContextId};
use crate::{CPU_COUNT, infohart, qemu_println, warnhart};
use crate::mem::aligned_box::AlignedBox;
use crate::mem::heap::OutOfMemory;
use crate::mem::PAGE_SIZE;
use crate::syscall::{enter_usermode, InterruptStack, IretRegisters};
use libvdso::error::{EAGAIN, ENOMEM};
use crate::mem::frame_allocator::frame_alloc_n;
use crate::mem::user_addr_space::RwLockUserAddrSpace;

lazy_static! {
    static ref CONTEXT_STORAGE: RwLock<ContextStorage> = {
        let cpu_count = CPU_COUNT.load(Ordering::Relaxed);
        RwLock::new(ContextStorage::new(cpu_count as usize))
    };
}

struct ContextIdAllocator {
    lock: AtomicBool,
    head: UPSafeCell<usize>,
    recycled: UPSafeCell<VecDeque<usize>>
}

impl ContextIdAllocator {
    pub fn new(init_id_exclusive: usize) -> Self {
        ContextIdAllocator {
            lock: AtomicBool::new(false),
            head: unsafe { UPSafeCell::new(init_id_exclusive) },
            recycled: unsafe { UPSafeCell::new(VecDeque::new()) }
        }
    }

    pub fn dealloc(&self, id: usize) -> usize {
        while self.lock.compare_exchange_weak(false, true, Ordering::SeqCst, Ordering::Relaxed).is_err() {
            spin_loop()
        }

        let mut queue_mut = self.recycled.inner_exclusive_mut();
        let result = match queue_mut.iter().find(|i| *i == &id) {
            None => {
                queue_mut.push_back(id);
                id
            }
            Some(_) => 0
        };

        self.lock.store(false, Ordering::SeqCst);
        result
    }

    pub fn alloc(&self) -> usize {
        while self.lock.compare_exchange_weak(false, true, Ordering::SeqCst, Ordering::Relaxed).is_err() {
            spin_loop()
        }

        let mut queue_mut = self.recycled.inner_exclusive_mut();
        let result = match queue_mut.pop_front() {
            None => {
                let mut head = self.head.inner_exclusive_mut();
                *head = *head + 1;
                *head
            }
            Some(id) => id,
        };

        self.lock.store(false, Ordering::SeqCst);
        result
    }
}

pub struct ContextStorage {
    map: BTreeMap<ContextId, Arc<RwSpinlock<Context>>>,
    id_allocator: ContextIdAllocator
}

impl ContextStorage {
    pub fn new(init_id_exclusive: usize) -> Self {
        ContextStorage {
            map: BTreeMap::new(),
            id_allocator: ContextIdAllocator::new(init_id_exclusive),
        }
    }
    /// Get the current context.
    pub fn current(&self) -> Option<&Arc<RwSpinlock<Context>>> {
        self.map.get(&super::context_id())
    }

    pub fn insert_context(&mut self, id: ContextId) -> Result<&Arc<RwSpinlock<Context>>, i32> {
        let old = self.map.insert(id, Arc::new(RwSpinlock::new(Context::new(id))));
        if old.is_some() {
            warnhart!("insert duplicated context id: {}", id.0);
            Err(EAGAIN)
        } else {
            Ok(self.map.get(&id).or_panic("failed to get newly inserted context"))
        }
    }

    pub fn remove(&mut self, id: ContextId) -> Option<Arc<RwSpinlock<Context>>> {
        self.map.remove(&id)
    }

    pub fn new_context(&mut self) -> Result<&Arc<RwSpinlock<Context>>, i32> {
        self.insert_context(ContextId::from(self.id_allocator.alloc()))
    }

    pub fn spawn(
        &mut self,
        userspace_allowed: bool,
        func: extern "C" fn()
    ) -> Result<&Arc<RwSpinlock<Context>>, i32> {
        let mut stack = match frame_alloc_n(64) {
            Some(frame) => unsafe {
                let ptr = frame.start_address().as_u64() as *mut u8;
                ptr::write_bytes(ptr, 0, PAGE_SIZE * 64);
                slice_from_raw_parts_mut(ptr, PAGE_SIZE * 64)
            }
            None => return Err(ENOMEM)
        };

        let new_context_lock = self.new_context()?;
        let mut new_context = new_context_lock.write();
        let addrsp = unsafe { RwLockUserAddrSpace::new(&new_context_lock, 0x1000) };

        {   // make kernel stack accessible for user space
            let mut rsp_cloned = Arc::clone(&addrsp);
            let mut rsp_guard = rsp_cloned.acquire_write();
            // 0x7fc0000000 是 PageTable[0][510] 1gb 页的起始虚拟地址
            let kstack_start_page = Page::<Size4KiB>::containing_address(VirtAddr::new(0x7f_8000_0000));
            let kstack_start_frame = PhysFrame::containing_address(PhysAddr::new(stack.as_mut_ptr() as u64));
            // stack start may not 4k aligned, so update one more page
            for page in Page::range(kstack_start_page, kstack_start_page + 64) {
                unsafe {
                    rsp_guard.raw_map_to(
                        page,
                        kstack_start_frame + (page - kstack_start_page),
                        PageTableFlags::PRESENT |
                            PageTableFlags::USER_ACCESSIBLE |
                            PageTableFlags::WRITABLE |
                            PageTableFlags::NO_EXECUTE
                    )
                }
            }
        }

        new_context.set_addr_space(Some(addrsp));

        infohart!("stack: {:x}", stack.as_mut_ptr() as u64);
        let mut stack_top = unsafe { stack.as_mut_ptr().add(PAGE_SIZE * 64) };
        infohart!("stack: {:x}", stack_top as u64);
        const INT_REGS_SIZE: usize = size_of::<InterruptStack>();

        unsafe {
            if userspace_allowed {
                // Zero-initialize InterruptStack registers.
                stack_top = stack_top.sub(INT_REGS_SIZE);
                stack_top.write_bytes(0_u8, INT_REGS_SIZE);
                let intr_stack = &mut *stack_top.cast::<InterruptStack>();
                intr_stack.init();
                let rsp_field_offset = offset_of!(InterruptStack, iret) + offset_of!(IretRegisters, rsp);
                intr_stack.set_stack_pointer(0x7f_8000_0000 + PAGE_SIZE * 64 - INT_REGS_SIZE + rsp_field_offset + size_of::<usize>());

                stack_top = stack_top.sub(size_of::<usize>());
                stack_top.cast::<usize>().write(enter_usermode as usize);
            }

            stack_top = stack_top.sub(size_of::<usize>());
            stack_top.cast::<usize>().write(func as usize);
        }

        new_context.ctx_regs.set_stack_pointer(stack_top as usize);
        new_context.kstack = Some(unsafe { &*stack });
        new_context.userspace = userspace_allowed;

        drop(new_context);
        Ok(new_context_lock)
    }


    pub fn iter(
        &self,
    ) -> ::alloc::collections::btree_map::Iter<ContextId, Arc<RwSpinlock<Context>>> {
        self.map.iter()
    }

    pub fn range(
        &self,
        range: impl RangeBounds<ContextId>,
    ) -> ::alloc::collections::btree_map::Range<'_, ContextId, Arc<RwSpinlock<Context>>> {
        self.map.range(range)
    }
}

impl Index<ContextId> for ContextStorage {
    type Output = Arc<RwSpinlock<Context>>;

    fn index(&self, index: ContextId) -> &Self::Output {
        self.map.get(&index).or_panic("failed to get context")
    }
}

/// Get the global context list, const
pub fn context_storage() -> RwLockReadGuard<'static, ContextStorage> {
    CONTEXT_STORAGE.read()
}

/// Get the global context list, mutable
pub fn context_storage_mut() -> RwLockWriteGuard<'static, ContextStorage> {
    CONTEXT_STORAGE.write()
}

#[test_case]
pub(crate) fn test_context_id_allocator() {
    let allocator = ContextIdAllocator::new(0);

    assert_eq!(allocator.alloc(), 1);
    assert_eq!(allocator.alloc(), 2);
    assert_eq!(allocator.alloc(), 3);
    assert_eq!(allocator.alloc(), 4);
    assert_eq!(allocator.alloc(), 5);
    assert_eq!(allocator.alloc(), 6);
    assert_eq!(allocator.alloc(), 7);
    assert_eq!(allocator.alloc(), 8);
    assert_eq!(allocator.alloc(), 9);

    assert_eq!(allocator.dealloc(10), 0);
    assert_eq!(allocator.dealloc(3), 3);
    assert_eq!(allocator.dealloc(4), 5);
    assert_eq!(allocator.dealloc(4), 5);

    assert_eq!(allocator.alloc(), 3);
    assert_eq!(allocator.alloc(), 4);
    assert_eq!(allocator.alloc(), 5);
    assert_eq!(allocator.alloc(), 10);
}