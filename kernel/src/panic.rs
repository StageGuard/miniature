use core::panic::PanicInfo;

#[cfg(not(test))]
#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    loop {

    }
}