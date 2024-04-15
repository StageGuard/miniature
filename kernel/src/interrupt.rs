use alloc::{boxed::Box, collections::BTreeMap};
use alloc::sync::Arc;
use lazy_static::lazy_static;
use log::info;
use shared::{print_panic::PrintPanic};
use spin::{Mutex, RwLock, RwLockReadGuard};
use x86_64::{PhysAddr, registers::control::Cr2, structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode}, VirtAddr};
use core::{fmt::Write};
use core::arch::asm;
use core::hint::spin_loop;
use core::slice::from_raw_parts;
use x86_64::instructions::interrupts;
use x86_64::structures::paging::{PhysFrame, Size4KiB};
use x86_64::structures::paging::mapper::TranslateResult;

use crate::{acpi::local_apic::LOCAL_APIC, cpu::LogicalCpuId, device::qemu::exit_qemu, gdt::{pcr}, halt, infohart, interrupt, interrupt_error, interrupt_stack, mem::{frame_allocator::frame_alloc_n, PAGE_SIZE}, qemu_print, qemu_println};
use crate::arch_spec::port::inb;
use crate::ipi::IpiKind;
use crate::{push_preserved, push_scratch, pop_preserved, pop_scratch, swapgs_iff_ring3_fast, swapgs_iff_ring3_fast_errorcode, nop, conditional_swapgs_back_paranoid, conditional_swapgs_paranoid};
use crate::context::list::{context_storage, ContextStorage};

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
    idt.breakpoint.set_handler_addr(VirtAddr::new(breakpoint as u64))
        .set_present(true)
        .set_privilege_level(x86_64::PrivilegeLevel::Ring3);
    idt.double_fault.set_handler_addr(VirtAddr::new(double_fault as u64))
        .set_stack_index(dependent_ist.into());
    idt.page_fault.set_handler_addr(VirtAddr::new(page_fault as u64));

    idt.divide_error.set_handler_addr(VirtAddr::new(divide_error as u64));
    idt.debug.set_handler_addr(VirtAddr::new(debug as u64));
    idt.non_maskable_interrupt.set_handler_addr(VirtAddr::new(non_maskable_interrupt as u64));
    idt.overflow.set_handler_addr(VirtAddr::new(overflow as u64));
    idt.bound_range_exceeded.set_handler_addr(VirtAddr::new(bound_range_exceeded as u64));
    idt.invalid_opcode.set_handler_addr(VirtAddr::new(invalid_opcode as u64));
    idt.device_not_available.set_handler_addr(VirtAddr::new(device_not_available as u64));
    idt.hv_injection_exception.set_handler_addr(VirtAddr::new(hv_injection_exception as u64));
    idt.machine_check.set_handler_addr(VirtAddr::new(machine_check as u64));
    idt.simd_floating_point.set_handler_addr(VirtAddr::new(simd_floating_point as u64));
    idt.virtualization.set_handler_addr(VirtAddr::new(virtualization as u64));
    idt.x87_floating_point.set_handler_addr(VirtAddr::new(x87_floating_point as u64));
    idt.invalid_tss.set_handler_addr(VirtAddr::new(invalid_tss as u64));
    idt.segment_not_present.set_handler_addr(VirtAddr::new(segment_not_present as u64));
    idt.stack_segment_fault.set_handler_addr(VirtAddr::new(stack_segment_fault as u64));
    idt.general_protection_fault.set_handler_addr(VirtAddr::new(general_protection_fault as u64));
    idt.alignment_check.set_handler_addr(VirtAddr::new(alignment_check as u64));
    idt.cp_protection_exception.set_handler_addr(VirtAddr::new(cp_protection_exception as u64));
    idt.vmm_communication_exception.set_handler_addr(VirtAddr::new(vmm_communication_exception as u64));
    idt.security_exception.set_handler_addr(VirtAddr::new(security_exception as u64));

    // legacy irqs
    if cpu_id == LogicalCpuId::BSP {
        idt[32].set_handler_addr(VirtAddr::new(pit_stack as u64));
        idt[33].set_handler_addr(VirtAddr::new(keyboard as u64));
        idt[34].set_handler_addr(VirtAddr::new(cascade as u64));
        idt[35].set_handler_addr(VirtAddr::new(com2 as u64));
        idt[36].set_handler_addr(VirtAddr::new(com1 as u64));
        idt[37].set_handler_addr(VirtAddr::new(lpt2 as u64));
        idt[38].set_handler_addr(VirtAddr::new(floppy as u64));
        idt[39].set_handler_addr(VirtAddr::new(lpt1 as u64));
        idt[40].set_handler_addr(VirtAddr::new(rtc as u64));
        idt[41].set_handler_addr(VirtAddr::new(pci1 as u64));
        idt[42].set_handler_addr(VirtAddr::new(pci2 as u64));
        idt[43].set_handler_addr(VirtAddr::new(pci3 as u64));
        idt[44].set_handler_addr(VirtAddr::new(mouse as u64));
        idt[45].set_handler_addr(VirtAddr::new(fpu as u64));
        idt[46].set_handler_addr(VirtAddr::new(ata1 as u64));
        idt[47].set_handler_addr(VirtAddr::new(ata2 as u64));
        idt[LAPIC_TIMER_HANDLER_IDT as usize].set_handler_addr(VirtAddr::new(lapic_timer as u64));
    }
    idt[49].set_handler_addr(VirtAddr::new(lapic_error as u64));

    // ipis
    idt[IpiKind::Wakeup as usize].set_handler_addr(VirtAddr::new(ipi_wakeup as u64));
    idt[IpiKind::Switch as usize].set_handler_addr(VirtAddr::new(ipi_switch as u64));
    idt[IpiKind::Pit as usize].set_handler_addr(VirtAddr::new(ipi_pit as u64));

    idt.load_unsafe();
    infohart!("interrupt descriptor table is initialized.")
}

pub unsafe fn write_idt_gate(cpu_id: LogicalCpuId, entry_index: usize, handler: u64) {
    let mut idt_list_guard = IDTS.write();
    let idt = idt_list_guard.get_mut(&cpu_id).unwrap();

    idt[entry_index].set_handler_addr(VirtAddr::new(handler));
}

/// Set interrupts and halt
/// This will atomically wait for the next interrupt
/// Performing enable followed by halt is not guaranteed to be atomic, use this instead!
#[inline(always)]
pub unsafe fn enable_and_halt() {
    core::arch::asm!("sti; hlt", options(nomem, nostack));
}

/// Set interrupts and nop
/// This will enable interrupts and allow the IF flag to be processed
/// Simply enabling interrupts does not gurantee that they will trigger, use this instead!
#[inline(always)]
pub unsafe fn enable_and_nop() {
    core::arch::asm!("sti; nop", options(nomem, nostack));
}

// exceptions
interrupt_stack!(divide_error, |stack| { qemu_println!("divide_error: stack: {:?}", stack) });
interrupt_stack!(debug, @paranoid, |stack| { qemu_println!("debug: stack: {:?}", stack) });
interrupt_stack!(non_maskable_interrupt, @paranoid, |stack| { qemu_println!("non_maskable_interrupt: stack: {:?}", stack) });
interrupt_stack!(breakpoint, |stack| { qemu_println!("breakpoint: stack: {:?}", stack) });
interrupt_stack!(overflow, |stack| { qemu_println!("overflow: stack: {:?}", stack) });
interrupt_stack!(bound_range_exceeded, |stack| { qemu_println!("bound_range_exceeded: stack: {:?}", stack) });
interrupt_stack!(invalid_opcode, |stack| { qemu_println!("invalid_opcode: stack: {:?}", stack) });
interrupt_stack!(device_not_available, |stack| { qemu_println!("device_not_available: stack: {:?}", stack) });
interrupt_stack!(hv_injection_exception, |stack| { qemu_println!("hv_injection_exception: stack: {:?}", stack) });
interrupt_stack!(machine_check, |stack| { qemu_println!("machine_check: stack: {:?}", stack) });
interrupt_stack!(simd_floating_point, |stack| { qemu_println!("simd_floating_point: stack: {:?}", stack) });
interrupt_stack!(virtualization, |stack| { qemu_println!("virtualization: stack: {:?}", stack) });
interrupt_stack!(x87_floating_point, |stack| { qemu_println!("x87_floating_point: stack: {:?}", stack) });
interrupt_stack!(cp_protection_exception, |stack| { qemu_println!("page_fault, stack: {:?}", stack) });
interrupt_stack!(vmm_communication_exception, |stack| { qemu_println!("page_fault, stack: {:?}", stack) });

interrupt_error!(page_fault, |stack, code| {
    let slice = from_raw_parts((stack.iret.rsp - 0x48) as *const u8, 0x48usize);
    qemu_println!("calle stacks: {:02x?}", slice);

    qemu_println!("page_fault: reading {:x}: {}, stack: {:?}", Cr2::read().as_u64(), code, stack);
    loop { spin_loop() }
});
interrupt_error!(invalid_tss, |stack, code| { qemu_println!("invalid_tss: {}, stack: {:?}", code, stack) });
interrupt_error!(double_fault, |stack, code| { qemu_println!("double_fault: {}, stack: {:?}", code, stack) });
interrupt_error!(segment_not_present, |stack, code| { qemu_println!("segment_not_present: {}, stack: {:?}", code, stack) });
interrupt_error!(stack_segment_fault, |stack, code| { qemu_println!("stack_segment_fault: {}, stack: {:?}", code, stack) });
interrupt_error!(general_protection_fault, |stack, code| { qemu_println!("general_protection_fault: {}, stack: {:?}", code, stack) });
interrupt_error!(alignment_check, |stack, code| { qemu_println!("alignment_check: {}, stack: {:?}", code, stack) });
interrupt_error!(security_exception, |stack, code| { qemu_println!("security_exception: {}, stack: {:?}", code, stack) });

// legacy irqs
interrupt!(pit_stack, || { LOCAL_APIC.eoi() });
interrupt!(keyboard, || {
    use pc_keyboard::{layouts, DecodedKey, HandleControl, Keyboard, ScancodeSet1};
    use spin::Mutex;

    lazy_static! {
        static ref KB: Mutex<Keyboard<layouts::Us104Key, ScancodeSet1>> =
            Mutex::new(Keyboard::new(ScancodeSet1::new(), layouts::Us104Key, HandleControl::Ignore));
    };

    let data: u8 = inb(0x60);
    LOCAL_APIC.eoi();

    let mut keyboard = KB.lock();
    if let Ok(Some(key_event)) = keyboard.add_byte(data) {
        if let Some(key) = keyboard.process_keyevent(key_event) {
            match key {
                DecodedKey::Unicode(character) => qemu_print!("{}", character),
                DecodedKey::RawKey(key) => qemu_print!("{:?}", key),
            }
        }
    }
});
interrupt!(cascade, || { LOCAL_APIC.eoi() });
interrupt!(com2, || { LOCAL_APIC.eoi() });
interrupt!(com1, || { LOCAL_APIC.eoi() });
interrupt!(lpt2, || { LOCAL_APIC.eoi() });
interrupt!(floppy, || { LOCAL_APIC.eoi() });
interrupt!(lpt1, || { LOCAL_APIC.eoi() });
interrupt!(rtc, || { LOCAL_APIC.eoi() });
interrupt!(pci1, || { LOCAL_APIC.eoi() });
interrupt!(pci2, || { LOCAL_APIC.eoi() });
interrupt!(pci3, || { LOCAL_APIC.eoi() });
interrupt!(mouse, || { LOCAL_APIC.eoi() });
interrupt!(fpu, || { LOCAL_APIC.eoi() });
interrupt!(ata1, || { LOCAL_APIC.eoi() });
interrupt!(ata2, || { LOCAL_APIC.eoi() });
interrupt!(lapic_timer, || { LOCAL_APIC.eoi() });
interrupt!(lapic_error, || { });

// ipis
interrupt!(ipi_wakeup, || {
    infohart!("ipi wakeup");
    LOCAL_APIC.eoi()
});
interrupt!(ipi_switch, || { LOCAL_APIC.eoi() });
interrupt!(ipi_pit, || { LOCAL_APIC.eoi() });


#[test_case]
fn test_breakpoint_exception() {
    x86_64::instructions::interrupts::int3();
}