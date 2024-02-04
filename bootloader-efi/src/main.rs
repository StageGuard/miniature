#![no_std]
#![no_main]
#![feature(step_trait)]
#![feature(maybe_uninit_uninit_array)]

use core::mem::MaybeUninit;
use core::slice;
use log::{info, warn, debug};
use mem::RTMemoryRegionDescriptor;
use shared::arg::{KernelArg, MemoryRegion, MemoryRegionKind};
use shared::framebuffer::Framebuffer;
use uefi::proto::media::partition::PartitionInfo;
use uefi::table::{SystemTable, Boot};
use uefi::table::boot::{MemoryDescriptor, MemoryMap, MemoryType};
use uefi::{entry, Handle, Status, allocator};
use x86_64::registers::control::{Cr0, Cr0Flags};
use x86_64::registers::model_specific::{Efer, EferFlags};
use x86_64::structures::paging::{Mapper, PageTableFlags};
use x86_64::VirtAddr;
use crate::context::context_switch;
use crate::device::partition::find_current_boot_partition;
use crate::device::retrieve::{list_handles, ProtocolWithHandle};
use crate::acpi::find_acpi_table_pointer;
use crate::fs::{open_sfs, load_file_sfs};
use crate::kernel::load_kernel_to_virt_mem;
use crate::mem::frame_allocator::LinearIncFrameAllocator;
use crate::mem::page_allocator;
use crate::mem::runtime_map::{
    alloc_and_map_gdt_identically, 
    alloc_and_map_kernel_stack, 
    map_context_switch_identically, 
    map_framebuffer, map_kernel_arg, 
    map_physics_memory
};
use shared::print_panic::PrintPanic;
use crate::framebuffer::locate_framebuffer;
use crate::logger::{init_framebuffer_logger, init_uefi_services_logger};

mod panic;
mod acpi;
mod fs;
mod kernel;
mod framebuffer;
mod logger;
mod mem;
mod device;
mod context;

#[entry]
fn efi_main(image_handle: Handle, mut system_table: SystemTable<Boot>) -> Status {
    // SAFETY: 详见 unsafe_clone 和 init
    let mut st = unsafe { 
        uefi::allocator::init(&mut system_table);
        system_table.unsafe_clone()
    };

    // locate framebuffer and iniitialize framebuffer logger
    let framebuffer: Option<Framebuffer> = match locate_framebuffer(&st) {
        Some(fb) => {
            // SAFETY: the framebuffer poniter points to the corresponding memory region
            // that is allocated by uefi
            init_framebuffer_logger(unsafe { &*(&fb as *const _) });
            info!("efi framebuffer logger is initialized.");
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
    const PWH_UNINITIALIZED: MaybeUninit<ProtocolWithHandle<'_, PartitionInfo>> = MaybeUninit::<ProtocolWithHandle<PartitionInfo>>::uninit();
    let mut partitions = [PWH_UNINITIALIZED; 256];
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
        None => panic!("kernel is not found in current loaded image!")
    };
    info!("loaded kernel to physics address: 0x{:x}", &kernel[0] as *const _ as usize);

    debug!("exiting boot services");
    let (system_table, mut memory_map) = system_table.exit_boot_services(MemoryType::LOADER_DATA);
    allocator::exit_boot_services();

    // // boot service 现在已经退出，所以我们需要自己实现一个 GlobalAllocator
    // // 要把之前的东西，例如 kernel 指针，framebuffer 指针映射到 runtime 的 memory map 中、
    // // 以免被新的 allocator 覆写（虽然他们可能不在同一个 UEFI 内存区域，但是保险起见还是要映射）。
    // // 之后内核也是访问这片 memory map？？

    memory_map.sort();

    let mut frame_allocator = LinearIncFrameAllocator::new(memory_map.entries().copied());

    // 使用 RTFrameAllocator 在 runtime memory map 新的 PML4 页表，写到 CR3.
    // 现在 CR3 寄存器是这个 bootloader_page_table 了，但是前面的一些引用，例如 kernel，framebuffer 依然是有效的。
    // 因为我们把旧 PML4 页表的 PTE 写入到了我们的新表，并正确设置了内存偏移（UEFI 直接映射物理内存，所以它的物理内存和虚拟内存之间没有偏移）
    // 为什么需要复制一份 boot 阶段的 PML4？
    // * 新的 RTFrameAllocator 没有之前的物理页帧分配信息，可能会覆写之前 CR3 指向的 PML4 的物理页帧，而我们新分配的 PML4 表是
    //   由 RTFrameAllocator 分配的物理页帧，不会被覆盖。
    // * 可以继续使用之前的指针，例如 kernel 和 framebuffer，稍后也要将这些内存建立和当前页表的映射防止被覆写。
    let bootloader_page_table = page_allocator::runtime::map_boot_stage_page_table(&mut frame_allocator);
    let (mut kernel_page_table, kernel_pml4_table_phys_frame) = 
        page_allocator::runtime::create_page_table(&mut frame_allocator, VirtAddr::new(0));

    unsafe {
        // Enable support for the no-execute bit in page tables.
        Efer::update(|efer| *efer |= EferFlags::NO_EXECUTE_ENABLE );
        // Make the kernel respect the write-protection bits even when in ring 0 by default
        Cr0::update(|cr0| *cr0 |= Cr0Flags::WRITE_PROTECT);
    };

    // 加载内核到内核的 PML4 页表里，加载到四级页表的最后一个表项的起始，0xffff_ff80_0000_0000
    // kernel 在物理内存的地址位置，这个物理地址实际上是由 boot 阶段的 BootServices.allocate_pages 分配的
    let load_kernel = load_kernel_to_virt_mem(kernel, &mut kernel_page_table, &mut frame_allocator);
    info!("kernel entry virt addr: 0x{:x}", load_kernel.kernel_entry.as_u64());

    // map gdt, identical map
    let kernel_gdt = alloc_and_map_gdt_identically(&mut kernel_page_table, &mut frame_allocator);
    info!("global descriptor table virt addr: 0x{:x}", kernel_gdt.as_u64());

    // map kernel pml4 table, identical map
    // because Cr3 register is also virtual address of page table.
    unsafe {
        kernel_page_table.identity_map(kernel_pml4_table_phys_frame, PageTableFlags::PRESENT, &mut frame_allocator)
            .or_panic("failed to map kernel pml4 table")
            .ignore();
    }

    // 创建内核栈并加载到内核 PML4 页表
    let kernel_stack_size = 4096 * 128; // 128 KiB
    let kernel_stack_virt_addr = alloc_and_map_kernel_stack(kernel_stack_size, &mut kernel_page_table, &mut frame_allocator);
    info!("kernel stack virt addr: 0x{:x}", kernel_stack_virt_addr.as_u64());
    let kernel_stack_top_virt_addr = (kernel_stack_virt_addr + kernel_stack_size).align_down(16u8).as_u64();

    let context_switch_virt_addr = map_context_switch_identically(context_switch as *const fn(), &mut kernel_page_table, &mut frame_allocator);
    info!("mapped context switch fn to virt addr: 0x{:x}", context_switch_virt_addr.as_u64());

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

    let regions = construct_unsafe_phys_mem_region_map(&memory_map, &frame_allocator, &framebuffer, &kernel);
    // 创建内核参数，把这些参数传给内核来让内核读取一些信息
    let kernel_arg = KernelArg {
        kernel_virt_space_offset:   load_kernel.kernel_virt_space_offset,

        gdt_start_addr:             kernel_gdt.as_u64(),
        //acpi_table_start_addr:      

        stack_top_addr:             (kernel_stack_virt_addr + kernel_stack_size).align_down(16u8).as_u64(),
        stack_size:                 kernel_stack_size,

        framebuffer_addr:           framebuffer_virt_addr.unwrap_or(VirtAddr::new(0)).as_u64(),
        framebuffer_len:            framebuffer.map(|f| f.len).unwrap_or(0),
        framebuffer_width:          framebuffer.map(|f| f.width).unwrap_or(0),
        framebuffer_height:         framebuffer.map(|f| f.height).unwrap_or(0),
        framebuffer_stride:         framebuffer.map(|f| f.stride).unwrap_or(0),

        phys_mem_mapped_addr:       mapped_phys_space_virt_addr.as_u64(),
        phys_mem_size:              frame_allocator.max_phys_addr().as_u64(),
        unav_phys_mem_regions:      unsafe { *(&regions.0 as *const _ as *const [MemoryRegion; 512]) },
        unav_phys_mem_regions_len:  regions.1,

        tls_template:               load_kernel.tls_template.unwrap_or_default(),
    };
    
    // TODO: 详见 map_kernel_arg 注解
    // SAFETY: 详见 map_kernel_arg 注解
    let kernel_arg_virt_addr: VirtAddr = map_kernel_arg(&mut unsafe { *(&kernel_arg as *const _ as *mut KernelArg) }, &mut kernel_page_table, &mut frame_allocator);

    info!("switching to kernel entry point virt addr: 0x{:x}, arg virt addr: 0x{:x}", load_kernel.kernel_entry, kernel_arg_virt_addr);
    unsafe {
        // 在 switch 的过程中就已经写入了内核 PML4 表
        context_switch(
            // 当前阶段还是无偏移映射，物理地址和虚拟地址无偏移并且是均等映射
            kernel_pml4_table_phys_frame, 
            kernel_stack_top_virt_addr,
            load_kernel.kernel_entry.as_u64(),
            kernel_arg_virt_addr.as_u64()
        )
    }
}

/// 构建内核无法使用的物理内存区域，这些内存区域存放了 bootloader 信息或者其他有用信息。
/// 内核在分配物理帧时应该跳过这些区域。
/// 
/// 以下物理页帧区域对内核来说不可用：
/// * UEFI 定义的除了 CONVENTIONAL，BOOT_SERVICES_CODE 和 BOOT_SERVICES_DATA 以外的所有区域
/// * runtime 阶段 FrameAllocator 分配的物理页帧区域
/// * framebuffer 和 kernel 文件所在的物理页帧区域（这些是在 exit_boot_services 之前分配的）
#[inline]
fn construct_unsafe_phys_mem_region_map<I: ExactSizeIterator<Item = MemoryDescriptor> + Clone>(
    memory_map: &MemoryMap,
    frame_allocator: &LinearIncFrameAllocator<I, MemoryDescriptor>,
    framebuffer: &Option<Framebuffer>,
    kernel_bytes: &[u8]
) -> ([MaybeUninit<MemoryRegion>; 512], usize) {
    let mut regions: [MaybeUninit<MemoryRegion>; 512] = MaybeUninit::uninit_array();
    let mut curr_idx = 0;

    // UEFI 定义的除了 CONVENTIONAL，BOOT_SERVICES_CODE 和 BOOT_SERVICES_DATA 以外的所有区域
    for rg in memory_map.entries().copied() {
        if !rg.usable_after_bootloader_exit() {
            regions[curr_idx].write(MemoryRegion {
                start: rg.start().as_u64(),
                length: rg.page_count * 4096,
                kind: MemoryRegionKind::Bootloader
            });
            curr_idx += 1;
        }
    }

    // runtime 阶段 FrameAllocator 分配的物理页帧区域
    regions[curr_idx].write(frame_allocator.allocated_region());
    curr_idx += 1;

    framebuffer.map(|framebuffer| {
        let framebuffer_start_phys_addr = framebuffer.ptr as u64;
        regions[curr_idx].write(MemoryRegion {
            start: framebuffer_start_phys_addr,
            length: framebuffer.len as u64,
            kind: MemoryRegionKind::Bootloader
        });
        curr_idx += 1;
    });

    let kernel_bytes_starst_phys_addr = &kernel_bytes[0] as *const _ as u64;
    regions[curr_idx].write(MemoryRegion {
        start: kernel_bytes_starst_phys_addr,
        length: kernel_bytes.len() as u64,
        kind: MemoryRegionKind::Bootloader
    });
    curr_idx += 1;

    (regions, curr_idx)
}