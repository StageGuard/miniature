use core::arch::asm;

use x86_64::structures::paging::PhysFrame;
use x86_64::VirtAddr;

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