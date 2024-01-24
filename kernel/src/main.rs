#![no_main]
#![no_std]

use device::qemu::exit_qemu;
use kernel_arg::KernelArg;

mod panic;
mod kernel_arg;
mod device;


#[no_mangle]
pub extern "C" fn _start(arg: &'static KernelArg) -> ! {
    exit_qemu(device::qemu::QemuExitCode::Success);
}