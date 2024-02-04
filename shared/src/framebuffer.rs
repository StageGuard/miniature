use core::slice;
use bitflags::bitflags;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct Framebuffer {
    pub ptr: *mut u8,
    pub len: usize,

    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub pixel_format: FBPixelFormat,
}

unsafe impl Sync for Framebuffer {}
unsafe impl Send for Framebuffer {}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct FBPixelFormat: u32 {
        const RGB = 1 << 0;
        const BGR = 1 << 1;
    }
}

impl Framebuffer {
    pub fn new(ptr: *mut u8, len: usize, width: usize, height: usize, stride: usize, pixel_format: FBPixelFormat) -> Self {
        Self { ptr, len, width, height, stride, pixel_format }
    }

    pub fn slice(&self) -> &'static mut [u8] {
        // SAFETY: we ensure that `self.ptr` is the pointer points to the framebuffer u8 array
        unsafe { slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}