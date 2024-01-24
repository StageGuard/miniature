#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MemoryRegion {
    pub start: u64,
    pub end: u64,
    pub kind: MemoryRegionKind,
}

#[repr(C)]
#[non_exhaustive]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum MemoryRegionKind {
    Usable,
    Bootloader,
    UnknownUefi(u32),
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