use libvdso::error::KResult;
use crate::mem::user_buffer::UserBuffer;

pub trait File: Send + Sync {
    fn readable(&self) -> bool;
    fn writable(&self) -> bool;
    fn read(&self, buf: UserBuffer) -> KResult<()>;
    fn write(&self, buf: UserBuffer) -> Result<usize, isize>;
    //fn awrite(&self, buf: UserBuffer, pid: usize, key: usize) -> Pin<Box<dyn Future<Output = ()> + 'static + Send + Sync>>;
    //fn aread(&self, buf: UserBuffer, cid: usize, pid: usize, key: usize) -> Pin<Box<dyn Future<Output = ()> + 'static + Send + Sync>>;
}