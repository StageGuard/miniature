use core::{arch::asm, panic::PanicInfo};
use log::error;

#[cfg(not(test))]
#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    error!("PANIC: {:?}", info);

    loop {
        unsafe { asm!("hlt", options(nomem, nostack)); }
    }
}