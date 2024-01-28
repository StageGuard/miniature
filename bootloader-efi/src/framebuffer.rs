use core::slice;

use bitflags::bitflags;
use uefi::{table::{SystemTable, Boot, boot::SearchType}, proto::console::gop::{GraphicsOutput, PixelFormat}, Identify};

use crate::panic::PrintPanic;


#[derive(Copy, Clone, Debug)]
pub struct Framebuffer {
    pub ptr: *mut u8,
    pub len: usize,

    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub pixel_format: FBPixelFormat,
}

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

pub fn locate_framebuffer(system_table: &SystemTable<Boot>) -> Option<Framebuffer> {
    let boot_services = system_table.boot_services();

    let graphics_output_handle_buffer = match boot_services
        .locate_handle_buffer(SearchType::ByProtocol(&GraphicsOutput::GUID))
    {
        Ok(handle_buffer) => handle_buffer,
        Err(e) => {
            return None
        }
    };

    let graphics_output_handle = match graphics_output_handle_buffer.first() {
        Some(handle) => *handle,
        None => {
            return None;
        },
    };

    let mut protocol = match boot_services.open_protocol_exclusive::<GraphicsOutput>(graphics_output_handle) {
        Ok(p) => p,
        Err(e) => {
            return None
        }
    };

    let largest_resolution_mode = protocol
        .modes(boot_services)
        .filter(|mode| {
            let (width, height) = mode.info().resolution();
            width <= 1600 && height <= 900 
        })
        .max_by(|a, b| {
            let (a_width, a_height) = a.info().resolution();
            let (b_width, b_height) = b.info().resolution();

            (a_width * a_height).cmp(&(b_width * b_height))
        });
        
    if let Some(mode) = largest_resolution_mode {
        protocol.set_mode(&mode)
            .or_panic("failed to set graphics output mode");
    }

    let current_info = protocol.current_mode_info();
    let mut framebuffer = protocol.frame_buffer();

    Some(Framebuffer::new(
        framebuffer.as_mut_ptr(), 
        framebuffer.size(), 
        current_info.resolution().0, 
        current_info.resolution().1, 
        current_info.stride(), 
        match current_info.pixel_format() {
            PixelFormat::Rgb => FBPixelFormat::RGB,
            PixelFormat::Bgr => FBPixelFormat::BGR,
            others => return None
        }
    ))
}