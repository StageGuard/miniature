#![no_std]
#![no_main]
#![feature(asm_const)]
#![feature(offset_of)]
#![feature(allocator_api)]
#![feature(naked_functions)]
#![feature(abi_x86_interrupt)]
#![feature(arbitrary_self_types)]
#![feature(custom_test_frameworks)]
#![feature(maybe_uninit_uninit_array)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::arch::asm;
use core::hint::spin_loop;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use acpi::local_apic::setup_apic;
use gdt::init_gdt;
use interrupt::init_idt;

use mem::frame_allocator::init_frame_allocator;
use shared::{arg::KernelArg};

use x86_64::{instructions::{self, interrupts::{self}}, VirtAddr};
use shared::print_panic::PrintPanic;

use crate::{arch_spec::cpuid::cpu_info, framebuffer::{init_framebuffer}, logger::{init_framebuffer_logger}};
use crate::acpi::ap_startup::setup_ap_startup;
use crate::acpi::io_apic::setup_io_apic;
use crate::context::init_context;
use crate::cpu::{LogicalCpuId, PercpuBlock};
use crate::device::com::init_com;
use crate::ipi::{ipi, ipi_single, IpiKind, IpiTarget};
use crate::mem::set_kernel_pml4_page_table;
use crate::syscall::init_syscall;

mod arch_spec;
mod panic;
mod device;
mod mem;
mod logger;
mod framebuffer;
mod gdt;
mod interrupt;
mod acpi;
mod cpu;
mod syscall;
mod context;
mod common;
mod syscall_module;
mod ipi;

extern crate alloc;

pub static CPU_COUNT: AtomicU32 = AtomicU32::new(0);
pub static AP_READY: AtomicBool = AtomicBool::new(false);
static BSP_READY: AtomicBool = AtomicBool::new(false);

// entry for all things
#[no_mangle]
pub extern "C" fn _start(arg: &'static KernelArg) -> ! {
    #[cfg(test)]
    test_main();

    init_framebuffer(arg);
    init_framebuffer_logger();

    cpu_info().or_panic("failed to print cpu info");

    set_kernel_pml4_page_table(arg.kernel_pml4_start_addr);
    init_frame_allocator(
        VirtAddr::new(arg.phys_mem_mapped_addr),
        arg.phys_mem_size,
        &arg.unav_phys_mem_regions[..arg.unav_phys_mem_regions_len]
    );

    interrupts::disable();

    unsafe {
        init_gdt(LogicalCpuId::BSP, arg.stack_top_addr);
        init_idt(LogicalCpuId::BSP);

        setup_apic(arg.acpi.local_apic_base as u64, LogicalCpuId::BSP);

        init_syscall();
    }

    interrupts::enable();

    CPU_COUNT.store(1, Ordering::SeqCst);
    AP_READY.store(false, Ordering::SeqCst);
    BSP_READY.store(false, Ordering::SeqCst);

    setup_ap_startup(
        &arg.acpi.local_apic[..arg.acpi.local_apic_count],
        VirtAddr::new(arg.kernel_pml4_start_addr)
    );

    setup_io_apic(
        &arg.acpi.io_apic[..arg.acpi.io_apic_count],
        &arg.acpi.interrupt_src_override[..arg.acpi.interrupt_src_override_count]
    );

    unsafe {
        init_com();
    }

    BSP_READY.store(true, Ordering::SeqCst);

    // bsp kernel main

    init_context();

    unreachable!()
}

#[repr(packed)]
pub struct KernelArgAp {
    // TODO: u32?
    cpu_id: u64,

    page_table: u64,
    stack_start: u64,
    stack_end: u64,
}

// entry for ap
pub unsafe extern "C" fn _start_ap(arg_ptr: *const KernelArgAp) -> ! {
    unsafe {
        let arg = &*arg_ptr;
        let cpu_id = LogicalCpuId(arg.cpu_id as u8);

        init_gdt(cpu_id, arg.stack_end);
        init_idt(cpu_id);


        setup_apic(0, cpu_id);
        init_syscall();
        AP_READY.store(true, Ordering::SeqCst);

        interrupts::enable();
    }

    // waiting for bsp initialization.
    while !BSP_READY.load(Ordering::SeqCst) {
        spin_loop()
    }

    unreachable!();
}

fn halt() -> ! {
    loop {
        instructions::hlt();
    }
}

#[cfg(test)]
pub fn test_runner(tests: &[&dyn Fn()]) {
    use crate::device::qemu::exit_qemu;
    qemu_println!("Running {} tests", tests.len());
    
    for test in tests {
        test();
    }

    exit_qemu(device::qemu::QemuExitCode::Success);
}