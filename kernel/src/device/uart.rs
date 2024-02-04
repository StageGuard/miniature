use core::fmt::{self, Arguments};

pub struct SerialPort {
    port: uart_16550::SerialPort,
}

impl SerialPort {
    /// # Safety
    ///
    /// unsafe because this function must only be called once
    pub unsafe fn init(base: u16) -> Self {
        let mut port = uart_16550::SerialPort::new(base);
        port.init();
        Self { port }
    }
}

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.port.write_str(s).unwrap();
        Ok(())
    }
    
    fn write_fmt(&mut self, args: Arguments) -> fmt::Result {
        self.port.write_fmt(args);
        Ok(())
    }
}