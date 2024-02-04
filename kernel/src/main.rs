#![no_main]
#![no_std]

use core::{arch::{self, asm}, fmt::Write, mem::MaybeUninit, slice};

use alloc::vec::Vec;
use device::qemu::exit_qemu;
use lazy_static::lazy_static;
use log::{info, Log};
use shared::{arg::KernelArg, framebuffer::{FBPixelFormat, Framebuffer}, uni_processor::UPSafeCell};
use spin::mutex::Mutex;

use crate::logger::{initialize_framebuffer_logger, FramebufferLogger};

extern crate alloc;

mod panic;
mod device;
mod mem;
mod logger;

lazy_static! {
    static ref FRAMEBUFFER: UPSafeCell<Mutex<MaybeUninit<Framebuffer>>> = unsafe { UPSafeCell::new(Mutex::new(MaybeUninit::uninit())) };
}

#[no_mangle]
pub extern "C" fn _start(arg: &'static KernelArg) -> ! {

    {
        // initialize framebuffer
        let framebuffer_mutex = FRAMEBUFFER.inner_exclusive_mut();
        let mut framebuffer = framebuffer_mutex.lock();
        
        let fb_ref = framebuffer.write(Framebuffer::new(
            arg.framebuffer_addr as *mut u8, 
            arg.framebuffer_len, 
            arg.framebuffer_width, 
            arg.framebuffer_height, 
            arg.framebuffer_stride, 
            FBPixelFormat::RGB
        ));

        initialize_framebuffer_logger(unsafe { &*(fb_ref as *const Framebuffer) });
        info!("kernel framebuffer logger is initialized.");
    }
    

    halt();
}

fn halt() -> ! {
    loop {
        unsafe { asm!("hlt") }
    }
}