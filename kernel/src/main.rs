#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(custom_test_frameworks)]
#![feature(maybe_uninit_uninit_array)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::{arch::{self, asm}, fmt::Write, mem::MaybeUninit, slice};

use acpi::apic::setup_apic;
use alloc::vec::Vec;
use device::qemu::exit_qemu;
use gdt::init_gdt_and_protected_mode;
use interrupt::init_idt;
use lazy_static::lazy_static;
use log::{info, Log};
use mem::frame_allocator::init_frame_allocator;
use shared::{arg::KernelArg, framebuffer::{FBPixelFormat, Framebuffer}, uni_processor::UPSafeCell};
use spin::mutex::Mutex;
use x86_64::{instructions::{self, interrupts::{self, int3}}, VirtAddr};

use crate::{framebuffer::{init_framebuffer, FRAMEBUFFER}, logger::{init_framebuffer_logger, FramebufferLogger}};

mod panic;
mod device;
mod mem;
mod logger;
mod framebuffer;
mod gdt;
mod interrupt;
mod acpi;

extern crate alloc;


#[no_mangle]
pub extern "C" fn _start(arg: &'static KernelArg) -> ! {
    #[cfg(test)]
    test_main();

    init_framebuffer(arg);
    init_framebuffer_logger();

    init_frame_allocator(
        VirtAddr::new(arg.phys_mem_mapped_addr),
        arg.phys_mem_size,
        &arg.unav_phys_mem_regions[..arg.unav_phys_mem_regions_len]
    );

    unsafe {
        init_gdt_and_protected_mode(arg.gdt_start_addr);
        init_idt();

        setup_apic(arg.acpi.local_apic_base as u64);
        interrupts::enable();

        
    }

    halt();
}

fn halt() -> ! {
    loop {
        instructions::hlt();
    }
}

#[cfg(test)]
pub fn test_runner(tests: &[&dyn Fn()]) {
    qemu_println!("Running {} tests", tests.len());
    for test in tests {
        test();
    }

    exit_qemu(device::qemu::QemuExitCode::Success);
}