use log::{info, Log, log};
use shared::{framebuffer::Framebuffer, framebuffer_writer::FrameBufferWriter, uni_processor::UPSafeCell};
use spin::Mutex;
use core::{fmt::Write, mem::MaybeUninit};
use lazy_static::lazy_static;

use crate::{device::qemu::exit_qemu, framebuffer::FRAMEBUFFER, qemu_println};
use crate::gdt::pcr;

lazy_static! {
    pub static ref FRAMEBUFFER_LOGGER: UPSafeCell<MaybeUninit<FramebufferLogger<'static>>> = unsafe { UPSafeCell::new(MaybeUninit::uninit()) };
}

pub struct FramebufferLogger<'a> {
    pub writer: Mutex<FrameBufferWriter<'a>>,
}

impl <'a> FramebufferLogger<'a> {
    pub fn new(framebuffer: &'a Framebuffer) -> Self {
        Self {
            writer: Mutex::new(FrameBufferWriter::new(framebuffer))
        }
    }
}

impl log::Log for FramebufferLogger<'_> {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let mut fb_writter = self.writer.lock();
        
        let _ = writeln!(fb_writter, "[{:5}]{}", record.level(), record.args());
    }

    fn flush(&self) {
        
    }
}

#[macro_export]
macro_rules! loghart {
    ($lvl:expr, $($arg:tt)+) => {
        ::log::log!($lvl, concat!("[#{}] {}"), unsafe { (*crate::gdt::pcr()).percpu.cpu_id.0 }, format_args!($($arg)+))
    };
    ($lvl:expr, target: $target:expr, $($arg:tt)+) => {
        ::log::log!(concat!("[#{}] ", target: $target), $lvl, unsafe { (*crate::gdt::pcr()).percpu.cpu_id.0 }, $($arg)+)
    };
}

#[macro_export]
macro_rules! infohart {
    ($($arg:tt)+) => ($crate::loghart!(::log::Level::Info, $($arg)+));
    ($target:expr, $($arg:tt)+) => ($crate::loghart!(::log::Level::Info, $target, $($arg)+));
}

#[macro_export]
macro_rules! warnhart {
    ($($arg:tt)+) => ($crate::loghart!(::log::Level::Warn, $($arg)+));
    ($target:expr, $($arg:tt)+) => ($crate::loghart!(::log::Level::Warn, $target, $($arg)+));
}

#[macro_export]
macro_rules! errorhart {
    ($($arg:tt)+) => ($crate::loghart!(::log::Level::Error, $($arg)+));
    ($target:expr, $($arg:tt)+) => ($crate::loghart!(::log::Level::Error, $target, $($arg)+));
}

pub fn init_framebuffer_logger() {
    let framebuffer = FRAMEBUFFER.inner_exclusive_mut();
    let framebuffer = framebuffer.lock();
    let framebuffer = unsafe { framebuffer.assume_init_ref() };

    let mut logger = FRAMEBUFFER_LOGGER.inner_exclusive_mut();
    let logger_ref = logger.write(
        FramebufferLogger::new(unsafe { &*(framebuffer as *const Framebuffer) })
    );

    if let Err(err) = log::set_logger(unsafe { &*(logger_ref as *const dyn Log) }) {
        qemu_println!("kernel failed to initialize framebuffer logger: {}", err);
        exit_qemu(crate::device::qemu::QemuExitCode::Success);
    };
    log::set_max_level(log::LevelFilter::Debug);

    info!("kernel framebuffer logger is initialized.");
}