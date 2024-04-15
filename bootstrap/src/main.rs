#![no_std]
#![no_main]

use core::arch::asm;
use core::hint::spin_loop;
use core::panic::PanicInfo;
use libvdso::syscall;

#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    let _ = syscall::write(1, b"panic");
    loop {
        spin_loop()
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let _ = syscall::write(1, b"hello from bootstrap\n");
    loop {
        spin_loop()
    }
}