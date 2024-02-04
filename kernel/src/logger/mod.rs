use shared::{framebuffer::Framebuffer, framebuffer_writer::FrameBufferWriter};
use spin::Mutex;
use core::{fmt::Write, slice};

use crate::device::qemu::exit_qemu;

pub struct FramebufferLogger<'a> {
    writter: Mutex<FrameBufferWriter<'a>>,
}

impl <'a> FramebufferLogger<'a> {
    pub fn new(framebuffer: &'a Framebuffer) -> Self {
        Self {
            writter: Mutex::new(FrameBufferWriter::new(framebuffer))
        }
    }
}

impl log::Log for FramebufferLogger<'_> {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let mut fb_writter = self.writter.lock();
        
        let _ = writeln!(fb_writter, "{:5}: {}", record.level(), record.args());
    }

    fn flush(&self) {
        
    }
}