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
#![feature(step_trait)]
#![feature(slice_ptr_get)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

use alloc::sync::Arc;
use core::arch::asm;
use core::hint::spin_loop;
use core::mem::{MaybeUninit, offset_of, transmute};
use core::ptr::addr_of_mut;
use core::slice;
use core::slice::from_raw_parts;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use log::info;
use spin::Once;
use spinning_top::RwSpinlock;
use acpi::local_apic::setup_apic;
use gdt::init_gdt;
use interrupt::init_idt;

use mem::frame_allocator::init_frame_allocator;
use shared::{arg::KernelArg, BOOTSTRAP_BYTES_P4};

use x86_64::{instructions::{self, interrupts::{self}}, VirtAddr};
use x86_64::instructions::tlb;
use x86_64::structures::paging::page_table::PageTableEntry;
use x86_64::structures::paging::{Page, PageTable, PageTableFlags, Size4KiB};
use shared::print_panic::PrintPanic;

use crate::{arch_spec::cpuid::cpu_info, framebuffer::{init_framebuffer}, logger::{init_framebuffer_logger}};
use crate::acpi::ap_startup::setup_ap_startup;
use crate::acpi::io_apic::setup_io_apic;
use crate::context::init_context;
use crate::context::list::{context_storage, context_storage_mut};
use crate::context::status::Status;
use crate::context::switch::{switch_context, SwitchResult};
use crate::cpu::{LogicalCpuId, PercpuBlock};
use crate::device::com::init_com;
use crate::interrupt::{enable_and_halt, enable_and_nop};
use crate::ipi::{ipi, ipi_single, IpiKind, IpiTarget};
use crate::mem::load_elf::elf_copy_to_addrsp;
use crate::mem::{get_kernel_pml4_page_table_addr, PAGE_SIZE, set_kernel_pml4_page_table};
use crate::mem::aligned_box::AlignedBox;
use crate::mem::heap::RT_HEAP_SPACE;
use crate::mem::user_addr_space::RwLockUserAddrSpace;
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
mod ipi;
mod fs;
mod interrupt_macro;

extern crate alloc;

pub static CPU_COUNT: AtomicU32 = AtomicU32::new(0);
pub static AP_READY: AtomicBool = AtomicBool::new(false);
static BSP_READY: AtomicBool = AtomicBool::new(false);

static BOOTSTRAP: Once<&'static [u8]> = Once::new();
static BOOTSTRAP_USR_ADDRSP_BASE: Once<u64> = Once::new();

// entry for all things
#[no_mangle]
pub extern "C" fn _start(arg: &'static KernelArg) -> ! {
    #[cfg(test)]
    test_main();

    init_framebuffer(arg);
    init_framebuffer_logger();

    cpu_info().or_panic("failed to print cpu info");

    BOOTSTRAP.call_once(|| unsafe {
        slice::from_raw_parts(arg.bootstrap_base as *const u8, arg.bootstrap_len)
    });

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

    // BSP_READY.store(true, Ordering::SeqCst);

    // bsp kernel main

    init_context();

    match context_storage_mut().spawn(true, userspace_init) {
        Ok(lock) => {
            let mut context = lock.write();
            context.status = Status::Runnable;

            // bootloader mapped bootstrap to KernelPageTable[BOOTSTRAP_P4][0]
            // so we map bootstrap: KernelPageTable[BOOTSTRAP_P4][0] -> AddrspPageTable[0][511]
            // bootstrap size bust be smaller than 1GB
            match &context.addrsp {
                Some(ref addrsp) => {
                    let mut addrsp_guard = addrsp.acquire_write();

                    let kpt_pml4 = unsafe { &*(get_kernel_pml4_page_table_addr() as *const PageTable) };
                    let kpt_bsp4_pml3 = unsafe {
                        &*((&kpt_pml4[BOOTSTRAP_BYTES_P4 as usize].addr()).as_u64() as *const PageTable)
                    };

                    let addrsp_pt = unsafe { addrsp_guard.page_table() };
                    let addrsp_pt_0_pml3 = unsafe {
                        &mut *((&addrsp_pt[0].addr()).as_u64() as *mut PageTable)
                    };

                    addrsp_pt_0_pml3[511] = kpt_bsp4_pml3[0].clone();
                    // 0x7fc0000000 是 PageTable[0][511] 1gb 页的起始虚拟地址
                    BOOTSTRAP_USR_ADDRSP_BASE.call_once(|| 0x7f_c000_0000);
                }
                None => panic!("user address space of bootstrap context is not found.")
            }

        }
        Err(err) => {
            panic!("failed to spawn userspace_init: {:?}", err);
        }
    }

    unsafe { run_userspace() }
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

    unreachable!()
}

extern "C" fn userspace_init() {
    let contexts = context_storage();
    let current_context = contexts.current()
        .or_panic("failed to get userspace init context");
    let context_read = current_context.read();
    let addrsp = match context_read.addrsp {
        None => panic!("failed to get address space of userspace init context"),
        Some(ref rsp) => Arc::clone(rsp)
    };
    let bootstrap_entry = unsafe {
        let bootstrap_slice_user_addrsp = from_raw_parts(
            *BOOTSTRAP_USR_ADDRSP_BASE
                .get()
                .or_panic("failed to get bootstrap base at user address space")
                as *const u8,
            BOOTSTRAP
                .get()
                .or_panic("failed to get bootstrap length")
                .len()
        );
        elf_copy_to_addrsp(bootstrap_slice_user_addrsp, addrsp)
    };
    infohart!("bootstrap entry: 0x{:x}", bootstrap_entry.as_u64());

    // validate
    {
        match context_read.addrsp {
            None => panic!("failed to get address space of userspace init context"),
            Some(ref rsp) => unsafe {
                let mut rsp = Arc::clone(rsp);
                let mut rsp_guard = rsp.acquire_write();
                rsp_guard.validate();
            }
        };
    }

    drop(context_read);

    match context_storage().current()
        .or_panic("bootstrap was not running inside any context")
        .write()
        .regs_mut()
        .or_panic("bootstrap needs registers to be available")
    {
        ref mut regs => {
            regs.init();
            regs.set_instr_pointer(bootstrap_entry.as_u64() as usize)
        }
    }
}

unsafe fn run_userspace() -> ! {
    loop {
        interrupts::disable();
        match switch_context() {
            SwitchResult::Switched { .. } => {
                enable_and_nop()
            }
            SwitchResult::AllContextsIdle => {
                enable_and_halt()
            }
        }
    }
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