#![no_main]
#![no_std]
#![feature(maybe_uninit_uninit_array)]

use core::{arch::{self, asm}, fmt::Write, mem::MaybeUninit, slice};

use alloc::vec::Vec;
use device::qemu::exit_qemu;
use lazy_static::lazy_static;
use log::{info, Log};
use mem::frame_allocator::initialize_frame_allocator;
use shared::{arg::KernelArg, framebuffer::{FBPixelFormat, Framebuffer}, uni_processor::UPSafeCell};
use spin::mutex::Mutex;

use crate::{framebuffer::{initialize_framebuffer, FRAMEBUFFER}, logger::{initialize_framebuffer_logger, FramebufferLogger}};

extern crate alloc;

mod panic;
mod device;
mod mem;
mod logger;
mod framebuffer;

#[no_mangle]
pub extern "C" fn _start(arg: &'static KernelArg) -> ! {
    initialize_framebuffer(arg);
    {
        let framebuffer = FRAMEBUFFER.inner_exclusive_mut();
        let framebuffer = framebuffer.lock();
        let framebuffer = unsafe { framebuffer.assume_init_ref() };
        initialize_framebuffer_logger(unsafe { &*(framebuffer as *const Framebuffer) });
        info!("kernel framebuffer logger is initialized.");
    }
    initialize_frame_allocator(&arg.unav_phys_mem_regions[..arg.unav_phys_mem_regions_len]);

    halt();
}

fn halt() -> ! {
    loop {
        unsafe { asm!("hlt") }
    }
}