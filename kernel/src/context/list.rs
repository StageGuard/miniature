use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use core::cell::{Cell, RefCell};
use core::hint::spin_loop;
use core::ops::{Add, Index, RangeBounds};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use lazy_static::lazy_static;
use spin::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use spinning_top::RwSpinlock;
use shared::print_panic::PrintPanic;
use shared::uni_processor::UPSafeCell;
use crate::context::{Context, ContextId};

lazy_static! {
    static ref CONTEXT_STORAGE: RwLock<ContextStorage> = RwLock::new(ContextStorage::new());
}

struct ContextIdAllocator {
    lock: AtomicBool,
    head: UPSafeCell<usize>,
    recycled: UPSafeCell<VecDeque<usize>>
}

impl ContextIdAllocator {
    pub fn new() -> Self {
        ContextIdAllocator {
            lock: AtomicBool::new(false),
            head: unsafe { UPSafeCell::new(1) },
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
    pub fn new() -> Self {
        ContextStorage {
            map: BTreeMap::new(),
            id_allocator: ContextIdAllocator::new(),
        }
    }
    /// Get the current context.
    pub fn current(&self) -> Option<&Arc<RwSpinlock<Context>>> {
        self.map.get(&super::context_id())
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
    let allocator = ContextIdAllocator::new();

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