use core::{mem::{transmute, MaybeUninit}, ops::Range};
use shared::arg::MemoryRegion;
use x86_64::{align_up, structures::paging::{FrameAllocator, PhysFrame, Size4KiB}, PhysAddr, VirtAddr};




const MAX_RANGE_COUNT: usize = 512;

pub struct LinearIncFrameAllocator {
    range_iterator: LinkedRangeIterator,
    base_address: u64,
    phys_mem_right_boundary: u64,
}

impl LinearIncFrameAllocator {
    pub fn new(
        phy_start_addr: VirtAddr,
        window: u64,
        phys_mem_size: u64,
        unav_regions: &[MemoryRegion]
    ) -> Self {
        let iter = LinkedRangeIterator::from_memory_regions(0x1000, window, unav_regions);

        Self { 
            range_iterator: iter, 
            base_address: phy_start_addr.as_u64(), 
            phys_mem_right_boundary: phy_start_addr.as_u64() + phys_mem_size 
        }
    }

    pub fn next(&mut self) -> Option<u64> {
        self.range_iterator.next()
    }
}

unsafe impl FrameAllocator<Size4KiB> for LinearIncFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        let phys_addr = self.next()?;

        // out of memory
        if phys_addr + 0x1000 > self.phys_mem_right_boundary {
            panic!("out of memory");
            return None
        }

        let phys_addr = PhysAddr::new(self.base_address + phys_addr);
        Some(PhysFrame::containing_address(phys_addr))
    }
}


struct LinkedRangeIterator {
    /// ranges without intersect, end exclusive
    ranges: [Range<u64>; MAX_RANGE_COUNT],
    range_size: usize,
    window: u64,

    current_range_index: usize,
    current_value: u64,
}

impl Iterator for LinkedRangeIterator {
    type Item = u64;

    fn next(&mut self) -> Option<Self::Item> {
        let mut curr = self.current_value;

        // iterates over the last range.
        if self.current_range_index == self.range_size {
            self.current_value += self.window;
            return Some(self.current_value);
        }

        // if not overlapped with next range.
        if curr + self.window < self.ranges[self.current_range_index].start {
            self.current_value += self.window;
            return Some(self.current_value);
        }

        let mut overlapped = true;

        while overlapped && self.current_range_index < self.range_size {
            let current_range = &self.ranges[self.current_range_index];
            curr = self.next_windowed_after(curr, current_range.end);

            self.current_range_index += 1;

            let next_range = &self.ranges[self.current_range_index];
            overlapped = next_range.contains(&curr);
        }

        self.current_value = curr;
        Some(curr)
    }
}

impl LinkedRangeIterator {
    /// caller should ensure the regions is sorted by `MemoryRegion.start`.
    fn from_memory_regions(start: u64, window: u64, regions: &[MemoryRegion]) -> Self {
        let mut merged_stack: [MaybeUninit<Range<u64>>; MAX_RANGE_COUNT] = MaybeUninit::uninit_array();
        let mut size: usize = 0;

        if regions.len() == 1 {
            merged_stack[0].write(regions[0].start..(regions[0].start + regions[0].length));
            size += 1;
        } else {
            let mut curr_idx: isize = 0;

            merged_stack[0].write(regions[0].start..(regions[0].start + regions[0].length));
            size += 1;

            for i in 1..regions.len() {
                let r_start = regions[i].start;
                let r_end_ex = regions[i].start + regions[i].length;

                let peek = &merged_stack[curr_idx as usize];
                let peek = unsafe { peek.assume_init_ref() };

                if peek.end < r_start {
                    curr_idx += 1;
                    merged_stack[curr_idx as usize].write(r_start..r_end_ex);
                    size += 1;
                } else if peek.end < r_end_ex {
                    merged_stack[curr_idx as usize].write(peek.start..r_end_ex);
                }
            }
        }

        let mut ranges_ret: [MaybeUninit<Range<u64>>; MAX_RANGE_COUNT] = MaybeUninit::uninit_array();
        let mut curr_idx = 0;

        while curr_idx < size {
            let pop = &merged_stack[curr_idx];
            let pop = unsafe { pop.assume_init_ref() };
            ranges_ret[curr_idx].write(pop.clone());
            curr_idx += 1;
        }

        Self {
            ranges: unsafe { transmute(ranges_ret) },
            current_range_index: 0,
            current_value: start,
            range_size: size,
            window
        }
    }

    /// next windowed value after right boundary.
    /// `neg_offset` should be smaller than `self.window`
    fn next_windowed_after(&self, start: u64, range_right: u64) -> u64 {
        if start >= range_right {
            panic!("start should be smaller than range_right, start = {start}, range_right = {range_right}");
        }

        align_up(range_right - start, self.window) + start
    }
}

pub fn initialize_frame_allocator(mem_regions: &[MemoryRegion]) {
    let mut iter = LinkedRangeIterator::from_memory_regions(0x1000, 0x1000, mem_regions);

}

#[test]
pub(super) fn test_iterator() {
    //let mem_regions = MaybeUninit
}