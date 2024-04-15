#[macro_export]
macro_rules! push_scratch {
    () => {
        "
        // Push scratch registers
        push rcx
        push rdx
        push rdi
        push rsi
        push r8
        push r9
        push r10
        push r11
    "
    };
}
#[macro_export]
macro_rules! pop_scratch {
    () => {
        "
        // Pop scratch registers
        pop r11
        pop r10
        pop r9
        pop r8
        pop rsi
        pop rdi
        pop rdx
        pop rcx
        pop rax
    "
    };
}

#[macro_export]
macro_rules! push_preserved {
    () => {
        "
        // Push preserved registers
        push rbx
        push rbp
        push r12
        push r13
        push r14
        push r15
    "
    };
}
#[macro_export]
macro_rules! pop_preserved {
    () => {
        "
        // Pop preserved registers
        pop r15
        pop r14
        pop r13
        pop r12
        pop rbp
        pop rbx
    "
    };
}
#[macro_export]
macro_rules! swapgs_iff_ring3_fast {
    // TODO: Spectre V1: LFENCE?
    () => {
        "
        // Check whether the last two bits RSP+8 (code segment) are equal to zero.
        test QWORD PTR [rsp + 8], 0x3
        // Skip the SWAPGS instruction if CS & 0b11 == 0b00.
        jz 1f
        swapgs
        1:
    "
    };
}
#[macro_export]
macro_rules! swapgs_iff_ring3_fast_errorcode {
    // TODO: Spectre V1: LFENCE?
    () => {
        "
        test QWORD PTR [rsp + 16], 0x3
        jz 1f
        swapgs
        1:
    "
    };
}

#[macro_export]
macro_rules! conditional_swapgs_paranoid {
    // For regular interrupt handlers and the syscall handler, managing IA32_GS_BASE and
    // IA32_KERNEL_GS_BASE (the "GSBASE registers") is more or less trivial when using the SWAPGS
    // instruction.
    //
    // The syscall handler simply runs SWAPGS, as syscalls can only originate from usermode,
    // whereas interrupt handlers conditionally SWAPGS unless the interrupt was triggered from
    // kernel mode, in which case the "swap state" is already valid, and there is no need to
    // SWAPGS.
    //
    // Handling GSBASE correctly for paranoid interrupts however, is not as simple. NMIs can occur
    // between the check of whether an interrupt came from usermode, and the actual SWAPGS
    // instruction. #DB can also be triggered inside of a kernel interrupt handler, due to
    // breakpoints, even though setting up such breakpoints in the first place, is not yet
    // supported by the kernel.
    //
    // Luckily, the GDT always resides in the PCR (at least after init_paging, but there are no
    // interrupt handlers set up before that), allowing GSBASE to be calculated relatively cheaply.
    // Out of the two GSBASE registers, at least one must be *the* kernel GSBASE, allowing for a
    // simple conditional SWAPGS.
    //
    // (An alternative to conditionally executing SWAPGS, would be to save and restore GSBASE via
    // e.g. the stack. That would nonetheless require saving and restoring both GSBASE registers,
    // if the interrupt handler should be allowed to context switch, which the current #DB handler
    // may do.)
    //
    // TODO: Handle nested NMIs like Linux does (https://lwn.net/Articles/484932/)?.

    () => { concat!(
        // Put the GDT base pointer in RDI.
        "
        sub rsp, 16
        sgdt [rsp + 6]
        mov rdi, [rsp + 8]
        add rsp, 16
        ",
        // Calculate the PCR address by subtracting the offset of the GDT in the PCR struct.
        "sub rdi, {PCR_GDT_OFFSET};",

        // Read the current IA32_GS_BASE value into RDX.
        "
        mov ecx, {IA32_GS_BASE}
        rdmsr
        shl rdx, 32
        or rdx, rax
        ",

        // If they were not equal, the PCR address must instead be in IA32_KERNEL_GS_BASE,
        // requiring a SWAPGS. GSBASE needs to be swapped back, so store the same flag in RBX.

        "
        cmp rdx, rdi
        sete bl
        je 1f
        swapgs
        1:
        ",
    ) }
}
#[macro_export]
macro_rules! conditional_swapgs_back_paranoid {
    () => {
        "
        test bl, bl
        jnz 1f
        swapgs
        1:
    "
    };
}
#[macro_export]
macro_rules! nop {
    () => {
        "
        // Unused: {IA32_GS_BASE} {PCR_GDT_OFFSET}
        "
    };
}

#[macro_export]
macro_rules! interrupt_stack {
    // XXX: Apparently we cannot use $expr and check for bool exhaustiveness, so we will have to
    // use idents directly instead.
    ($name:ident, $save1:ident!, $save2:ident!, $rstor2:ident!, $rstor1:ident!, is_paranoid: $is_paranoid:expr, |$stack:ident| $code:block) => {
        #[naked]
        pub unsafe extern "C" fn $name() {
            unsafe extern "C" fn inner($stack: &mut $crate::syscall::InterruptStack) {
                #[allow(unused_unsafe)]
                unsafe {
                    $code
                }
            }
            core::arch::asm!(concat!(
                // Clear direction flag, required by ABI when running any Rust code in the kernel.
                "cld;",

                // Backup all userspace registers to stack
                $save1!(),
                "push rax\n",
                push_scratch!(),
                push_preserved!(),

                $save2!(),

                // Call inner function with pointer to stack
                "
                mov rdi, rsp
                call {inner}
                ",

                $rstor2!(),

                // Restore all userspace registers
                pop_preserved!(),
                pop_scratch!(),

                $rstor1!(),
                "iretq\n",
            ),

            inner = sym inner,
            IA32_GS_BASE = const 0xc0000101u32,

            PCR_GDT_OFFSET = const(core::mem::offset_of!(crate::gdt::ProcessorControlRegion, gdt)),

            options(noreturn),

            );
        }
    };
    ($name:ident, |$stack:ident| $code:block) => { interrupt_stack!($name, swapgs_iff_ring3_fast!, nop!, nop!, swapgs_iff_ring3_fast!, is_paranoid: false, |$stack| $code); };
    ($name:ident, @paranoid, |$stack:ident| $code:block) => { interrupt_stack!($name, nop!, conditional_swapgs_paranoid!, conditional_swapgs_back_paranoid!, nop!, is_paranoid: true, |$stack| $code); }
}

#[macro_export]
macro_rules! interrupt {
    ($name:ident, || $code:block) => {
        #[naked]
        pub unsafe extern "C" fn $name() {
            unsafe extern "C" fn inner() {
                $code
            }

            core::arch::asm!(concat!(
                // Clear direction flag, required by ABI when running any Rust code in the kernel.
                "cld;",

                // Backup all userspace registers to stack
                swapgs_iff_ring3_fast!(),
                "push rax\n",
                push_scratch!(),

                // Call inner function with pointer to stack
                "call {inner}\n",

                // Restore all userspace registers
                pop_scratch!(),

                swapgs_iff_ring3_fast!(),
                "iretq\n",
            ),

            inner = sym inner,

            options(noreturn),
            );
        }
    };
}

#[macro_export]
macro_rules! interrupt_error {
    ($name:ident, |$stack:ident, $error_code:ident| $code:block) => {
        #[naked]
        pub unsafe extern "C" fn $name() {
            unsafe extern "C" fn inner($stack: &mut $crate::syscall::InterruptStack, $error_code: usize) {
                #[allow(unused_unsafe)]
                unsafe {
                    $code
                }
            }

            core::arch::asm!(concat!(
                // Clear direction flag, required by ABI when running any Rust code in the kernel.
                "cld;",

                swapgs_iff_ring3_fast_errorcode!(),

                // Don't push RAX yet, as the error code is already stored in RAX's position.

                // Push all userspace registers
                push_scratch!(),
                push_preserved!(),

                // Now that we have a couple of usable registers, put the error code in the second
                // argument register for the inner function, and save RAX where it would normally
                // be.
                "mov rsi, [rsp + {rax_offset}];",
                "mov [rsp + {rax_offset}], rax;",

                // Call inner function with pointer to stack, and error code.
                "mov rdi, rsp;",
                "call {inner};",

                // Restore all userspace registers
                pop_preserved!(),
                pop_scratch!(),

                // The error code has already been popped, so use the regular macro.
                swapgs_iff_ring3_fast!(),
                "iretq;",
            ),

            inner = sym inner,
            rax_offset = const(::core::mem::size_of::<$crate::syscall::PreservedRegisters>() + ::core::mem::size_of::<$crate::syscall::ScratchRegisters>() - 8),

            options(noreturn));
        }
    };
}