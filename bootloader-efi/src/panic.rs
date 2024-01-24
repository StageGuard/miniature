use core::{fmt, sync::atomic::AtomicPtr, ffi::c_void, ptr, panic::PanicInfo, arch::asm};
use lazy_static::lazy_static;
use log::{info, error};

lazy_static! {
    static ref SYSTEM_TABLE_BOOT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
}

#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    error!("PANIC: {:?}", info);

    loop {
        unsafe { asm!("hlt", options(nomem, nostack)); }
    }
}

pub trait PrintPanic<R> {
    fn or_panic(self, msg: &str) -> R;
}

impl <T, E> PrintPanic<T> for Result<T, E>
    where E: fmt::Debug
{
    fn or_panic(self, msg: &str) -> T {
        match self {
            Ok(t) => t,
            Err(e) => panic!("efi panicked: {}: {:?}", msg, e) ,
        }
    }
}

impl <T> PrintPanic<T> for Option<T> {
    fn or_panic(self, msg: &str) -> T {
        match self {
            Some(t) => t,
            None => {
                info!("efi panicked: {}", msg);
                panic!("{}", msg);
            },
        }
    }
}