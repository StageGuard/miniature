use uefi::table::boot::{MemoryDescriptor, MemoryType};
use x86_64::PhysAddr;

use crate::mem::{RTMemoryRegionDescriptor, MemoryRegionKind};

const PAGE_SIZE: u64 = 4096;

impl RTMemoryRegionDescriptor for MemoryDescriptor {
    fn start(&self) -> PhysAddr {
        PhysAddr::new(self.phys_start)
    }

    fn len(&self) -> u64 {
        self.page_count * PAGE_SIZE 
    }

    fn kind(&self) -> MemoryRegionKind {
        match self.ty {
            MemoryType::CONVENTIONAL => MemoryRegionKind::Usable,
            other => MemoryRegionKind::UnknownUefi(other.0),
        }
    }

    fn usable_after_bootloader_exit(&self) -> bool {
        match self.ty {
            MemoryType::CONVENTIONAL => true,
            | MemoryType::BOOT_SERVICES_CODE
            | MemoryType::BOOT_SERVICES_DATA => {
                // we don't need this data anymore after the bootloader
                // passes control to the kernel
                true
            }
            MemoryType::RUNTIME_SERVICES_CODE | MemoryType::RUNTIME_SERVICES_DATA => {
                // the UEFI standard specifies that these should be presevered
                // by the bootloader and operating system
                false
            }
            _ => false,
        }
    }
}
