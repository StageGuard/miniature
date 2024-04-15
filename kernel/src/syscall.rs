use core::arch::asm;
use core::fmt::{Debug, Formatter, LowerHex, write};
use core::mem::offset_of;
use core::slice::from_raw_parts;
use log::info;
use x86_64::{PhysAddr, PrivilegeLevel};
use x86_64::PrivilegeLevel::Ring3;
use x86_64::registers::rflags::RFlags;
use x86_64::registers::segmentation::SegmentSelector;
use x86_64::structures::paging::{PhysFrame, Size4KiB};
use x86_64::structures::tss::TaskStateSegment;
use libvdso::error::{KError, KResult};
use shared::print_panic::PrintPanic;
use crate::arch_spec::msr::{rdmsr, wrmsr};
use crate::gdt::{GDT_USER_CODE64, GDT_USER_DATA, pcr, ProcessorControlRegion};
use crate::{infohart, push_scratch, push_preserved, pop_scratch, pop_preserved, qemu_println};
use crate::cpu::PercpuBlock;
use crate::mem::PAGE_SIZE;

#[derive(Default)]
#[repr(C)]
pub struct InterruptStack {
    pub preserved: PreservedRegisters,
    pub scratch: ScratchRegisters,
    pub iret: IretRegisters,
}

impl Debug for InterruptStack {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write(f, format_args!("INTRS {{ "))?;
        write(f, format_args!("{:?}, ", self.preserved))?;
        write(f, format_args!("{:?}, ", self.scratch))?;
        write(f, format_args!("{:?}", self.iret))?;
        write(f, format_args!(" }}"))
    }
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

impl Debug for PreservedRegisters {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write(f, format_args!("PRESERVED {{ "))?;
        write(f, format_args!("R15: 0x{:x}, ", self.r15))?;
        write(f, format_args!("R14: 0x{:x}, ", self.r14))?;
        write(f, format_args!("R13: 0x{:x}, ", self.r13))?;
        write(f, format_args!("R12: 0x{:x}, ", self.r12))?;
        write(f, format_args!("RBP: 0x{:x}, ", self.rbp))?;
        write(f, format_args!("RBX: 0x{:x}", self.rbx))?;
        write(f, format_args!(" }}"))
    }
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

impl Debug for ScratchRegisters {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write(f, format_args!("SCRATCH {{ "))?;
        write(f, format_args!("R11: 0x{:x}, ", self.r11))?;
        write(f, format_args!("R10: 0x{:x}, ", self.r10))?;
        write(f, format_args!("R9: 0x{:x}, ", self.r9))?;
        write(f, format_args!("R8: 0x{:x}, ", self.r8))?;
        write(f, format_args!("RSI: 0x{:x}, ", self.rsi))?;
        write(f, format_args!("RDI: 0x{:x}, ", self.rdi))?;
        write(f, format_args!("RDX: 0x{:x}, ", self.rdx))?;
        write(f, format_args!("RCX: 0x{:x}, ", self.rcx))?;
        write(f, format_args!("RAX: 0x{:x}", self.rax))?;
        write(f, format_args!(" }}"))
    }
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

impl Debug for IretRegisters {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write(f, format_args!("IRET {{ "))?;
        write(f, format_args!("RIP: 0x{:x}, ", self.rip))?;
        write(f, format_args!("CS: {:b}, ", self.cs))?;
        write(f, format_args!("RFLAGS: {:b}, ", self.rflags))?;
        write(f, format_args!("RSP: 0x{:x}, ", self.rsp))?;
        write(f, format_args!("SS: {:b}", self.ss))?;
        write(f, format_args!(" }}"))
    }
}

impl InterruptStack {
    pub fn init(&mut self) {
        // Always enable interrupts!
        self.iret.rflags = RFlags::INTERRUPT_FLAG.bits() as usize;
        self.iret.cs = GDT_USER_CODE64.get().or_panic("failed to get user code segment sector").0 as usize;
        self.iret.ss = GDT_USER_DATA.get().or_panic("failed to get user data segment sector").0 as usize;
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
    pub fn set_syscall_ret_reg(&mut self, ret: usize) -> usize {
        self.scratch.rax = ret;
        ret
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

    PercpuBlock::current().inside_syscall.set(true);

    infohart!("syscall: args = {:?}", stack_ref);
    let result = Ok(0);

    PercpuBlock::current().inside_syscall.set(false);

    stack_ref.set_syscall_ret_reg(KError::mux(result));
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

        // Push context registers
        "push rax;",
        push_scratch!(),
        push_preserved!(),

        // Call inner funtion
        "mov rdi, rsp;",
        "call __inner_syscall_instruction;",

        "
    .globl enter_usermode
        enter_usermode:
        ",
        // Pop context registers
        pop_preserved!(),
        pop_scratch!(),

        // Restore user GSBASE by swapping GSBASE and KGSBASE.
        "swapgs;",

        // check trap flag
        "test BYTE PTR [rsp + 17], 1;",
        // If set, return using IRETQ instead.
        "jnz 1f;",

        // Otherwise, continue with the fast sysretq.

        // Pop userspace return pointer
        "pop rcx;", // InterruptStack.iret.rip

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

        "add rsp, 8;",              // Pop fake userspace CS, skip InterruptStack.iret.cs
        "pop r11;",                 // Pop rflags, InterruptStack.iret.rflags
        "pop rsp;",                 // Restore userspace stack pointer, InterruptStack.iret.rsp
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
        cs_sel = const(SegmentSelector::new(5, Ring3).0),
        ss_sel = const(SegmentSelector::new(4, Ring3).0),

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