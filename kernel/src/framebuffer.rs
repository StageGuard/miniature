use core::mem::MaybeUninit;

use lazy_static::lazy_static;
use shared::{arg::KernelArg, framebuffer::{FBPixelFormat, Framebuffer}, uni_processor::UPSafeCell};
use spin::mutex::Mutex;


lazy_static! {
    pub static ref FRAMEBUFFER: UPSafeCell<Mutex<MaybeUninit<Framebuffer>>> = unsafe { UPSafeCell::new(Mutex::new(MaybeUninit::uninit())) };
}

pub fn init_framebuffer(kernel_arg: &KernelArg) {
    // initialize framebuffer
    let framebuffer_mutex = FRAMEBUFFER.inner_exclusive_mut();
    let mut framebuffer = framebuffer_mutex.lock();
    
    framebuffer.write(Framebuffer::new(
        kernel_arg.framebuffer_addr as *mut u8, 
        kernel_arg.framebuffer_len, 
        kernel_arg.framebuffer_width, 
        kernel_arg.framebuffer_height, 
        kernel_arg.framebuffer_stride, 
        FBPixelFormat::RGB
    ));
}