use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use core::cell::{Cell, RefCell};
use core::hint::spin_loop;
use core::mem::size_of;
use core::ops::{Add, Index, RangeBounds};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use lazy_static::lazy_static;
use spin::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use spinning_top::RwSpinlock;
use shared::print_panic::PrintPanic;
use shared::uni_processor::UPSafeCell;
use crate::context::{Context, ContextId};
use crate::{CPU_COUNT, infohart, warnhart};
use crate::mem::aligned_box::AlignedBox;
use crate::mem::heap::OutOfMemory;
use crate::mem::PAGE_SIZE;
use crate::syscall::{enter_usermode, InterruptStack};
use libvdso::error::{EAGAIN, ENOMEM};
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
        let mut stack = match AlignedBox::<[u8; PAGE_SIZE * 64], { PAGE_SIZE }>::try_zeroed() {
            Ok(value) => { value }
            Err(OutOfMemory) => { return Err(ENOMEM) }
        };

        let new_context_lock = self.new_context()?;
        let mut new_context = new_context_lock.write();
        new_context.set_addr_space(unsafe { Some(RwLockUserAddrSpace::new(&new_context_lock, 0x1000)) });

        let mut stack_top = unsafe { stack.as_mut_ptr().add(PAGE_SIZE * 64) };
        const INT_REGS_SIZE: usize = size_of::<InterruptStack>();

        unsafe {
            if userspace_allowed {
                // Zero-initialize InterruptStack registers.
                stack_top = stack_top.sub(INT_REGS_SIZE);
                stack_top.write_bytes(0_u8, INT_REGS_SIZE);
                (&mut *stack_top.cast::<InterruptStack>()).init();

                stack_top = stack_top.sub(size_of::<usize>());
                stack_top.cast::<usize>().write(enter_usermode as usize);
            }

            stack_top = stack_top.sub(size_of::<usize>());
            stack_top.cast::<usize>().write(func as usize);
        }

        new_context.ctx_regs.set_stack_pointer(stack_top as usize);
        new_context.kstack = Some(stack);
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