use core::cell::{RefCell, RefMut};


pub struct UPSafeCell<T> {
    inner: RefCell<T>,
}

impl<T> UPSafeCell<T> {
    pub unsafe fn new(value: T) -> Self {
        Self {
            inner: RefCell::new(value),
        }
    }
    pub fn borrow_mut(&self) -> RefMut<'_, T> {
        self.inner.borrow_mut()
    }
}

unsafe impl<T> Sync for UPSafeCell<T> {}
unsafe impl<T> Send for UPSafeCell<T> where T: Send {}