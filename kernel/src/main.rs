#![no_main]
#![no_std]

use core::{arch::{self, asm}, mem::MaybeUninit, slice};

use alloc::vec::Vec;
use device::qemu::exit_qemu;
use lazy_static::lazy_static;
use log::{info, Log};
use shared::{arg::KernelArg, framebuffer::{FBPixelFormat, Framebuffer}, uni_processor::UPSafeCell};
use spin::Mutex;

use crate::logger::FramebufferLogger;

extern crate alloc;

mod panic;
mod device;
mod mem;
mod logger;

lazy_static! {
    static ref FRAMEBUFFER: UPSafeCell<Mutex<MaybeUninit<Framebuffer>>> = unsafe { UPSafeCell::new(Mutex::new(MaybeUninit::uninit())) };
    static ref FRAMEBUFFER_LOGGER: UPSafeCell<MaybeUninit<FramebufferLogger<'static>>> = unsafe { UPSafeCell::new(MaybeUninit::uninit()) };
}

#[no_mangle]
pub extern "C" fn _start(arg: &'static KernelArg) -> ! {
    unsafe {
        (arg.framebuffer_addr as *mut u8).add(512).write_volatile(0);
        exit_qemu(device::qemu::QemuExitCode::Success)
    }
    {
        // initialize framebuffer
        let framebuffer_mutex = FRAMEBUFFER.inner_exclusive_mut();
        let mut framebuffer = framebuffer_mutex.lock();
        
        framebuffer.write(Framebuffer::new(
            arg.framebuffer_addr as *mut u8, 
            arg.framebuffer_len, 
            arg.framebuffer_width, 
            arg.framebuffer_height, 
            arg.framebuffer_stride, 
            FBPixelFormat::RGB
        ));

        let mut logger = FRAMEBUFFER_LOGGER.inner_exclusive_mut();
        let logger_ref = logger.write(
            FramebufferLogger::new(unsafe { &*(&framebuffer.assume_init() as *const Framebuffer) })
        );

        if let Err(err) = log::set_logger(unsafe { &*(logger_ref as *const dyn Log) }) {
            
        };
        log::set_max_level(log::LevelFilter::Debug);

    }
    

    info!("kernel framebuffer logger is initialized.");
    halt();


}

fn halt() -> ! {
    loop {
        unsafe { asm!("hlt") }
    }
}