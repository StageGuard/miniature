use shared::arg::MemoryRegionKind;
use x86_64::PhysAddr;

pub mod page_allocator;
pub mod frame_allocator;
pub mod memory_descriptor;
pub mod runtime_map;
pub mod tracked_mapper;

/// Abstraction trait for a memory region returned by the UEFI.
pub trait RTMemoryRegionDescriptor: Copy + core::fmt::Debug {
    /// Returns the physical start address of the region.
    fn start(&self) -> PhysAddr;
    /// Returns the size of the region in bytes.
    fn len(&self) -> u64;
    /// Returns whether this region is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Returns the type of the region, e.g. whether it is usable or reserved.
    fn kind(&self) -> MemoryRegionKind;

    /// Some regions become usable when the bootloader jumps to the kernel.
    fn usable_after_bootloader_exit(&self) -> bool;
}