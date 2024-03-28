use core::ptr::{read_volatile, write_volatile};
use core::fmt::Write;
use log::info;
use x86_64::{instructions::port::{Port, PortGeneric, ReadWriteAccess}, registers::model_specific::Msr, structures::idt::InterruptStackFrame};

use crate::{arch_spec::msr::{rdmsr, wrmsr}, cpuid::cpuid, device::port::{inb, outb}, interrupt::write_idt_gate, qemu_println};


const IA32_APIC_BASE_MSR: u32 = 0x1B;
const IA32_APIC_BASE_MSR_ENABLE: u64 = 0x800;

pub static mut LOCAL_APIC: LocalApic = LocalApic {
    base: 0,
    x2: false,
};

#[derive(Clone, Copy)]
struct LocalApic {
    base: u64,
    x2: bool,
}

impl LocalApic {
    fn init(&mut self, base: u64, x2: bool) {
        self.base = base;
        self.x2 = x2;
    }

    unsafe fn read(&self, reg: u32) -> u32 {
        read_volatile(((self.base & 0xffffffff) as u32 + reg) as *const u32)
    }

    unsafe fn write(&mut self, reg: u32, value: u32) {
        write_volatile(((self.base & 0xffffffff) as u32 + reg) as *mut u32, value);
    }

    
    pub fn id(&self) -> u32 {
        if self.x2 {
            unsafe { rdmsr(0x802) as u32 }
        } else {
            unsafe { self.read(0x20) }
        }
    }

    pub fn version(&self) -> u32 {
        if self.x2 {
            unsafe { rdmsr(0x803) as u32 }
        } else {
            unsafe { self.read(0x30) }
        }
    }

    pub fn icr(&self) -> u64 {
        if self.x2 {
            unsafe { rdmsr(0x830) }
        } else {
            unsafe { (self.read(0x310) as u64) << 32 | self.read(0x300) as u64 }
        }
    }

    pub fn set_icr(&mut self, value: u64) {
        if self.x2 {
            unsafe {
                wrmsr(0x830, value);
            }
        } else {
            unsafe {
                const PENDING: u32 = 1 << 12;
                while self.read(0x300) & PENDING == PENDING {
                    core::hint::spin_loop();
                }
                self.write(0x310, (value >> 32) as u32);
                self.write(0x300, value as u32);
                while self.read(0x300) & PENDING == PENDING {
                    core::hint::spin_loop();
                }
            }
        }
    }

    pub fn ipi(&mut self, apic_id: u32, kind: IpiKind) {
        let mut icr = 0x40 | kind as u64;
        if self.x2 {
            icr |= u64::from(apic_id) << 32;
        } else {
            icr |= u64::from(apic_id) << 56;
        }
        self.set_icr(icr);
    }
    pub fn ipi_nmi(&mut self, apic_id: u32) {
        let shift = if self.x2 { 32 } else { 56 };
        self.set_icr((u64::from(apic_id) << shift) | (1 << 14) | (0b100 << 8));
    }

    pub unsafe fn eoi(&mut self) {
        if self.x2 {
            wrmsr(0x80B, 0);
        } else {
            self.write(0xB0, 0);
        }
    }
    /// Reads the Error Status Register.
    pub unsafe fn esr(&mut self) -> u32 {
        if self.x2 {
            // update the ESR to the current state of the local apic.
            wrmsr(0x828, 0);
            // read the updated value
            rdmsr(0x828) as u32
        } else {
            self.write(0x280, 0);
            self.read(0x280)
        }
    }
    pub unsafe fn lvt_timer(&mut self) -> u32 {
        if self.x2 {
            rdmsr(0x832) as u32
        } else {
            self.read(0x320)
        }
    }
    pub unsafe fn set_lvt_timer(&mut self, value: u32) {
        if self.x2 {
            wrmsr(0x832, u64::from(value));
        } else {
            self.write(0x320, value);
        }
    }
    pub unsafe fn init_count(&mut self) -> u32 {
        if self.x2 {
            rdmsr(0x838) as u32
        } else {
            self.read(0x380)
        }
    }
    pub unsafe fn set_init_count(&mut self, initial_count: u32) {
        if self.x2 {
            wrmsr(0x838, u64::from(initial_count));
        } else {
            self.write(0x380, initial_count);
        }
    }
    pub unsafe fn cur_count(&mut self) -> u32 {
        if self.x2 {
            rdmsr(0x839) as u32
        } else {
            self.read(0x390)
        }
    }
    pub unsafe fn div_conf(&mut self) -> u32 {
        if self.x2 {
            rdmsr(0x83E) as u32
        } else {
            self.read(0x3E0)
        }
    }
    pub unsafe fn set_div_conf(&mut self, div_conf: u32) {
        if self.x2 {
            wrmsr(0x83E, u64::from(div_conf));
        } else {
            self.write(0x3E0, div_conf);
        }
    }
    pub unsafe fn lvt_error(&mut self) -> u32 {
        if self.x2 {
            rdmsr(0x837) as u32
        } else {
            self.read(0x370)
        }
    }
    pub unsafe fn set_lvt_error(&mut self, lvt_error: u32) {
        if self.x2 {
            wrmsr(0x837, u64::from(lvt_error));
        } else {
            self.write(0x370, lvt_error);
        }
    }
    unsafe fn setup_error_int(&mut self) {
        let vector = 49u32;
        self.set_lvt_error(vector);
    }
}

extern "x86-interrupt" fn isr_timer_handler(stack_frame: InterruptStackFrame) {

    // reset apic EOI register to proceed interrupt.
    unsafe { write_volatile(0xfee000b0 as *mut u32, 0); }
}

extern "x86-interrupt" fn isr_spurious_handler(stack_frame: InterruptStackFrame) { }

extern "x86-interrupt" fn isr_lint0_handler(stack_frame: InterruptStackFrame) {
    qemu_println!("LINT0 interrupt handled. {:?}", stack_frame);
    // reset apic EOI register to proceed interrupt.
    unsafe { write_volatile(0xfee000b0 as *mut u32, 0); }
}

extern "x86-interrupt" fn isr_lint1_handler(stack_frame: InterruptStackFrame) {
    qemu_println!("LINT1 interrupt handled. {:?}", stack_frame);
    // reset apic EOI register to proceed interrupt.
    unsafe { write_volatile(0xfee000b0 as *mut u32, 0); }
}

/**
 * https://wiki.osdev.org/APIC_timer#Enabling_APIC_Timer
 */
pub unsafe fn setup_apic(apic_base: u64) {
    // Hardware enable the Local APIC if it wasn't enabled
    let mut msr = Msr::new(IA32_APIC_BASE_MSR);
    msr.write(apic_base | IA32_APIC_BASE_MSR_ENABLE);

    LOCAL_APIC.init(apic_base & 0xffff0000, cpuid()
        .get_feature_info()
        .map_or(false, |feature_info| feature_info.has_x2apic()));

    // disable 8259 PIC
    outb(0x21, 0xff);
    outb(0xa1, 0xff);

    // setup isrs
    write_idt_gate(32, isr_timer_handler as u64);
    write_idt_gate(33, isr_lint0_handler as u64);
    write_idt_gate(34, isr_lint1_handler as u64);
    write_idt_gate(39, isr_spurious_handler as u64);

    // initialize LAPIC to a well known state
    // flat mode 
    LOCAL_APIC.write(0xe0, 0xffffffff); // Destination Format Register
    LOCAL_APIC.write(0xd0, (LOCAL_APIC.read(0xd0) & 0xffffff) | 1); // Logical Destination Register
    //clear lvt
    LOCAL_APIC.write(0x320, 0x10000); // LVT Timer Register
    LOCAL_APIC.write(0x340, 4 << 8); // LVT Performance Monitoring Counters Register
    LOCAL_APIC.write(0x350, 0x10000); // LVT LINT0 Register
    LOCAL_APIC.write(0x360, 0x10000); // LVT LINT1 Register
    // clear TPR, receiving all interrupts
    LOCAL_APIC.write(0x80, 0); // Task Priority Register


    // software enable, map spurious interrupt to dummy isr
    LOCAL_APIC.write(0xf0, LOCAL_APIC.read(0xf0) | 0x100); // Spurious Interrupt Vector Register


    // map APIC timer to an interrupt, and by that enable it in one-shot mode
    LOCAL_APIC.write(0x320, 0x20); // LVT Timer Register
    // set up divide value to 1
    LOCAL_APIC.write(0x3e0, 0xb); // Divide Configuration Register

    // initialize PIT Ch 2 in one-shot mode
    // PIT has fixed frequency 1193182 Hz, so let PIT ch2 tick 10ms.
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
    LOCAL_APIC.write(0x380, 0xffffffff /* = -1 */); // Initial Count Register (for Timer)

    // wait until PIT counter reaches 0
    let mut port_pit2_gate: PortGeneric<u8, ReadWriteAccess> = Port::new(0x61);
    while port_pit2_gate.read() & 0x20 == 0 { }
    // stop APIC timer
    LOCAL_APIC.write(0x320, 0x10000); // LVT Timer Register

    // 0x390 = Current Count Register (for Timer)
    let lapic_ticks_in_10_ms: u32 = 0xffffffff - LOCAL_APIC.read(0x390);

    // apply freq
    // 0x2000 = periodic mode
    LOCAL_APIC.write(0x320, 0x20 | 0x20000); // LVT Timer Register
    LOCAL_APIC.write(0x3e0, 0xb); // Divide Configuration Register
    // let apic timer send irq every 1ms
    LOCAL_APIC.write(0x380, lapic_ticks_in_10_ms / 10); // Initial Count Register (for Timer)

    info!("LAPIC initialized, CPU bus frequency: {:} Hz", lapic_ticks_in_10_ms * 100);
}

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum IpiKind {
    Wakeup = 0x40,
    Tlb = 0x41,
    Switch = 0x42,
    Pit = 0x43
}