use core::{ops::{Index, Range}, usize};

use alloc::rc::Rc;
use log::warn;
use spin::MutexGuard;
use x86_64::{structures::paging::{PageTable, page_table::{PageTableEntry, PageTableLevel}, PageTableIndex, mapper::{MapperAllSizes, CleanUp}, Translate, Mapper, Size4KiB, Page}, VirtAddr};

const PTE_COUNT_PER_PT: usize = 512;

/// A `PageTable` wrapper that can track if it is allocated.
pub struct TrackedMapper<PT> {
    inner: PT,
    level: PageTableLevel,
    used: [bool; 512],
    // only for optimizing internal indexing
    first_available_index: usize,
    last_available_index: usize,
}

impl <PT> TrackedMapper<PT>
where
    PT : Mapper<Size4KiB> + Translate
{
    pub fn new(table: PT, level: PageTableLevel) -> Self {
        Self { 
            inner: table, 
            level, 
            used: [false; PTE_COUNT_PER_PT],
            first_available_index: 0,
            last_available_index: PTE_COUNT_PER_PT - 1,
        }
    }


    /// 标记此 PTE 已被使用，会更新内部索引优化
    /// 标记失败返回 None
    pub fn mark_as_used(&mut self, index: usize) -> Option<PageTableIndex> {
        if index < 0 || index > 512 {
            return None
        }

        if self.used[index] == true {
            return None
        }
        self.used[index] = true;

        if self.first_available_index == index {
            // 第一个可用 PTE 索引被标记使用，则寻找下一个可用的作为第一个可用
            // 此时 curr index 及前面的都已经使用
            let mut curr = self.first_available_index;
            while curr < 512 && self.used[curr] { 
                curr += 1;
            }

            self.first_available_index = curr;
        } else if self.last_available_index == index {
            // 最后一个可用 pTE 索引被标记使用，则寻找上一个可用作为最后一个可用
            // 此时 curr index 及后面的都已经使用
            let mut curr = self.last_available_index;
            while curr > 0 && self.used[curr] { 
                curr -= 1;
            }

            self.last_available_index -= curr;
        }

        Some(PageTableIndex::new(index as u16))
    }

    // 标记此 PTE 未被使用，会更新内部索引优化
    // 不会返回任何
    pub fn mark_as_unused(&mut self, index: usize) {
        if index < 0 || index > 512 {
            return
        }

        self.used[index] = false;

        // update index
        if self.first_available_index > index {
            self.first_available_index = index;
        } else if self.last_available_index < index {
            self.last_available_index = index;
        }
    }

    // 获取指定虚拟内存位置，[size] 大小的 PTE 索引，并标记为已使用
    // 如果所有 PTE 都标记失败就返回 None，但是不影响已经标记好的
    pub fn mark_range_as_used(&mut self, range: Range<VirtAddr>) -> Option<(PageTableIndex, usize)> {
        let start = Page::<Size4KiB>::containing_address(range.start);
        let end_exclusive = Page::<Size4KiB>::containing_address(range.end - 1usize);

        let pti_range = match self.level {
            PageTableLevel::Four => u16::from(start.p4_index())..=u16::from(end_exclusive.p4_index()),
            PageTableLevel::Three => u16::from(start.p3_index())..=u16::from(end_exclusive.p3_index()),
            PageTableLevel::Two => u16::from(start.p2_index())..=u16::from(end_exclusive.p2_index()),
            PageTableLevel::One => u16::from(start.p1_index())..=u16::from(end_exclusive.p1_index()),
        };

        let mut partial_mark_success = false;
        for pti_idx in pti_range.clone() {
            if self.mark_as_used(pti_idx as usize).is_some() {
                partial_mark_success = true
            }
        }

        return if partial_mark_success {
            Some((PageTableIndex::new(*pti_range.start()), pti_range.len()))
        } else { None }
    }

    /// 获取每个 PTE 最大能索引多少内存
    fn total_available_mem_size(&self) -> usize {
        match self.level {
            PageTableLevel::One => 4096,
            PageTableLevel::Two => 4096 * PTE_COUNT_PER_PT,
            PageTableLevel::Three => 4096 * PTE_COUNT_PER_PT * PTE_COUNT_PER_PT,
            PageTableLevel::Four => 4096 * PTE_COUNT_PER_PT * PTE_COUNT_PER_PT * PTE_COUNT_PER_PT,
        }
    }

    /// 获取可以索引 [size] 大小的 PTE，返回一个 PTE 索引和需要的 PTE 个数，并标记为已使用
    /// 也就是说这个索引对应的页表或者往后的几个页表一共可以索引大于等于 [size] 大小的内存
    /// 如果 [reverse] 为 true，则从页表末尾开始找
    /// 找不到可以用的 PTE 会返回 None
    pub fn find_free_space_and_mark(&mut self, size: usize, reverse: bool)-> Option<(PageTableIndex, usize)> {
        // calculate required count of child PageTableEnry to index the size of memory
        let each_pte_max_size = self.total_available_mem_size();
        let required_pte_size = (size + (each_pte_max_size - 1)) / each_pte_max_size;
        
        if i32::from(self.last_available_index as u16) - i32::from(self.first_available_index as u16) + 1 < i32::from(required_pte_size as u16) {
            // not available child PageTableEnry to distrubute
            return None
        }

        // fast path
        if required_pte_size == 1 {
            return self.mark_as_used(self.first_available_index).map(|pte| (pte, 1))
        }

        let mut curr_idx = if reverse { self.last_available_index } else { self.first_available_index };

        let len = required_pte_size;
        while (required_pte_size..(PTE_COUNT_PER_PT-required_pte_size-1)).contains(&curr_idx) {
            // 如果当前索引的 PTE 可用
            if self.used[curr_idx] == false {
                // 获取对应长度的 PTE 片段
                let range = if reverse { curr_idx-len..curr_idx } else { curr_idx..curr_idx+len };
                let target_pte_slice = &self.used[range.clone()];

                // 如果此片段所有的都可用，那就返回
                if target_pte_slice.iter().all(|state| !*state) {
                    // 标记这些 PTE 片段被使用了
                    if reverse { 
                        range.rev().for_each(|pte_index: usize| { self.mark_as_used(pte_index); });
                    } else { 
                        range.for_each(|pte_index: usize| { self.mark_as_used(pte_index); });
                    }

                    return Some((PageTableIndex::new(if reverse { curr_idx-len } else { curr_idx } as u16), len))
                }

                // 这些片段其中有些不可用，继续迭代
            }

            // 当前片段不可用，继续迭代
            if reverse { curr_idx -= 1 } else { curr_idx += 1 }
        }

        return None
    }
}