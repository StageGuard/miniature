use core::panic::PanicInfo;
use core::fmt::Write;

#[cfg(not(test))]
#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    loop {

    }
}

#[cfg(test)]
#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    use crate::{device::qemu::exit_qemu, qemu_println};

    qemu_println!("KERNEL TEST FAILED...{:?}", info);
    exit_qemu(crate::device::qemu::QemuExitCode::Failed)
}