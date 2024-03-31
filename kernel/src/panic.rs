use core::panic::PanicInfo;
use crate::errorhart;


#[cfg(not(test))]
#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    use log::error;
    use crate::halt;

    errorhart!("kernel panic: {:?}", info);
    loop {
        halt();
    }
}

#[cfg(test)]
#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    use crate::{device::qemu::exit_qemu, qemu_println};

    qemu_println!("KERNEL TEST FAILED...{:?}", info);
    exit_qemu(crate::device::qemu::QemuExitCode::Failed)
}