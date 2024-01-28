#![no_main]
#![no_std]

use core::{borrow::{Borrow, BorrowMut}, mem::MaybeUninit};

use device::qemu::exit_qemu;
use kernel_arg::{Framebuffer, KernelArg};
use lazy_static::lazy_static;
use spin::Mutex;
use sync::upsafe_cell::UPSafeCell;

mod panic;
mod kernel_arg;
mod device;
mod sync;

lazy_static! {
    static ref FRAMEBUFFER: UPSafeCell<Mutex<MaybeUninit<Framebuffer>>> = unsafe { UPSafeCell::new(Mutex::new(MaybeUninit::uninit())) };
}

#[no_mangle]
pub extern "C" fn _start(arg: &'static KernelArg) -> ! {

    let framebuffer = FRAMEBUFFER.borrow();

    


    exit_qemu(device::qemu::QemuExitCode::Success);
}