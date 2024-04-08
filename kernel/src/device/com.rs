use lazy_static::lazy_static;
use spin::Mutex;
use uart_16550::SerialPort;

lazy_static! {
    pub static ref COM1: Mutex<SerialPort> = unsafe { Mutex::new(SerialPort::new(0x3F8)) };
    pub static ref COM2: Mutex<SerialPort> = unsafe { Mutex::new(SerialPort::new(0x2F8)) };
}

pub unsafe fn init_com() {
    COM1.lock().init();
    COM2.lock().init();
}