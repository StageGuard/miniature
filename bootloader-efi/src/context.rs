use core::arch::asm;

use x86_64::VirtAddr;

use crate::kernel::TlsTemplate;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KernelArg {
    pub kernel_virt_space_offset: i128,

    pub kernel_start_addr: u64,

    pub gdt_start_addr: u64,

    pub stack_top_addr: u64,
    pub stack_size: usize,

    pub framebuffer_addr: u64,
    pub framebuffer_len: usize,
    pub framebuffer_width: usize,
    pub framebuffer_height: usize,
    pub framebuffer_stride: usize,

    pub phys_mem_mapped_addr: u64,
    pub phys_mem_size: u64,

    pub tls_template: TlsTemplate
}

/// # SAFETY
/// we assumes all these address are valid
/// 
/// the `pml4_table` is the virt addr at boot stage pml4 page scope
///
/// `stack_top`, `entry` and `arg` is at kernel pml4 page scope
pub unsafe fn context_switch(
    pml4_table: u64,
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
            jmp {}
            "#,
            in(reg) pml4_table,
            in(reg) stack_top,
            in(reg) entry,
            in("rdi") arg as usize,
        );
    }
    panic!("unreachable, context switched");
}