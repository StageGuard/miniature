#![no_std]

pub mod framebuffer;
pub mod framebuffer_writer;
pub mod print_panic;
pub mod arg;
pub mod uni_processor;

// 内核 bytes 在 kernel pml4 page table 位置
pub const KERNEL_BYTES_P4: u16 = 511;
// bootstrap bytes 在 kernel pml4 page table 位置
pub const BOOTSTRAP_BYTES_P4: u16 = 510;
// 物理地址空间在 kernel pml4 page table 位置
pub const PHYS_MEM_P4: u16 = 0;
// kernel 栈在 kernel pml4 page table 位置
pub const KERNEL_STACK_P4: u16 = 509;
// framebuffer 在 kernel pml4 page table 位置
pub const FRAMEBUFFER_P4: u16 = 508;
pub const KERNEL_ARG_P4: u16 = 507;