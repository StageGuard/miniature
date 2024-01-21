use core::ops::Deref;

use alloc::{vec::Vec, string::ToString};
use log::{warn, info};
use uefi::{proto::{Protocol, media::{fs::SimpleFileSystem, file::{FileMode, FileAttribute, File}}, device_path::DevicePath, loaded_image::{self, LoadedImage}}, table::boot::{BootServices, ScopedProtocol, LoadImageSource, OpenProtocolParams}, cstr16};
use uefi_services::system_table;

use crate::{device::retrieve::ProtocolWithHandle, print_panic::PrintPanic};

use super::retrieve::get_device_path_str;

pub fn find_current_boot_partition<'a, T : Protocol>(
    boot_services: &'a BootServices,
    partitions: &'a Vec<ProtocolWithHandle<T>>
) -> Option<&'a ProtocolWithHandle<'a, T>> {
    let current_image = boot_services.open_protocol_exclusive::<LoadedImage>(boot_services.image_handle());
    if current_image.is_err() {
        warn!("failed to open protocol LoadedImage of current loaded image handle");
        return None
    }
    
    let current_image = current_image.unwrap();
    let current_image_device = current_image.device().or_panic("failed to get device handle of current loaded image");

    let current_image_device_path = unsafe {
        let protocol = boot_services
            .open_protocol_exclusive::<DevicePath>(current_image_device)
            .or_panic("failed to open protocol DevicePath of device of current loaded image");
        get_device_path_str(boot_services, &protocol)
    };

    for part in partitions {
        if part.device_path_string.to_string() == current_image_device_path.to_string() {
            return Some(part)
        }
    }

    None
}