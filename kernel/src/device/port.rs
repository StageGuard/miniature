use x86_64::instructions::port::Port;

#[inline]
pub unsafe fn inb(port: u16) -> u8 {
    Port::new(port).read()
}

#[inline]
pub unsafe fn outb(port: u16, value: u8) {
    Port::new(port).write(value)
}