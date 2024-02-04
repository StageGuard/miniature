use core::{fmt, slice};

use crate::{framebuffer::{Framebuffer, FBPixelFormat}, print_panic::PrintPanic};
use noto_sans_mono_bitmap::{
    get_raster, get_raster_width, FontWeight, RasterHeight, RasterizedChar,
};
use core::ptr;

const LINE_SPACING: usize = 2;
const LETTER_SPACING: usize = 0;

const BORDER_PADDING: usize = 1;


pub struct FrameBufferWriter<'a> {
    framebuffer: &'a Framebuffer,
    buffer_slice: &'a mut [u8],

    curr_x_pos: usize,
    curr_y_pos: usize,
    
}

impl <'a> FrameBufferWriter<'a> {
    /// Creates a new logger that uses the given framebuffer.
    pub fn new(framebuffer: &'a Framebuffer) -> Self {
        let mut writer = Self {
            framebuffer,
            buffer_slice: unsafe { slice::from_raw_parts_mut(framebuffer.ptr, framebuffer.len) },
            curr_x_pos: 0,
            curr_y_pos: 0,
        };
        writer.clear();
        writer
    }

    fn newline(&mut self) {
        self.curr_y_pos += RasterHeight::Size16.val() + LINE_SPACING;
        self.carriage_return()
    }

    fn carriage_return(&mut self) {
        self.curr_x_pos = BORDER_PADDING;
    }

    pub fn clear(&mut self) {
        self.curr_x_pos = BORDER_PADDING;
        self.curr_y_pos = BORDER_PADDING;
        self.buffer_slice.fill(0);
    }

    fn write_char(&mut self, c: char) {
        match c {
            '\n' => self.newline(),
            '\r' => self.carriage_return(),
            c => {
                let new_xpos = self.curr_x_pos +  get_raster_width(FontWeight::Regular, RasterHeight::Size16);
                if new_xpos >= self.framebuffer.width {
                    self.newline();
                }
                let new_ypos = self.curr_y_pos + RasterHeight::Size16.val() + BORDER_PADDING;
                if new_ypos >= self.framebuffer.height {
                    self.clear();
                }
                self.write_rendered_char(get_raser_or_fallback(c));
            }
        }
    }


    fn write_rendered_char(&mut self, rendered_char: RasterizedChar) {
        for (y, row) in rendered_char.raster().iter().enumerate() {
            for (x, byte) in row.iter().enumerate() {
                self.write_pixel(self.curr_x_pos + x, self.curr_y_pos + y, *byte);
            }
        }
        self.curr_x_pos += rendered_char.width() + LETTER_SPACING;
    }

    fn write_pixel(&mut self, x: usize, y: usize, intensity: u8) {
        let pixel_offset = y * self.framebuffer.stride + x;
        let color = match self.framebuffer.pixel_format {
            FBPixelFormat::RGB => [intensity, intensity, intensity / 2, 0],
            FBPixelFormat::BGR => [intensity / 2, intensity, intensity, 0],
            other => {
                panic!("pixel format {:?} not supported in logger", other)
            }
        };
        let bytes_per_pixel = 4;
        let byte_offset = pixel_offset * bytes_per_pixel;
        self.buffer_slice[byte_offset..(byte_offset + bytes_per_pixel)]
            .copy_from_slice(&color[..bytes_per_pixel]);
        let _ = unsafe { ptr::read_volatile(&self.buffer_slice[byte_offset]) };
    }
}

fn get_raser_or_fallback(c: char) -> RasterizedChar {
    get_raster(c, FontWeight::Regular, RasterHeight::Size16)
        .unwrap_or_else(|| get_raster('\u{FFFD}', FontWeight::Regular, RasterHeight::Size16)
            .or_panic("failed to get char and its fallback")
        )
}

unsafe impl Send for FrameBufferWriter<'_> {}
unsafe impl Sync for FrameBufferWriter<'_> {}

impl fmt::Write for FrameBufferWriter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            self.write_char(c);
        }
        Ok(())
    }
}
