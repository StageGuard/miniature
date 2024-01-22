#![no_std]
#![no_main]

mod panic;
mod acpi;
mod fs;
mod kernel;
mod framebuffer;
mod logger;
mod sync;
mod mem;
mod global_alloc;
mod device;

use core::arch::asm;
use core::mem::MaybeUninit;

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
use x86_64::structures::paging::FrameAllocator;
use crate::device::partition::find_current_boot_partition;
use crate::device::qemu::exit_qemu;
use crate::device::retrieve::{list_handles, ProtocolWithHandle};
use crate::acpi::find_acpi_table_pointer;
use crate::fs::{open_sfs, load_file_sfs};
use crate::global_alloc::switch_to_runtime_global_allocator;
use crate::mem::RTMemoryRegion;
use crate::mem::frame_allocator::RTFrameAllocator;
use crate::mem::page_allocator::runtime;
use crate::panic::PrintPanic;
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
        Some(fb) => {
            init_framebuffer_logger(unsafe { &*(&fb as *const _) });
            info!("framebuffer logger is initialized.");
            Some(fb)
        },
        None => {
            init_uefi_services_logger(&mut st);
            warn!("failed to initialize framebuffer logger, use uefi stdout logger as fallback.");
            None
        },
    };
    let boot_services = st.boot_services();

    let acpi_ptr = find_acpi_table_pointer(&st);

    // find partition of current loaded image.
    const uninited: MaybeUninit<ProtocolWithHandle<'_, PartitionInfo>> = MaybeUninit::<ProtocolWithHandle<PartitionInfo>>::uninit();
    let mut partitions = [uninited; 256];
    let partition_len = list_handles::<PartitionInfo>(boot_services, &mut partitions);
    let current_image_partition = match find_current_boot_partition(boot_services, &partitions[..partition_len]) {
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

    // // boot service 现在已经退出，所以我们需要自己实现一个 GlobalAllocator
    // // 要把之前的东西，例如 kernel 指针，framebuffer 指针映射到 runtime 的 memory map 中、
    // // 以免被新的 allocator 覆写（虽然他们可能不在同一个 UEFI 内存区域，但是保险起见还是要映射）。
    // // 之后内核也是访问这片 memory map？？

    switch_to_runtime_global_allocator();
    memory_map.sort();

    let mut frame_allocator = RTFrameAllocator::new(memory_map.entries());

    let bootloader_page_table = runtime::map_boot_stage_page_table(&mut frame_allocator);
    let kernel_page_table = runtime::create_page_table(&mut frame_allocator, VirtAddr::new(0));

    info!("efi reaches program end, halt cpu");
    halt();
    Status::SUCCESS
}

fn halt() -> ! {
    loop {
        unsafe { asm!("hlt") }
    }
}