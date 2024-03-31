#![no_std]
#![no_main]
#![feature(asm_const)]
#![feature(offset_of)]
#![feature(abi_x86_interrupt)]
#![feature(custom_test_frameworks)]
#![feature(maybe_uninit_uninit_array)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use acpi::lapic::setup_apic;
use gdt::init_gdt;
use interrupt::init_idt;

use mem::frame_allocator::init_frame_allocator;
use shared::{arg::KernelArg};

use x86_64::{instructions::{self, interrupts::{self}}, VirtAddr};
use shared::print_panic::PrintPanic;

use crate::{arch_spec::cpuid::cpu_info, framebuffer::{init_framebuffer}, logger::{init_framebuffer_logger}};
use crate::acpi::ap_startup::setup_ap_startup;
use crate::cpu::LogicalCpuId;

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

extern crate alloc;

pub static CPU_COUNT: AtomicU32 = AtomicU32::new(0);
pub static AP_READY: AtomicBool = AtomicBool::new(false);
static BSP_READY: AtomicBool = AtomicBool::new(false);

#[no_mangle]
pub extern "C" fn _start(arg: &'static KernelArg) -> ! {
    #[cfg(test)]
    test_main();

    init_framebuffer(arg);
    init_framebuffer_logger();

    cpu_info().or_panic("failed to print cpu info");

    init_frame_allocator(
        VirtAddr::new(arg.phys_mem_mapped_addr),
        arg.phys_mem_size,
        &arg.unav_phys_mem_regions[..arg.unav_phys_mem_regions_len]
    );

    unsafe {
        interrupts::disable();

        init_gdt(cpu::LogicalCpuId::BSP, arg.stack_top_addr);
        init_idt(cpu::LogicalCpuId::BSP);

        setup_apic(arg.acpi.local_apic_base as u64, LogicalCpuId::BSP);
        interrupts::enable();

        CPU_COUNT.store(1, Ordering::SeqCst);
        AP_READY.store(false, Ordering::SeqCst);
        BSP_READY.store(false, Ordering::SeqCst);

        setup_ap_startup(
            &arg.acpi.local_apic[0..arg.acpi.local_apic_count],
            VirtAddr::new(arg.kernel_pml4_start_addr)
        );
    }

    unreachable!()
}

#[repr(packed)]
pub struct KernelArgsAp {
    // TODO: u32?
    cpu_id: u64,

    page_table: u64,
    stack_start: u64,
    stack_end: u64,
}

pub unsafe extern "C" fn _start_ap(arg_ptr: *const KernelArgsAp) -> ! {
    unsafe {
        let arg = &*arg_ptr;
        let cpu_id = LogicalCpuId(arg.cpu_id as u8);

        interrupts::disable();

        init_gdt(cpu_id, arg.stack_end);
        init_idt(cpu_id);

        interrupts::enable();

        setup_apic(0, cpu_id);
        AP_READY.store(true, Ordering::SeqCst);
    }

    while !BSP_READY.load(Ordering::SeqCst) {
        pause()
    }

    unreachable!();
}

fn halt() -> ! {
    loop {
        instructions::hlt();
    }
}

pub fn pause() {
    unsafe { asm!("pause", options(nomem, nostack)); }
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