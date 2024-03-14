use core::mem::MaybeUninit;

use lazy_static::lazy_static;
use log::info;
use shared::uni_processor::UPSafeCell;
use x86_64::{instructions::{self, interrupts, tables::load_tss}, registers::control::{Cr0, Cr0Flags}, structures::{gdt::{Descriptor, GlobalDescriptorTable}, tss::TaskStateSegment}, VirtAddr};

const STACK_SIZE: usize = 10 * 0x1000; // 10 KiB
pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

static mut DOUBLE_FAULT_STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];

lazy_static! {
    static ref TSS: TaskStateSegment = unsafe {
        let mut r = TaskStateSegment::new();
        r.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = VirtAddr::new(DOUBLE_FAULT_STACK.as_ptr() as u64) + STACK_SIZE;
        r
    };

    static ref GDT_PTR: UPSafeCell<MaybeUninit<u64>> = unsafe { UPSafeCell::new(MaybeUninit::uninit()) };
}

pub unsafe fn init_gdt_and_protected_mode(gdt_virt_addr: u64) {
    let mut gdt_ptr = GDT_PTR.inner_exclusive_mut();
    gdt_ptr.write(gdt_virt_addr);

    let gdt = &mut *(gdt_virt_addr as *mut GlobalDescriptorTable);
    let tss_selector = gdt.add_entry(Descriptor::tss_segment(&TSS));

    interrupts::disable();
    gdt.load_unsafe();
    Cr0::update(|cr0| *cr0 |= Cr0Flags::PROTECTED_MODE_ENABLE);

    load_tss(tss_selector);
}