#![no_std]
#![no_main]
#![feature(step_trait)]

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
mod context;

use core::arch::asm;
use core::mem::MaybeUninit;

extern crate alloc;

use alloc::slice;
use log::{info, warn, debug, error};
use uefi::proto::media::partition::PartitionInfo;
use uefi::table::{SystemTable, Boot};
use uefi::table::boot::MemoryType;
use uefi::{entry, Handle, Status, allocator};
use x86_64::registers::control::{Cr0, Cr0Flags};
use x86_64::registers::model_specific::{Efer, EferFlags};
use x86_64::VirtAddr;
use crate::context::{context_switch, KernelArg};
use crate::device::partition::find_current_boot_partition;
use crate::device::qemu::exit_qemu;
use crate::device::retrieve::{list_handles, ProtocolWithHandle};
use crate::acpi::find_acpi_table_pointer;
use crate::fs::{open_sfs, load_file_sfs};
use crate::global_alloc::switch_to_runtime_global_allocator;
use crate::kernel::load_kernel_to_virt_mem;
use crate::mem::frame_allocator::RTFrameAllocator;
use crate::mem::page_allocator;
use crate::mem::runtime_map::{alloc_and_map_gdt, alloc_and_map_kernel_stack, map_framebuffer, map_kernel_arg, map_physics_memory};
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


    let kernel = match load_file_sfs(&system_table, &mut fs, "kernel-x86_64") {
        Some(kernel_slice) => kernel_slice,
        None => {
            error!("kernel is not found in current loaded image!");
            halt();
        }
    };
    info!("loaded kernel to physics address: 0x{:x}", &kernel[0] as *const _ as usize);

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

    // 使用 RTFrameAllocator 在 runtime memory map 新的 PML4 页表，写到 CR3.
    // 现在 CR3 寄存器是这个 bootloader_page_table 了，但是前面的一些引用，例如 kernel，framebuffer 依然是有效的。
    // 因为我们把旧 PML4 页表的 PTE 写入到了我们的新表，并正确设置了内存偏移（UEFI 直接映射物理内存，所以它的物理内存和虚拟内存之间没有偏移）
    // 为什么需要复制一份 boot 阶段的 PML4？
    // * 新的 RTFrameAllocator 没有之前的物理页帧分配信息，可能会覆写之前 CR3 指向的 PML4 的物理页帧，而我们新分配的 PML4 表是
    //   由 RTFrameAllocator 分配的物理页帧，不会被覆盖。
    // * 可以继续使用之前的指针，例如 kernel 和 framebuffer，稍后也要将这些内存建立和当前页表的映射防止被覆写。
    let bootloader_page_table = page_allocator::runtime::map_boot_stage_page_table(&mut frame_allocator);
    let (mut kernel_page_table, kernel_pml4_phys_addr) = 
        page_allocator::runtime::create_page_table(&mut frame_allocator, VirtAddr::new(0));

    unsafe {
        // Enable support for the no-execute bit in page tables.
        Efer::update(|efer| *efer |= EferFlags::NO_EXECUTE_ENABLE);
        // Make the kernel respect the write-protection bits even when in ring 0 by default
        Cr0::update(|cr0| *cr0 |= Cr0Flags::WRITE_PROTECT);
    };

    // 加载内核到内核的 PML4 页表里，加载到四级页表的最后一个表项的起始，0xffff_ff80_0000_0000
    // kernel 在物理内存的地址位置，这个物理地址实际上是由 boot 阶段的 BootServices.allocate_pages 分配的
    let load_kernel = load_kernel_to_virt_mem(kernel, &mut kernel_page_table, &mut frame_allocator);
    info!("kernel entry virt addr: 0x{:x}", load_kernel.kernel_entry.as_u64());

    // map gdt
    let kernel_gdt = alloc_and_map_gdt(&mut kernel_page_table, &mut frame_allocator);
    info!("global descriptor virt addr: 0x{:x}", kernel_gdt);

    // 创建内核栈并加载到内核 PML4 页表
    let kernel_stack_size = 4096 * 128; // 128 KiB
    let kernel_stack_virt_addr = alloc_and_map_kernel_stack(kernel_stack_size, &mut kernel_page_table, &mut frame_allocator);
    info!("kernel stack virt addr: 0x{:x}", kernel_stack_virt_addr.as_u64());
    let kernel_stack_top_virt_addr = (kernel_stack_virt_addr + kernel_stack_size).align_down(16u8).as_u64();

    // map framebuffer to kernel virt addr
    let framebuffer_virt_addr = framebuffer.map(|f| {
        let framebuffer_virt_addr: VirtAddr = map_framebuffer(
            unsafe { slice::from_raw_parts(f.ptr, f.len) }, 
            &mut kernel_page_table,
            &mut frame_allocator
        );

        info!("kernel framebuffer virt addr: 0x{:x}", framebuffer_virt_addr.as_u64());
        framebuffer_virt_addr
    });



    // 映射帧分配器可用的地址空间（也就是物理内存地址空间）到内核页表
    let mapped_phys_space_virt_addr = map_physics_memory(
        frame_allocator.max_phys_addr(), 
        &mut kernel_page_table, 
        &mut frame_allocator
    );
    info!("kernel mapped all physics address space to virt addr: 0x{:x}", mapped_phys_space_virt_addr);

    // 创建内核参数，把这些参数传给内核来让内核读取一些信息    
    let kernel_arg = KernelArg {
        kernel_virt_space_offset:   load_kernel.kernel_virt_space_offset,

        kernel_start_addr:          kernel_stack_virt_addr.as_u64(),

        gdt_start_addr:             kernel_gdt.as_u64(),

        stack_top_addr:             (kernel_stack_virt_addr + kernel_stack_size).align_down(16u8).as_u64(),
        stack_size:                 kernel_stack_size,

        framebuffer_addr:           framebuffer_virt_addr.unwrap_or(VirtAddr::new(0)).as_u64(),
        framebuffer_len:            framebuffer.map(|f| f.len).unwrap_or(0),
        framebuffer_width:          framebuffer.map(|f| f.width).unwrap_or(0),
        framebuffer_height:         framebuffer.map(|f| f.height).unwrap_or(0),
        framebuffer_stride:         framebuffer.map(|f| f.stride).unwrap_or(0),

        phys_mem_mapped_addr:       mapped_phys_space_virt_addr.as_u64(),
        phys_mem_size:              frame_allocator.max_phys_addr().as_u64(),

        tls_template:               load_kernel.tls_template.unwrap_or_default()
    };
    let kernel_arg_virt_addr = map_kernel_arg(kernel_arg, &mut kernel_page_table, &mut frame_allocator);

    info!("switching to kernel, args: {:#?}", kernel_arg);
    unsafe {
        // 在 switch 的过程中就已经写入了内核 PML4 表
        context_switch(
            // 当前阶段还是无偏移映射，物理地址和虚拟地址无偏移并且是均等映射
            kernel_pml4_phys_addr.as_u64(), 
            kernel_stack_top_virt_addr,
            load_kernel.kernel_entry.as_u64(),
            kernel_arg_virt_addr.as_u64()
        )
    }
}

fn halt() -> ! {
    loop {
        unsafe { asm!("hlt") }
    }
}