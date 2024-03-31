use alloc::{boxed::Box, collections::BTreeMap};
use lazy_static::lazy_static;
use log::info;
use shared::{print_panic::PrintPanic};
use spin::RwLock;
use x86_64::{registers::control::Cr2, structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode}, VirtAddr};
use core::{fmt::Write};

use crate::{acpi::lapic::LOCAL_APIC, cpu::LogicalCpuId, device::qemu::exit_qemu, gdt::{pcr}, halt, infohart, mem::{frame_allocator::frame_alloc_n, PAGE_SIZE}, qemu_println};
use crate::acpi::lapic::IpiKind;

const DEPENDENT_STACK_SIZE: usize = 65536;
pub const LAPIC_TIMER_HANDLER_IDT: u32 = 48;

lazy_static! {
    static ref IDTS: RwLock<BTreeMap<LogicalCpuId, &'static mut InterruptDescriptorTable>> = RwLock::new(BTreeMap::new());
}

pub unsafe fn init_idt(cpu_id: LogicalCpuId) {
    let mut idts_guard = IDTS.write();
    idts_guard.insert(cpu_id, Box::leak(Box::new(InterruptDescriptorTable::new())));
    let idt = idts_guard.get_mut(&cpu_id).or_panic("failed to get idt");

    let dependent_ist = {
        let index = 0_u8;
        let stack = frame_alloc_n(DEPENDENT_STACK_SIZE / PAGE_SIZE)
            .or_panic("failed to allocate backup dependent stack");

        (*pcr()).tss.interrupt_stack_table[usize::from(index)] = 
            VirtAddr::new(stack.start_address().as_u64()) + DEPENDENT_STACK_SIZE;

        index
    };

    // exceptions
    idt.breakpoint.set_handler_addr(VirtAddr::new(breakpoint_handler as u64))
        .set_present(true)
        .set_privilege_level(x86_64::PrivilegeLevel::Ring3);
    idt.double_fault.set_handler_addr(VirtAddr::new(double_fault_handler as u64))
        .set_stack_index(dependent_ist.into());
    idt.page_fault.set_handler_addr(VirtAddr::new(page_fault_handler as u64));

    idt[LAPIC_TIMER_HANDLER_IDT as usize].set_handler_addr(VirtAddr::new(lapic_timer_handler as u64));

    // ipis
    idt[IpiKind::Wakeup as usize].set_handler_addr(VirtAddr::new(ipi_wakeup_handler as u64));
    idt[IpiKind::Switch as usize].set_handler_addr(VirtAddr::new(ipi_switch_handler as u64));
    idt[IpiKind::Pit as usize].set_handler_addr(VirtAddr::new(ipi_pit_handler as u64));

    idt.load_unsafe();
    infohart!("interrupt descriptor table is initialized.")
}

pub unsafe fn write_idt_gate(cpu_id: LogicalCpuId, entry_index: usize, handler: u64) {
    let mut idt_list_guard = IDTS.write();
    let idt = idt_list_guard.get_mut(&cpu_id).unwrap();

    idt[entry_index].set_handler_addr(VirtAddr::new(handler));
}

extern "x86-interrupt" fn ipi_wakeup_handler(stack_frame: InterruptStackFrame) {
    unsafe { LOCAL_APIC.eoi() }
}

extern "x86-interrupt" fn ipi_switch_handler(stack_frame: InterruptStackFrame) {
    unsafe { LOCAL_APIC.eoi() }
}

extern "x86-interrupt" fn ipi_pit_handler(stack_frame: InterruptStackFrame) {
    unsafe { LOCAL_APIC.eoi() }
}

extern "x86-interrupt" fn lapic_timer_handler(_stack_frame: InterruptStackFrame) {
    unsafe { LOCAL_APIC.eoi() }
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    qemu_println!("EXCEPTION: BREAKPOINT {:?}", stack_frame);
}

extern "x86-interrupt" fn double_fault_handler(stack_frame: InterruptStackFrame, error_code: u64) -> ! {
    qemu_println!("EXCEPTION: DOUBLE FAULT code: {}, {:?}", error_code, stack_frame);
    exit_qemu(crate::device::qemu::QemuExitCode::Failed);
}

extern "x86-interrupt" fn page_fault_handler(stack_frame: InterruptStackFrame, error_code: PageFaultErrorCode) {
    qemu_println!("EXCEPTION: PAGE FAULT: access violation while reading 0x{:x}: {:?}\nstack: {:?}", Cr2::read(), error_code, stack_frame);
    halt();
}

extern "x86-interrupt" fn pit_stack(_stack_frame: InterruptStackFrame) {

}

#[test_case]
fn test_breakpoint_exception() {
    x86_64::instructions::interrupts::int3();
}