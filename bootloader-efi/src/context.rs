use core::{arch::asm, mem::MaybeUninit};

use x86_64::structures::paging::PhysFrame;

use crate::{kernel::TlsTemplate, mem::MemoryRegion};

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

/// # SAFETY
/// we assumes all these address are valid
///
/// `stack_top`, `entry` and `arg` is at kernel pml4 page scope
pub unsafe fn context_switch(
    pml4_table: PhysFrame,
    stack_top: u64,
    entry: u64,
    arg: u64
) -> ! {
    unsafe {
        asm!(
            r#"
            xor rbp, rbp
            mov cr3, {}
            mov rsp, {}
            push 0
            3:
                hlt
                jmp 3b
            jmp {}
            "#,
            in(reg) pml4_table.start_address().as_u64(),
            in(reg) stack_top,
            in(reg) entry,
            in("rdi") arg as usize,
        );
    }
    panic!("unreachable, context switched");
}