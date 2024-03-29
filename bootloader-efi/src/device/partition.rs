use core::mem::MaybeUninit;

use log::warn;
use shared::print_panic::PrintPanic;
use uefi::{proto::{Protocol, device_path::DevicePath, loaded_image::LoadedImage}, table::boot::BootServices};

use crate::device::retrieve::ProtocolWithHandle;

use super::retrieve::get_device_path_str;

pub fn find_current_boot_partition<'a, T : Protocol>(
    boot_services: &'a BootServices,
    partitions: &'a [MaybeUninit<ProtocolWithHandle<T>>]
) -> Option<&'a ProtocolWithHandle<'a, T>> {
    let current_image = boot_services.open_protocol_exclusive::<LoadedImage>(boot_services.image_handle());
    if current_image.is_err() {
        warn!("failed to open protocol LoadedImage of current loaded image handle");
        return None
    }
    
    let current_image = current_image.unwrap();
    let current_image_device = current_image.device().or_panic("failed to get device handle of current loaded image");

    let current_image_device_path = {
        let protocol = boot_services
            .open_protocol_exclusive::<DevicePath>(current_image_device)
            .or_panic("failed to open protocol DevicePath of device of current loaded image");
        get_device_path_str(boot_services, &protocol)
    };

    for part in partitions {
        let part = part.as_ptr();
        unsafe {
            // SAFETY: make sure all entries of `partitions` is initialized.
            if (*part).device_path_string.as_bytes() == current_image_device_path.as_bytes() {
                return Some(&*part)
            }
        }
    }

    None
}