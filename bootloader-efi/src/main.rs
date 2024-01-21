#![no_std]
#![no_main]

mod print_panic;
mod acpi;
mod fs;
mod kernel;
mod framebuffer;
mod logger;
mod sync;
mod mem;
mod global_alloc;
mod device;

extern crate alloc;

use log::{info, warn, debug};
use uefi::proto::device_path::text::{
    AllowShortcuts, DevicePathToText, DisplayOnly,
};
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::partition::PartitionInfo;
use uefi::table::{SystemTable, Boot};
use uefi::table::boot::{SearchType, MemoryType, BootServices};
use uefi::{Identify, Result, entry, Handle, Status, allocator};
use x86_64::VirtAddr;
use crate::device::partition::find_current_boot_partition;
use crate::device::qemu::exit_qemu;
use crate::device::retrieve::list_handles;
use crate::acpi::find_acpi_table_pointer;
use crate::fs::{open_sfs, load_file_sfs};
use crate::mem::frame_allocator::RTFrameAllocator;
use crate::mem::page_allocator::runtime;
use crate::print_panic::PrintPanic;
use crate::framebuffer::{locate_framebuffer, Framebuffer};
use crate::logger::{init_framebuffer_logger, init_uefi_services_logger};



#[entry]
fn efi_main(image_handle: Handle, mut system_table: SystemTable<Boot>) -> Status {
    let mut st = unsafe { 
        uefi::allocator::init(&mut system_table);
        system_table.unsafe_clone()
    };

    // locate framebuffer and iniitialize framebuffer logger
    let framebuffer: Option<Framebuffer> = match locate_framebuffer(&st) {
        Ok(fb) => {
            init_framebuffer_logger(unsafe { &*(&fb as *const _) });
            info!("framebuffer logger is initialized.");
            Some(fb)
        },
        Err(e) => {
            init_uefi_services_logger(&mut st);
            warn!("failed to initialize framebuffer logger, use uefi stdout logger as fallback.");
            None
        },
    };
    let boot_services = st.boot_services();

    find_acpi_table_pointer(&st);

    // find partition of current loaded image.
    let partitions = list_handles::<PartitionInfo>(boot_services);
    let current_image_partition = match find_current_boot_partition(boot_services, &partitions) {
        Some(t) => t,
        None => panic!("failed to find partition of current loaded image")
    };
    info!("current loaded image partition: {}", &*current_image_partition.device_path_string);

    // load kernel to memory
    let mut fs = open_sfs(boot_services, current_image_partition.handle)
        .or_panic("cannot open protocol SimpleFileSystem of efi image handle.")
        .open_volume()
        .or_panic("cannot open volumn of efi image filesystem");


    let kernel = load_file_sfs(&system_table, &mut fs, "kernel-x86_64");
    info!("kernel size: {}", kernel.unwrap().len());

    debug!("exiting boot services");
    let (system_table, mut memory_map) = system_table.exit_boot_services(MemoryType::LOADER_DATA);
    allocator::exit_boot_services();

    // boot service 现在已经退出，所以我们需要自己实现一个 GlobalAllocator
    // 要把之前的东西，例如 kernel 指针，framebuffer 指针映射到 runtime 的 memory map 中、
    // 以免被新的 allocator 覆写（虽然他们可能不在同一个 UEFI 内存区域，但是保险起见还是要映射）。
    // 之后内核也是访问这片 memory map？？

    memory_map.sort();
    exit_qemu(device::qemu::QemuExitCode::Success);

    let mut frame_allocator = RTFrameAllocator::new(memory_map.entries());

    let bootloader_page_table = runtime::map_boot_stage_page_table(&mut frame_allocator);
    let kernel_page_table = runtime::create_page_table(&mut frame_allocator, VirtAddr::new(0));

    //info!("sleep 3600 seconds...");
    Status::SUCCESS
}


fn print_image_path(boot_services: &BootServices, image_handle: &Handle) -> Result {
    let loaded_image = boot_services
        .open_protocol_exclusive::<LoadedImage>(*image_handle)?;

    let device_path_to_text_handle = *boot_services
        .locate_handle_buffer(SearchType::ByProtocol(&DevicePathToText::GUID))?
        .first()
        .expect("DevicePathToText is missing");

    let device_path_to_text = boot_services
        .open_protocol_exclusive::<DevicePathToText>(
            device_path_to_text_handle,
        )?;

    let image_device_path =
        loaded_image.file_path().expect("File path is not set");
    let image_device_path_text = device_path_to_text
        .convert_device_path_to_text(
            boot_services,
            image_device_path,
            DisplayOnly(true),
            AllowShortcuts(false),
        )
        .expect("convert_device_path_to_text failed");

    info!("Image path: {}", &*image_device_path_text);
    Ok(())
}