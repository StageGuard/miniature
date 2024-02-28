use lazy_static::lazy_static;
use shared::uni_processor::UPSafeCell;
use x86_64::{structures::idt::{InterruptDescriptorTable, InterruptStackFrame}, VirtAddr};
use core::{borrow::Borrow, fmt::Write};

use crate::{device::qemu::exit_qemu, gdt, qemu_println};

lazy_static! {
    static ref IDT: UPSafeCell<InterruptDescriptorTable> = unsafe { UPSafeCell::new(InterruptDescriptorTable::new()) };
}

pub unsafe fn init_idt() {
    let mut idt = IDT.inner_exclusive_mut();

    idt.breakpoint.set_handler_addr(VirtAddr::new(breakpoint_handler as u64));
    idt.double_fault.set_handler_addr(VirtAddr::new(double_fault_handler as u64)).set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
    idt.load_unsafe();
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    qemu_println!("EXCEPTION: BREAKPOINT {:?}", stack_frame);
}

extern "x86-interrupt" fn double_fault_handler(stack_frame: InterruptStackFrame, error_code: u64) -> ! {
    qemu_println!("EXCEPTION: DOUBLE FAULT code: {}, {:?}", error_code, stack_frame);
    exit_qemu(crate::device::qemu::QemuExitCode::Failed);
}

#[test_case]
fn test_breakpoint_exception() {
    x86_64::instructions::interrupts::int3();
}