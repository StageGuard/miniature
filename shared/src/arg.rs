use core::{fmt::{self, Debug}, mem::MaybeUninit};

pub const MAX_CPUS: usize = 256;

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct MemoryRegion {
    /// The physical start address of the region.
    pub start: u64,
    /// length.
    pub length: u64,
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
            length: 0,
            kind: MemoryRegionKind::Bootloader,
        }
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

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TlsTemplate {
    pub start_virt_addr: u64,
    pub mem_size: usize,
    pub file_size: usize
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KernelArg {
    // kerrnel 的定义虚拟地址空间与实际虚拟地址空间的偏移
    pub kernel_virt_space_offset: i128,

    // GlobalDescriptorTable 起始虚拟地址
    pub gdt_start_addr: u64,

    // 内核四级页表地址
    pub kernel_pml4_start_addr: u64,

    // ACPI 参数
    pub acpi: AcpiSettings,

    // 栈顶起始虚拟地址
    pub stack_top_addr: u64,
    // 栈大小
    pub stack_size: usize,

    // framebuffer 起始虚拟地址
    pub framebuffer_addr: u64,
    // framebuffer 大小
    pub framebuffer_len: usize,
    pub framebuffer_width: usize,
    pub framebuffer_height: usize,
    pub framebuffer_stride: usize,

    // 实际物理地址空间起始虚拟地址
    pub phys_mem_mapped_addr: u64,
    pub phys_mem_size: u64,
    pub unav_phys_mem_regions: [MemoryRegion; 512],
    pub unav_phys_mem_regions_len: usize,

    pub tls_template: TlsTemplate
}


#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AcpiSettings {
    pub local_apic_base: usize,
    pub local_apic: [[u8; 2]; MAX_CPUS],
    pub local_apic_count: usize,
}

impl Default for AcpiSettings {
    fn default() -> Self {
        Self {
            local_apic_base: Default::default(),
            local_apic: [Default::default(); MAX_CPUS],
            local_apic_count: Default::default()
        }
    }
}