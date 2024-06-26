use core::{fmt, ptr::NonNull};
use shared::print_panic::PrintPanic;

/// A small wrapper around NonNull<T> that is Send + Sync, which is
/// only correct if the pointer is never accessed from multiple
/// locations across threads. Which is always, if the pointer is
/// unique.
pub struct Unique<T: ?Sized>(NonNull<T>);

impl<T: ?Sized> Copy for Unique<T> {}
impl<T: ?Sized> Clone for Unique<T> {
    fn clone(&self) -> Self {
        *self
    }
}
unsafe impl<T: ?Sized> Send for Unique<T> {}
unsafe impl<T: ?Sized> Sync for Unique<T> {}

impl<T: ?Sized> Unique<T> {
    pub fn new(ptr: *mut T) -> Self {
        Self(NonNull::new(ptr).or_panic("Did not expect pointer to be null"))
    }
    pub unsafe fn new_unchecked(ptr: *mut T) -> Self {
        Self(NonNull::new_unchecked(ptr))
    }
    pub fn as_ptr(self) -> *mut T {
        self.0.as_ptr()
    }
}
impl<T: ?Sized> fmt::Debug for Unique<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}
