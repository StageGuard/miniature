#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;
use libvdso::syscall;

#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    syscall::write(1, b"panic");
    halt();
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    syscall::write(1, b"hello from bootstrap\n");
    halt();
}

#[inline]
fn halt() -> ! {
    loop {
        unsafe { asm!("hlt;") }
    }
}