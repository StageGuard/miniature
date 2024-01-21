use core::fmt;
use log::info;

pub trait PrintPanic<R> {
    fn or_panic(self, msg: &str) -> R;
}

impl <T, E> PrintPanic<T> for Result<T, E>
    where E: fmt::Debug
{
    fn or_panic(self, msg: &str) -> T {
        match self {
            Ok(t) => t,
            Err(e) => {
                info!("efi panicked: {}: {:?}", msg, e);
                panic!("{} {:?}", msg, e);
            },
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