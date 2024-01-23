use core::{mem::MaybeUninit, fmt::Write};

use lazy_static::lazy_static;
use log::info;
use spin::mutex::Mutex;
use uefi::table::{SystemTable, Boot};

use crate::framebuffer::Framebuffer;
use crate::logger::writer::FrameBufferWriter;
use crate::sync::upsafe_cell::UPSafeCell;

pub mod writer;

lazy_static! {
    static ref FRAMEBUFFER_LOGGER: UPSafeCell<Mutex<MaybeUninit<FramebufferLogger<'static>>>> = unsafe { UPSafeCell::new(Mutex::new(MaybeUninit::uninit())) };
    static ref UEFI_STDOUT_LOGGER: UPSafeCell<Mutex<MaybeUninit<uefi::logger::Logger>>> = unsafe { UPSafeCell::new(Mutex::new(MaybeUninit::uninit())) };
}

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

pub fn init_framebuffer_logger(framebuffer: &'static Framebuffer) {

    let logger_mutex = FRAMEBUFFER_LOGGER.borrow_mut();
    let mut logger = logger_mutex.lock();
    logger.write(FramebufferLogger::new(framebuffer));

    if let Err(err) = log::set_logger(unsafe { &*logger.as_ptr() }) {
        info!("failed to set global logger: {}", err);
    };
    log::set_max_level(log::LevelFilter::Debug);
}

pub fn init_uefi_services_logger(system_table: &mut SystemTable<Boot>) {
    let logger_mutex: core::cell::RefMut<'_, Mutex<MaybeUninit<uefi::logger::Logger>>> = UEFI_STDOUT_LOGGER.borrow_mut();
    let mut logger = logger_mutex.lock();

    let uefi_logger = uefi::logger::Logger::new();
    unsafe { uefi_logger.set_output(system_table.stdout()); };

    let _ = log::set_logger(unsafe { &*(&uefi_logger as *const _) });
    log::set_max_level(log::LevelFilter::Debug);

    logger.write(uefi_logger);
}