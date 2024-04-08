use alloc::{boxed::Box, collections::BTreeMap};
use lazy_static::lazy_static;
use log::info;
use shared::{print_panic::PrintPanic};
use spin::{Mutex, RwLock};
use x86_64::{registers::control::Cr2, structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode}, VirtAddr};
use core::{fmt::Write};

use crate::{acpi::local_apic::LOCAL_APIC, cpu::LogicalCpuId, device::qemu::exit_qemu, gdt::{pcr}, halt, infohart, mem::{frame_allocator::frame_alloc_n, PAGE_SIZE}, qemu_print, qemu_println};
use crate::acpi::local_apic::IpiKind;
use crate::arch_spec::port::inb;

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

macro_rules! interrupt {
    ($name:ident) => {
        extern "x86-interrupt" fn $name(stack_frame: InterruptStackFrame) -> ! {
            qemu_println!("EXCEPTION: {}, {:?}",stringify!($name), stack_frame);
            halt();
        }
    };
    ($name:ident, @err_code) => {
        extern "x86-interrupt" fn $name(stack_frame: InterruptStackFrame, error_code: u64) -> ! {
            qemu_println!("EXCEPTION: {} code: {}, {:?}",stringify!($name), error_code, stack_frame);
            halt();
        }
    };
    ($name:ident, !, |$stack_frame:ident $(, $error_code:ident)?| $code:block) => {
        extern "x86-interrupt" fn $name($stack_frame: InterruptStackFrame$(, $error_code: u64)?) -> ! {
            unsafe { $code }
        }
    };
    ($name:ident, |$stack_frame:ident $(, $error_code:ident)?| $code:block) => {
        extern "x86-interrupt" fn $name($stack_frame: InterruptStackFrame$(, $error_code: u64)?) {
            unsafe { $code };
        }
    };
}

// exceptions
interrupt!(divide_error);
interrupt!(debug);
interrupt!(non_maskable_interrupt);
interrupt!(breakpoint);
interrupt!(overflow);
interrupt!(bound_range_exceeded);
interrupt!(invalid_opcode);
interrupt!(device_not_available);
interrupt!(hv_injection_exception);
interrupt!(machine_check);
interrupt!(simd_floating_point);
interrupt!(virtualization);
interrupt!(x87_floating_point);
interrupt!(page_fault, !, |stack, err_code| {
    qemu_println!("EXCEPTION: PAGE FAULT: access violation while reading 0x{:x}: {:?}\nstack: {:?}", Cr2::read(), stack, err_code);
    halt();
});
interrupt!(invalid_tss, @err_code);
interrupt!(double_fault, @err_code);
interrupt!(segment_not_present, @err_code);
interrupt!(stack_segment_fault, @err_code);
interrupt!(general_protection_fault, @err_code);
interrupt!(alignment_check, @err_code);
interrupt!(cp_protection_exception, @err_code);
interrupt!(vmm_communication_exception, @err_code);
interrupt!(security_exception, @err_code);

// legacy irqs
interrupt!(pit_stack, |stack| { LOCAL_APIC.eoi() });
interrupt!(keyboard, |stack| {
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
interrupt!(cascade, |stack| { LOCAL_APIC.eoi() });
interrupt!(com2, |stack| { LOCAL_APIC.eoi() });
interrupt!(com1, |stack| { LOCAL_APIC.eoi() });
interrupt!(lpt2, |stack| { LOCAL_APIC.eoi() });
interrupt!(floppy, |stack| { LOCAL_APIC.eoi() });
interrupt!(lpt1, |stack| { LOCAL_APIC.eoi() });
interrupt!(rtc, |stack| { LOCAL_APIC.eoi() });
interrupt!(pci1, |stack| { LOCAL_APIC.eoi() });
interrupt!(pci2, |stack| { LOCAL_APIC.eoi() });
interrupt!(pci3, |stack| { LOCAL_APIC.eoi() });
interrupt!(mouse, |stack| { LOCAL_APIC.eoi() });
interrupt!(fpu, |stack| { LOCAL_APIC.eoi() });
interrupt!(ata1, |stack| { LOCAL_APIC.eoi() });
interrupt!(ata2, |stack| { LOCAL_APIC.eoi() });
interrupt!(lapic_timer, |stack| { LOCAL_APIC.eoi() });
interrupt!(lapic_error, |stack| { });

// ipis
interrupt!(ipi_wakeup, |stack| { LOCAL_APIC.eoi() });
interrupt!(ipi_switch, |stack| { LOCAL_APIC.eoi() });
interrupt!(ipi_pit, |stack| { LOCAL_APIC.eoi() });


#[test_case]
fn test_breakpoint_exception() {
    x86_64::instructions::interrupts::int3();
}