use crate::error::KResult;
use crate::r#macro::syscall3;
use crate::syscall_number::SYS_WRITE;

/// Write a buffer to a fs descriptor
///
/// The kernel will attempt to write the bytes in `buf` to the fs descriptor `fd`, returning
/// either an `Err`, explained below, or `Ok(count)` where `count` is the number of bytes which
/// were written.
///
/// # Errors
///
/// * `EAGAIN` - the fs descriptor was opened with `O_NONBLOCK` and writing would block
/// * `EBADF` - the fs descriptor is not valid or is not open for writing
/// * `EFAULT` - `buf` does not point to the process's addressible memory
/// * `EIO` - an I/O error occurred
/// * `ENOSPC` - the device containing the fs descriptor has no room for data
/// * `EPIPE` - the fs descriptor refers to a pipe or socket whose reading end is closed
pub fn write(fd: usize, buf: &[u8]) -> KResult<usize> {
    unsafe { syscall3(SYS_WRITE, fd, buf.as_ptr() as usize, buf.len()) }
}