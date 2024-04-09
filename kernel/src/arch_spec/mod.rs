use core::arch::asm;

pub mod msr;
pub mod cpuid;
pub mod port;

#[naked]
pub unsafe extern "C" fn copy_to(dst: usize, src: usize, len: usize) -> u8 {
    // `movsb` instruction copies from rsi(arg 1) to rdi(arg 0)
    // control length with `rep` instruction
    asm!(
        "
        xor eax, eax
        mov rcx, rdx
        stac
        rep movsb
        clac
        ret
        ",
        options(noreturn)
    )
}