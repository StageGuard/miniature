use core::mem::{size_of, MaybeUninit};

use lazy_static::lazy_static;
use log::info;
use shared::uni_processor::UPSafeCell;
use x86_64::{instructions::{self, interrupts, tables::load_tss}, registers::{control::{Cr0, Cr0Flags}, segmentation::{Segment, CS, DS, ES, GS, SS}}, structures::{gdt::{Descriptor, DescriptorFlags, GlobalDescriptorTable, SegmentSelector}, tss::TaskStateSegment}, VirtAddr};

use crate::{arch_spec::msr::wrmsr, mem::{frame_allocator::{frame_alloc, frame_alloc_n}, PAGE_SIZE}};

const STACK_SIZE: usize = 10 * 0x1000; // 10 KiB
pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;
const IOBITMAP_SIZE: u32 = 65536 / 8;

// TODO: each cpu should has its own interrupt stack
static mut DOUBLE_FAULT_STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];

#[repr(C, align(4096))]
pub struct ProcessorControlRegion {
    pub self_ref: usize,

    pub user_rsp_tmp: usize,
    pub gdt: GlobalDescriptorTable,
    pub _percpu: (),
    _rsvd: Align,
    pub tss: TaskStateSegment,

    // These two fields are read by the CPU, but not currently modified by the kernel. Instead, the
    // kernel sets the `iom ap_base` field in the TSS, to either point to this bitmap, or outside
    // the TSS, in which case userspace is not granted port IO access.
    pub _iobitmap: [u8; IOBITMAP_SIZE as usize],
    pub _all_ones: u8,
}

#[repr(C, align(16))]
struct Align([usize; 2]);


// from redox-os kernel
pub unsafe fn init_gdt(kernel_stack_top: u64) {
    interrupts::disable();

    let pcr = &mut *(frame_alloc_n(size_of::<ProcessorControlRegion>().div_ceil(PAGE_SIZE))
        .expect("failed to allocate phys farme for ProcessorControlRegion")
        .start_address().as_u64() as *mut ProcessorControlRegion);

    pcr.self_ref = pcr as *mut ProcessorControlRegion as usize;
    pcr.gdt = GlobalDescriptorTable::new();

    pcr.tss = TaskStateSegment::new();
    pcr.tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = VirtAddr::new(DOUBLE_FAULT_STACK.as_ptr() as u64) + STACK_SIZE;
    pcr.tss.privilege_stack_table[0] = VirtAddr::new(kernel_stack_top);

    pcr.tss.iomap_base = 0xffff;
    pcr._all_ones = 0xff;

    // GDT[0] = NULL
    let code_selector = pcr.gdt.add_entry(Descriptor::kernel_code_segment()); // GDT[1] = KERNEL_CODE,
    let data_selector = pcr.gdt.add_entry(Descriptor::kernel_data_segment()); // GDT[2] = KERNEL_DATA
    pcr.gdt.add_entry(Descriptor::UserSegment(DescriptorFlags::KERNEL_CODE32.bits() | DescriptorFlags::DPL_RING_3.bits())); // GDT[3] = USER_CODE_32
    pcr.gdt.add_entry(Descriptor::user_code_segment()); // GDT[4] = USER_CODE
    pcr.gdt.add_entry(Descriptor::user_data_segment()); // GDT[5] = USER_DATE
    let tss_selector = pcr.gdt.add_entry(Descriptor::tss_segment(&pcr.tss)); // GDT[6.=7] = TSS


    pcr.gdt.load_unsafe();
    
    unsafe {
        CS::set_reg(code_selector);
        SS::set_reg(data_selector);
        DS::set_reg(SegmentSelector(0));
        ES::set_reg(SegmentSelector(0));
        GS::set_reg(SegmentSelector(0));
    }
    
    wrmsr(0xc0000101, pcr as *const _ as usize as u64); // IA32_GS_BASE
    wrmsr(0xc0000102, 0); // IA32_KERNEL_GSBASE
    wrmsr(0xc0000100, 0); // IA32_FS_BASE

    load_tss(tss_selector);

    Cr0::update(|cr0| *cr0 |= Cr0Flags::PROTECTED_MODE_ENABLE);

}