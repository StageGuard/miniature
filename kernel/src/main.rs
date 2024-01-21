#![no_main]
#![no_std]

mod panic;
use core::arch::global_asm;

global_asm!(include_str!("kernel_entry.asm"));
global_asm!(include_str!("print.asm"));

#[no_mangle]
pub extern "C" fn kernel_entry() -> ! {
    loop {}
}