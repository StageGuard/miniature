use lazy_static::lazy_static;
use spin::Mutex;
use uart_16550::SerialPort;

lazy_static! {
    pub static ref STDIO_PORT: Mutex<SerialPort> = unsafe { 
        let mut port = SerialPort::new(0x3F8);
        port.init();
        Mutex::new(port)
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

pub fn exit_qemu(exit_code: QemuExitCode) -> ! {
    use x86_64::instructions::{nop, port::Port};

    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }

    loop {
        nop();
    }
}

#[macro_export]
macro_rules! qemu_print {
    ($fmt: literal $(, $($arg: tt)+)?) => {{
        $crate::device::qemu::STDIO_PORT.lock().write_fmt(format_args!($fmt $(, $($arg)+)?));
    }};
}

#[macro_export]
macro_rules! qemu_println {
    ($fmt: literal $(, $($arg: tt)+)?) => {{
        use ::core::fmt::Write;
        $crate::device::qemu::STDIO_PORT.lock().write_fmt(format_args!(concat!($fmt, "\n") $(, $($arg)+)?));
    }};
}