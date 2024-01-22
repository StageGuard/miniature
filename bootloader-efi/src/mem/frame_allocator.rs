use log::{info, debug};
use uefi::table::boot::{MemoryDescriptor, MemoryMapIter};
use x86_64::{
    structures::paging::{FrameAllocator, PhysFrame, Size4KiB},
    PhysAddr,
};

use super::{RTMemoryRegion, MemoryRegionKind};



/// A physical frame allocator based onUEFI provided memory map.
pub struct RTFrameAllocator<'a> {
    original: MemoryMapIter<'a>,
    memory_map: MemoryMapIter<'a>,
    current_descriptor: Option<MemoryDescriptor>,
    next_frame: PhysFrame,
}

impl <'a> RTFrameAllocator<'a> {
    /// Creates a new frame allocator based on the given memory regions.
    ///
    /// Skips the frame at physical address zero to avoid potential problems. For example
    /// identity-mapping the frame at address zero is not valid in Rust, because Rust's `core`
    /// library assumes that references can never point to virtual address `0`.  
    pub fn new(memory_map: MemoryMapIter<'a>) -> Self {
        // skip frame 0 because the rust core library does not see 0 as a valid address
        let start_frame = PhysFrame::containing_address(PhysAddr::new(0x1000));
        Self::new_starting_at(start_frame, memory_map)
    }

    /// Creates a new frame allocator based on the given legacy memory regions. Skips any frames
    /// before the given `frame`.
    pub fn new_starting_at(frame: PhysFrame, memory_map: MemoryMapIter<'a>) -> Self {
        Self {
            original: memory_map.clone(),
            memory_map,
            current_descriptor: None,
            next_frame: frame,
        }
    }

    fn allocate_frame_from_descriptor(&mut self, descriptor: MemoryDescriptor) -> Option<PhysFrame> {
        let start_addr = descriptor.start();
        let start_frame = PhysFrame::containing_address(start_addr);
        let end_addr = start_addr + descriptor.len();
        let end_frame = PhysFrame::containing_address(end_addr - 1u64);

        // increase self.next_frame to start_frame if smaller
        if self.next_frame < start_frame {
            self.next_frame = start_frame;
        }

        if self.next_frame <= end_frame {
            let ret = self.next_frame;
            self.next_frame += 1;
            Some(ret)
        } else {
            None
        }
    }

    /// Returns the number of memory regions in the underlying memory map.
    ///
    /// The function always returns the same value, i.e. the length doesn't
    /// change after calls to `allocate_frame`.
    pub fn len(&self) -> usize {
        self.original.len()
    }

    /// Returns whether this memory map is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the largest detected physical memory address.
    ///
    /// Useful for creating a mapping for all physical memory.
    pub fn max_phys_addr(&self) -> PhysAddr {
        self.original
            .clone()
            .map(|r| r.start() + r.len())
            .max()
            .unwrap()
    }
}

unsafe impl FrameAllocator<Size4KiB> for RTFrameAllocator<'_> {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        if let Some(current_descriptor) = self.current_descriptor {
            match self.allocate_frame_from_descriptor(current_descriptor) {
                Some(frame) => return Some(frame),
                None => {
                    self.current_descriptor = None;
                }
            }
        }

        while let Some(descriptor) = self.memory_map.next() {
            if descriptor.kind() != MemoryRegionKind::Usable {
                continue;
            }
            if let Some(frame) = self.allocate_frame_from_descriptor(*descriptor) {
                self.current_descriptor = Some(*descriptor);
                return Some(frame);
            }
        }

        None
    }
}
