use core::arch::asm;
use core::mem::offset_of;
use log::info;
use x86_64::PrivilegeLevel::Ring3;
use x86_64::registers::rflags::RFlags;
use x86_64::registers::segmentation::SegmentSelector;
use x86_64::structures::tss::TaskStateSegment;
use crate::arch_spec::msr::{rdmsr, wrmsr};
use crate::gdt::{pcr, ProcessorControlRegion};
use crate::infohart;

/*
 https://gitlab.redox-os.org/redox-os/kernel/-/blob/master/src/arch/x86_64/interrupt/syscall.rs
 */

#[derive(Default)]
#[repr(C)]
pub struct InterruptStack {
    pub preserved: PreservedRegisters,
    pub scratch: ScratchRegisters,
    pub iret: IretRegisters,
}

#[derive(Default)]
#[repr(C)]
pub struct PreservedRegisters {
    pub r15: usize,
    pub r14: usize,
    pub r13: usize,
    pub r12: usize,
    pub rbp: usize,
    pub rbx: usize,
}

#[derive(Default)]
#[repr(C)]
pub struct ScratchRegisters {
    pub r11: usize,
    pub r10: usize,
    pub r9: usize,
    pub r8: usize,
    pub rsi: usize,
    pub rdi: usize,
    pub rdx: usize,
    pub rcx: usize,
    pub rax: usize,
}

#[derive(Default)]
#[repr(C)]
pub struct IretRegisters {
    pub rip: usize,
    pub cs: usize,
    pub rflags: usize,

    // In x86 Protected Mode, i.e. 32-bit kernels, the following two registers are conditionally
    // pushed if the privilege ring changes. In x86 Long Mode however, i.e. 64-bit kernels, they
    // are unconditionally pushed, mostly due to stack alignment requirements.
    pub rsp: usize,
    pub ss: usize,
}

impl InterruptStack {
    pub fn init(&mut self) {
        // Always enable interrupts!
        self.iret.rflags = RFlags::INTERRUPT_FLAG.bits() as usize;
        self.iret.cs = (4 << 3) | 3; // GDT[4] = GDT_USER_CODE
        self.iret.ss = (5 << 3) | 3; // GDT[5] = GDT_USER_DATA
    }
    pub fn set_stack_pointer(&mut self, rsp: usize) {
        self.iret.rsp = rsp;
    }
    pub fn stack_pointer(&self) -> usize {
        self.iret.rsp
    }
    pub fn set_instr_pointer(&mut self, rip: usize) {
        self.iret.rip = rip;
    }
    // TODO: This can maybe be done in userspace?
    pub fn set_syscall_ret_reg(&mut self, ret: usize) {
        self.scratch.rax = ret;
    }
}

#[no_mangle]
pub unsafe extern "C" fn __inner_syscall_instruction(stack: *mut InterruptStack) {
    let stack_ref = &mut *stack;

    let args = [
        &stack_ref.scratch.rax,
        &stack_ref.scratch.rdi,
        &stack_ref.scratch.rsi,
        &stack_ref.scratch.rdx,
        &stack_ref.scratch.r10,
        &stack_ref.scratch.r8
    ];

    infohart!("syscall_module: arg = {:?}", args);

    stack_ref.set_syscall_ret_reg(0);
}

#[naked]
#[allow(named_asm_labels)]
pub unsafe extern "C" fn syscall_instruction() {
    asm!(concat!(
        "swapgs;",                    // Swap KGSBASE with GSBASE, allowing fast TSS access.
        "mov gs:[{sp}], rsp;",        // Save userspace stack pointer
        "mov rsp, gs:[{ksp}];",       // Load kernel stack pointer
        "push QWORD PTR {ss_sel};",   // Push fake userspace SS (resembling iret frame)
        "push QWORD PTR gs:[{sp}];",  // Push userspace rsp
        "push r11;",                  // Push rflags
        "push QWORD PTR {cs_sel};",   // Push fake CS (resembling iret stack frame)
        "push rcx;",                  // Push userspace return pointer

        // Push scratch registers
        "push rax;",
        "push rcx;",
        "push rdx;",
        "push rdi;",
        "push rsi;",
        "push r8;",
        "push r9;",
        "push r10;",
        "push r11;",
        // Push preserved registers
        "push rbx;",
        "push rbp;",
        "push r12;",
        "push r13;",
        "push r14;",
        "push r15;",

        // Call inner function
        "mov rdi, rsp;",
        "call __inner_syscall_instruction;",

        "
    .globl enter_usermode
        enter_usermode:
        ",

        // Pop preserved registers
        "pop r15;",
        "pop r14;",
        "pop r13;",
        "pop r12;",
        "pop rbp;",
        "pop rbx;",
        // Pop scratch registers,
        "pop r11;",
        "pop r10;",
        "pop r9;",
        "pop r8;",
        "pop rsi;",
        "pop rdi;",
        "pop rdx;",
        "pop rcx;",
        "pop rax;",

        "swapgs;",

        // check trap flag
        "test BYTE PTR [rsp + 17], 1;",
        // If set, return using IRETQ instead.
        "jnz 1f;",

        // Otherwise, continue with the fast sysretq.

        // Pop userspace return pointer
        "pop rcx;",

        // We must ensure RCX is canonical; if it is not when running sysretq, the consequences can be
        // fatal from a security perspective.

        // This is not just theoretical; ptrace allows userspace to change RCX (via RIP) of target
        // processes.

        // While we could also conditionally IRETQ here, an easier method is to simply sign-extend RCX:

        // Shift away the upper 16 bits (0xBAAD_8000_DEAD_BEEF => 0x8000_DEAD_BEEF_XXXX).
        "shl rcx, 16;",
        // Shift arithmetically right by 16 bits, effectively extending the 47th sign bit to bits
        // 63:48 (0x8000_DEAD_BEEF_XXXX => 0xFFFF_8000_DEAD_BEEF).
        "sar rcx, 16;",

        "add rsp, 8;",              // Pop fake userspace CS
        "pop r11;",                 // Pop rflags
        "pop rsp;",                 // Restore userspace stack pointer
        "sysretq;",                 // Return into userspace; RCX=>RIP,R11=>RFLAGS

        // IRETQ fallback:
        "
        .p2align 4
    1:
        xor rcx, rcx
        xor r11, r11
        iretq
        "),

        sp = const(offset_of!(ProcessorControlRegion, user_rsp_tmp)),
        ksp = const(offset_of!(ProcessorControlRegion, tss) + offset_of!(TaskStateSegment, privilege_stack_table)),
        ss_sel = const(SegmentSelector::new(5, Ring3).0),
        cs_sel = const(SegmentSelector::new(4, Ring3).0),

        options(noreturn),
    );
}

extern "C" {
    pub fn enter_usermode();
}

pub unsafe fn init_syscall() {
    let syscall_cs_ss_base = (1u16) << 3;
    let sysret_cs_ss_base = ((3u16) << 3) | 3;
    let star_high = u32::from(syscall_cs_ss_base) | (u32::from(sysret_cs_ss_base) << 16);

    wrmsr(0xc0000081, u64::from(star_high) << 32); // IA32_STAR
    wrmsr( 0xc0000082, syscall_instruction as u64); // IA32_LSTAR

    let mask_critical = RFlags::DIRECTION_FLAG
        | RFlags::INTERRUPT_FLAG
        | RFlags::TRAP_FLAG
        | RFlags::ALIGNMENT_CHECK;
    let mask_other = RFlags::CARRY_FLAG
        | RFlags::PARITY_FLAG
        | RFlags::AUXILIARY_CARRY_FLAG
        | RFlags::ZERO_FLAG
        | RFlags::SIGN_FLAG
        | RFlags::OVERFLOW_FLAG;
    wrmsr(0xc0000084, (mask_critical | mask_other).bits()); // IA32_FMASK

    wrmsr(0xc0000080, rdmsr(0xc0000080) | 1); //  IA32_EFER
}