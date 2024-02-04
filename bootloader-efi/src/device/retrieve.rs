use core::mem::MaybeUninit;

use log::warn;
use shared::print_panic::PrintPanic;
use uefi::proto::Protocol;
use uefi::{Identify, Handle};
use uefi::proto::device_path::DevicePath;
use uefi::proto::device_path::text::{DevicePathToText, DisplayOnly, AllowShortcuts, PoolString};
use uefi::table::boot::{SearchType, BootServices, ScopedProtocol};

#[derive(Debug)]
pub struct ProtocolWithHandle<'a, P : Protocol> {
    pub handle: Handle,
    pub protocol: ScopedProtocol<'a, P>,
    pub device_path_string: PoolString<'a>
}


pub fn list_handles<'a, P : Protocol>(boot_services: &'a BootServices, out: &mut [MaybeUninit<ProtocolWithHandle<'a, P>>]) -> usize {
    let handle_buffer = boot_services
        .locate_handle_buffer(SearchType::ByProtocol(&P::GUID))
        .or_panic("failed to locate protocol handle buffers");

    let mut idx: usize = 0;
    handle_buffer.iter().for_each(|h| {
        let protocol = boot_services.open_protocol_exclusive::<P>(*h);
        if protocol.is_err() {
            return;
        }

        let device_path = boot_services.open_protocol_exclusive::<DevicePath>(*h);
        if device_path.is_err() {
            warn!("failed to open protocol DevicePath of handle {:?}: {}", h, device_path.unwrap_err());
            return;
        }
        let device_path = device_path.unwrap();
        let device_path_str = get_device_path_str(boot_services, &device_path);

        out[idx] = MaybeUninit::new(ProtocolWithHandle {
            handle: *h, 
            protocol: protocol.unwrap(),
            device_path_string: device_path_str
        });
        idx += 1;
    });
    idx
}

pub fn get_device_path_str<'a>(boot_services: &'a BootServices, device_path: &ScopedProtocol<'a, DevicePath>) -> PoolString<'a> {
    let dptt_handle_buffers = boot_services
        .locate_handle_buffer(SearchType::ByProtocol(&DevicePathToText::GUID))
        .or_panic("failed to locate DevicePathToText handle buffers");
    let dptt_handle = dptt_handle_buffers.first().or_panic("failed to get DevicePathToText handle");

    let dptt_protocol = boot_services
        .open_protocol_exclusive::<DevicePathToText>(*dptt_handle)
        .or_panic("failed to open DevicePathToText of handle.");

    let path_string = dptt_protocol
        .convert_device_path_to_text(boot_services, device_path, DisplayOnly(true), AllowShortcuts(false))
        .or_panic("failed to convert_device_path_to_text");

    path_string
}