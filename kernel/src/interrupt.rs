use lazy_static::lazy_static;
use shared::uni_processor::UPSafeCell;
use x86_64::{registers::control::Cr2, structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode}, VirtAddr};
use core::{borrow::Borrow, fmt::Write};

use crate::{device::qemu::exit_qemu, gdt, halt, qemu_println};

lazy_static! {
    static ref IDT: UPSafeCell<InterruptDescriptorTable> = unsafe { UPSafeCell::new(InterruptDescriptorTable::new()) };
}

pub unsafe fn init_idt() {
    let mut idt = IDT.inner_exclusive_mut();

    idt.breakpoint.set_handler_addr(VirtAddr::new(breakpoint_handler as u64));
    idt.double_fault.set_handler_addr(VirtAddr::new(double_fault_handler as u64)).set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
    idt.page_fault.set_handler_addr(VirtAddr::new(page_fault_handler as u64));
    idt.load_unsafe();
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

pub unsafe fn write_idt_gate(entry_index: usize, handler: u64) {
    let mut idt = IDT.inner_exclusive_mut();
    idt[entry_index].set_handler_addr(VirtAddr::new(handler));
}

#[test_case]
fn test_breakpoint_exception() {
    x86_64::instructions::interrupts::int3();
}