use core::mem::MaybeUninit;

use x86_64::PhysAddr;

pub mod page_allocator;
pub mod frame_allocator;
pub mod memory_descriptor;
pub mod runtime_map;
pub mod tracked_mapper;

pub struct MemoryRegion {
    /// The physical start address of the region.
    pub start: u64,
    /// The physical end address (exclusive) of the region.
    pub end: u64,
    /// The memory type of the memory region.
    ///
    /// Only [`Usable`][MemoryRegionKind::Usable] regions can be freely used.
    pub kind: MemoryRegionKind,
}

impl MemoryRegion {
    /// Creates a new empty memory region (with length 0).
    pub const fn empty() -> Self {
        MemoryRegion {
            start: 0,
            end: 0,
            kind: MemoryRegionKind::Bootloader,
        }
    }

    pub fn add_region(
        region: Self,
        regions: &mut [MaybeUninit<Self>],
        next_index: &mut usize,
    ) {
        if region.start == region.end {
            // skip zero sized regions
            return;
        }
        unsafe {
            regions
                .get_mut(*next_index)
                .expect("cannot add region: no more free entries in memory map")
                .as_mut_ptr()
                .write(region)
        };
        *next_index += 1;
    }
}

/// Represents the different types of memory.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[non_exhaustive]
#[repr(C)]
pub enum MemoryRegionKind {
    /// Unused conventional memory, can be used by the kernel.
    Usable,
    /// Memory mappings created by the bootloader, including the page table and boot info mappings.
    ///
    /// This memory should _not_ be used by the kernel.
    Bootloader,
    /// An unknown memory region reported by the UEFI firmware.
    ///
    /// Contains the UEFI memory type tag.
    UnknownUefi(u32),
    /// An unknown memory region reported by the BIOS firmware.
    UnknownBios(u32),
}

/// Abstraction trait for a memory region returned by the UEFI.
pub trait RTMemoryRegion: Copy + core::fmt::Debug {
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