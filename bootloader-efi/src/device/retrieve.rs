use alloc::string::ToString;
use alloc::vec::Vec;
use log::{info, warn};
use uefi::proto::Protocol;
use uefi::proto::device_path::media::RamDisk;
use uefi::{Identify, Handle};
use uefi::proto::device_path::DevicePath;
use uefi::proto::device_path::text::{DevicePathToText, DisplayOnly, AllowShortcuts, PoolString};
use uefi::proto::media::block::BlockIO;
use uefi::proto::media::partition::PartitionInfo;
use uefi::table::{Boot, SystemTable};
use uefi::table::boot::{SearchType, BootServices, ScopedProtocol};
use crate::print_panic::PrintPanic;

#[derive(Debug)]
pub struct ProtocolWithHandle<'a, P : Protocol> {
    pub handle: Handle,
    pub protocol: ScopedProtocol<'a, P>,
    pub device_path_string: PoolString<'a>
}


pub fn list_handles<P : Protocol>(boot_services: &BootServices) -> Vec<ProtocolWithHandle<P>> {
    let handle_buffer = boot_services
        .locate_handle_buffer(SearchType::ByProtocol(&P::GUID))
        .or_panic("failed to locate protocol handle buffers");

    let mut handles = Vec::with_capacity(handle_buffer.len());

    for h in handle_buffer.iter() {
        let protocol = boot_services.open_protocol_exclusive::<P>(*h);
        if protocol.is_err() {
            continue;
        }

        let device_path = boot_services.open_protocol_exclusive::<DevicePath>(*h);
        if device_path.is_err() {
            warn!("failed to open protocol DevicePath of handle {:?}: {}", h, device_path.unwrap_err());
            continue;
        }
        let device_path = device_path.unwrap();
        let device_path_str = get_device_path_str(boot_services, &device_path);

        handles.push( ProtocolWithHandle {
            handle: *h, 
            protocol: protocol.unwrap(),
            device_path_string: device_path_str
        })
    }

    handles
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