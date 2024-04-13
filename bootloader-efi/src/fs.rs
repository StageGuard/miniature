
use core::ops::Deref;
use core::ptr;

use core::slice;
use log::{warn, error};
use crate::mem::page_allocator::boot::{paging_allocate, allocate_zeroed_page_aligned};
use uefi::proto::device_path::DevicePath;
use uefi::{prelude::*, CStr16};
use uefi::proto::media::file::{File, FileMode, FileAttribute, Directory, FileInfo, FileType};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::table::boot::ScopedProtocol;

pub fn open_sfs(boot_services: &BootServices, device_handle: Handle) -> Option<ScopedProtocol<'_, SimpleFileSystem>> {
    let device_path = boot_services.open_protocol_exclusive::<DevicePath>(device_handle);
    if device_path.is_err() {
        error!("failed to open protocol DevicePath, {:?}", device_path.unwrap_err());
        return None;
    }

    let device_path = device_path.unwrap();
    let fs_handle = boot_services.locate_device_path::<SimpleFileSystem>(&mut device_path.deref());
    if fs_handle.is_err() {
        error!("Failed to open protocol SimpleFileSystem {:?}", fs_handle.unwrap_err());
        return None;
    }

    let fs_handle = fs_handle.unwrap();

    let opened_handle = boot_services.open_protocol_exclusive::<SimpleFileSystem>(fs_handle);

    if opened_handle.is_err() {
        error!("Failed to open protocol SimpleFileSystem, {:?}", opened_handle.unwrap_err());
        return None;
    }
    Some(opened_handle.unwrap())
}


pub fn load_file_sfs(
    system_table: &SystemTable<Boot>,
    root: &mut Directory,
    path: &str
) -> Option<&'static mut [u8]> {
    let mut buf = [0u16; 256];
    let filename = match CStr16::from_str_with_buf(path.trim_end_matches('\0'), &mut buf) {
        Err(e) => {
            warn!("cannot convert filename to cstr16: {}", e);
            return None
        },
        Ok(r) => r,
    };
    
    let file_handle = match root.open(filename, FileMode::Read, FileAttribute::empty()) {
        Err(e) => {
            warn!("cannot open fs {}: {}", &*filename, e);
            return None
        },
        Ok(handle) => handle,
    };

    let mut file = match file_handle.into_type().unwrap() {
        FileType::Regular(f) => f,
        FileType::Dir(_) => {
            warn!("open fs {} which is directory", &*filename);
            return None
        },
    };

    let mut buf = unsafe { paging_allocate::<u8>(&system_table).unwrap() };
    let file_info: &mut FileInfo = file.get_info(&mut buf).unwrap();
    let file_size = usize::try_from(file_info.file_size()).unwrap();

    let file_ptr = allocate_zeroed_page_aligned(&system_table, file_size);
    unsafe { ptr::write_bytes(file_ptr, 0, file_size) };

    let file_slice = unsafe { slice::from_raw_parts_mut(file_ptr, file_size) };
    file.read(file_slice).unwrap();

    Some(file_slice)
}