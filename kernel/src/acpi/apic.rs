use core::{mem::MaybeUninit, ptr::{read_volatile, write_volatile}};

use lazy_static::lazy_static;
use log::info;
use shared::uni_processor::UPSafeCell;
use x86_64::{instructions::{interrupts, port::{Port, PortGeneric, ReadWriteAccess}}, registers::model_specific::Msr, structures::idt::InterruptStackFrame};

use crate::interrupt::write_idt_gate;


const IA32_APIC_BASE_MSR: u32 = 0x1B;
const IA32_APIC_BASE_MSR_ENABLE: u64 = 0x800;

lazy_static! {
    static ref LAPIC_ACCESSOR: UPSafeCell<MaybeUninit<LApicAccessor>> = unsafe { UPSafeCell::new(MaybeUninit::uninit()) };
}

#[derive(Clone, Copy)]
struct LApicAccessor(u64);

impl LApicAccessor {
    unsafe fn read(&self, reg: u32) -> u32 {
        read_volatile(((self.0 & 0xffffffff) as u32 + reg) as *const u32)
    }

    unsafe fn write(&mut self, reg: u32, value: u32) {
        write_volatile(((self.0 & 0xffffffff) as u32 + reg) as *mut u32, value);
    }
}

extern "x86-interrupt" fn isr_dummytmr_handler(stack_frame: InterruptStackFrame) {
    unsafe {
        let mut apic_ref = LAPIC_ACCESSOR.inner_exclusive_mut();
        let mut apic = apic_ref.assume_init_mut().clone();
        drop(apic_ref);

        apic.write(0xb0, 0); // EOI Register
        info!("isr_dummytmr_handler")
    }
}

extern "x86-interrupt" fn isr_spurious_handler(stack_frame: InterruptStackFrame) { 
    info!("isr_dummytmr_handler")
}

/**
 * https://wiki.osdev.org/APIC_timer#Enabling_APIC_Timer
 */
pub unsafe fn setup_apic(apic_base: u64) {
    // Hardware enable the Local APIC if it wasn't enabled
    let mut msr = Msr::new(IA32_APIC_BASE_MSR);
    msr.write(apic_base | IA32_APIC_BASE_MSR_ENABLE);

    let mut apic = LApicAccessor(apic_base & 0xffff0000);

    let mut apic_ref = LAPIC_ACCESSOR.inner_exclusive_mut();
    apic_ref.write(apic.clone());
    drop(apic_ref);

    // disable 8259 PIC
    outb(0x21, 0xff);
    outb(0xa1, 0xff);

    // setup isrs
    write_idt_gate(32, isr_dummytmr_handler as u64);
    write_idt_gate(39, isr_spurious_handler as u64);

    // initialize LAPIC to a well known state
    // flat mode 
    apic.write(0xe0, 0xffffffff); // Destination Format Register
    apic.write(0xd0, (apic.read(0xd0) & 0xffffff) | 1); // Logical Destination Register
    //clear lvt
    apic.write(0x320, 0x10000); // LVT Timer Register
    apic.write(0x340, 4 << 8); // LVT Performance Monitoring Counters Register
    apic.write(0x350, 0x10000); // LVT LINT0 Register
    apic.write(0x360, 0x10000); // LVT LINT1 Register
    // clear TPR, receiving all interrupts
    apic.write(0x80, 0); // Task Priority Register


    // software enable, map spurious interrupt to dummy isr
    apic.write(0xf0, apic.read(0xf0) | 0x100); // Spurious Interrupt Vector Register


    // map APIC timer to an interrupt, and by that enable it in one-shot mode
    apic.write(0x320, 0x20); // LVT Timer Register
    // set up divide value to 16
    apic.write(0x3e0, 0b111); // Divide Configuration Register

    // initialize PIT Ch 2 in one-shot mode
    // waiting 1 sec could slow down boot time considerably,
    // so we'll wait 1/100 sec, and multiply the counted ticks
    outb(0x61, (inb(0x61) & 0xfd) | 1);
    outb(0x43, 0b10110010);

    const FREQ: u32 = 1193182 / 100;

    outb(0x42, (FREQ & 0xff) as u8);
    inb(0x60);
    outb(0x42, ((FREQ >> 8) & 0xff) as u8);

    // reset PIT one-shot counter (start counting)
    let pit2_gate = inb(0x61) & 0xfe;
    outb(0x61, pit2_gate); // gate low
    outb(0x61, pit2_gate | 1); // gate high

    // reset APIC timer
    apic.write(0x380, 0xffffffff /* = -1 */); // Initial Count Register (for Timer)

    // wait until PIT counter reaches 0
    let mut port_pit2_gate: PortGeneric<u8, ReadWriteAccess> = Port::new(0x61);
    while port_pit2_gate.read() & 0x20 == 0 { }
    // stop APIC timer
    apic.write(0x320, 0x10000); // LVT Timer Register

    // 0x390 = Current Count Register (for Timer)
    let ccr = apic.read(0x390);
    let freq: u32 = 0xffffffff - ccr;
    info!("freq: {:} Hz", freq * 100);

    // apply freq
    // 0x2000 = periodic mode
    apic.write(0x320, 0x20 | 0x20000); // LVT Timer Register
    apic.write(0x3e0, 0b111); // Divide Configuration Register
    apic.write(0x380, freq); // Initial Count Register (for Timer)
}



#[inline]
unsafe fn inb(port: u16) -> u8 {
    Port::new(port).read()
}

#[inline]
unsafe fn outb(port: u16, value: u8) {
    Port::new(port).write(value)
}