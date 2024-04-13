use alloc::sync::Arc;
use core::arch::asm;
use core::cell::Cell;
use core::hint::spin_loop;
use core::mem::transmute;
use core::mem::offset_of;
use core::ops::Bound;
use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicBool, Ordering};
use log::info;
use spin::RwLockWriteGuard;
use spinning_top::guard::ArcRwSpinlockWriteGuard;
use shared::print_panic::PrintPanic;
use crate::context::{Context, ContextId, ContextRegisters};
use crate::context::list::context_storage;
use crate::cpu::{LogicalCpuId, PercpuBlock};
use crate::device::qemu::{exit_qemu, QemuExitCode};
use crate::gdt::pcr;
use crate::{infohart, qemu_println};
use crate::mem::user_addr_space::RwLockUserAddrSpace;

// if is in context switch, preventing multiple call to [`switch_context`]
static CONTEXT_SWITCH_LOCK: AtomicBool = AtomicBool::new(false);

struct SwitchResultInner {
    prev_ctx: ArcRwSpinlockWriteGuard<Context>,
    next_ctx: ArcRwSpinlockWriteGuard<Context>
}

#[derive(Default)]
pub struct ContextSwitchPercpu {
    switch_result: Cell<Option<SwitchResultInner>>,
    pit_ticks: Cell<usize>,
    /// Unique ID of the currently running context.
    context_id: Cell<ContextId>,
    // The ID of the idle process
    idle_id: Cell<ContextId>,
    switch_signal: Cell<bool>,
}

impl ContextSwitchPercpu {
    pub fn context_id(&self) -> ContextId {
        self.context_id.get()
    }
    pub unsafe fn set_context_id(&self, new: ContextId) {
        self.context_id.set(new)
    }
    pub fn idle_id(&self) -> ContextId {
        self.idle_id.get()
    }
    pub unsafe fn set_idle_id(&self, new: ContextId) {
        self.idle_id.set(new)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SwitchResult {
    Switched { signal: bool },
    AllContextsIdle,
}

unsafe fn upgrade_runnable(context: &mut Context, cpu_id: LogicalCpuId) -> Result<bool, ()> {
    if context.running {
        return Err(())
    }
    // 支持调度到其他核
    if !context.cpu_id.map_or(true, |x| x == cpu_id) {
        return Err(())
    }

    let signal_deliverable = context.signal.deliverable() != 0;

    if context.status.is_soft_blocked() && signal_deliverable {
        context.unblock_no_ipi();
    }

    // TODO: userspace sleep wake

    if context.status.is_runnable() {
        Ok(signal_deliverable)
    } else {
        Err(())
    }
}

/// Switch to the next context, picked by the scheduler.
///
/// This is not memory-unsafe to call, but do NOT call this while holding locks!
pub unsafe fn switch_context() -> SwitchResult {
    let percpu = PercpuBlock::current();
    //set PIT Interrupt counter to 0, giving each process same amount of PIT ticks
    percpu.context_switch.pit_ticks.set(0);

    while CONTEXT_SWITCH_LOCK.compare_exchange_weak(false, true, Ordering::SeqCst, Ordering::Relaxed).is_err() {
        spin_loop()
    }

    let mut selected_switch_context = None;
    {
        let contexts = context_storage();

        let prev_context_lock = contexts.current()
            .or_panic("failed to get current context");
        let prev_context = prev_context_lock.write_arc();

        let idle_id = percpu.context_switch.idle_id();
        let mut skip_idle = false;

        let contexts_iter = contexts
            .range((Bound::Excluded(prev_context.id), Bound::Unbounded))
            .chain(contexts.range(..prev_context.id))
            .chain(contexts.range(idle_id..idle_id));

        for (cid, ctx_lock) in contexts_iter {
            if cid == &idle_id && skip_idle {
                // Skip idle process the first time it shows up
                skip_idle = false;
                continue;
            }

            let mut ctx = ctx_lock.write_arc();

            if let Ok(signal_deliverable) = upgrade_runnable(&mut *ctx, percpu.cpu_id) {
                infohart!("selected: prev: {:?}, curr: {:?}", prev_context.id, ctx.id);
                selected_switch_context = Some((prev_context, ctx));
                percpu.context_switch.switch_signal.set(signal_deliverable);

                break
            }
        }
    }

    if let Some((mut prev_ctx_guard, mut next_ctx_guard)) = selected_switch_context {
        // Set old context as not running and update CPU time
        let prev_ctx = &mut *prev_ctx_guard;
        prev_ctx.running = false;
        // todo: add prev ctx cpu time

        // Set new context as running and set switch time
        let next_ctx = &mut *next_ctx_guard;
        next_ctx.running = true;
        next_ctx.cpu_id = Some(percpu.cpu_id);

        percpu.context_switch.context_id.set(next_ctx.id);

        // context guard 要保存起来防止被 RAII 释放
        // 下面 switch 后会改变程序流，所以把 guard 所有权交给 percpu block
        // 在这里 leak guard，下面能继续用
        let prev_ctx_unguarded: &mut Context = transmute(&mut *prev_ctx_guard);
        let next_ctx_unguarded: &mut Context = transmute(&mut *next_ctx_guard);

        percpu.context_switch.switch_result.set(
            Some(SwitchResultInner {
                prev_ctx: prev_ctx_guard,
                next_ctx: next_ctx_guard
            })
        );

        prev_ctx_unguarded.inside_syscall = percpu.inside_syscall.replace(next_ctx_unguarded.inside_syscall);

        // switch
        let pcr = pcr();
        if let Some(ref stack) = next_ctx_unguarded.kstack {
            pcr.set_tss_stack((stack.as_ptr() as usize + stack.len()) as u64);
        }
        pcr.set_userspace_io_allowed(next_ctx_unguarded.ctx_regs.userspace_io_allowed);

        // save gs and fs
        asm!(
            "
            mov ecx, {MSR_FSBASE}
            rdmsr
            mov [{prev}+{fsbase_off}], eax
            mov rdx, [{next}+{fsbase_off}]
            mov eax, edx
            shr rdx, 32
            wrmsr

            mov ecx, {MSR_KERNEL_GSBASE}
            rdmsr
            mov [{prev}+{gsbase_off}], eax
            mov rdx, [{next}+{gsbase_off}]
            mov eax, edx
            shr rdx, 32
            wrmsr
            ",
            out("rax") _,
            out("rdx") _,
            out("ecx") _,
            prev = in(reg) addr_of!(prev_ctx_unguarded.ctx_regs),
            next = in(reg) addr_of!(next_ctx_unguarded.ctx_regs),
            MSR_FSBASE = const 0xc0000100u32, // IA32_FS_BASE,
            MSR_KERNEL_GSBASE = const 0xc0000102u32, // IA32_KERNEL_GSBASE,
            gsbase_off = const offset_of!(ContextRegisters, gsbase),
            fsbase_off = const offset_of!(ContextRegisters, fsbase),
        );

        switch_context_inner(&mut prev_ctx_unguarded.ctx_regs, &mut next_ctx_unguarded.ctx_regs);

        // NOTE: After switch_to is called, the return address can even be different from the
        // current return address, meaning that we cannot use local variables here, and that we
        // need to use the `switch_finish_hook` to be able to release the locks. Newly created
        // contexts will return directly to the function pointer passed to context::spawn, and not
        // reach this code until the next context switch back.

        SwitchResult::Switched {
            signal: PercpuBlock::current().context_switch.switch_signal.get()
        }
    } else {
        CONTEXT_SWITCH_LOCK.store(false, Ordering::SeqCst);
        SwitchResult::AllContextsIdle
    }
}

#[naked]
unsafe extern "sysv64" fn switch_context_inner(
    _prev: &mut ContextRegisters,
    _next: &mut ContextRegisters
) {
    // As a quick reminder for those who are unfamiliar with the System V ABI (extern "C"):
    //
    // - the current parameters are passed in the registers `rdi`, `rsi`,
    // - we can modify scratch registers, e.g. rax
    // - we cannot change callee-preserved registers arbitrarily, e.g. rbx, which is why we
    //   store them here in the first place.

    asm!(
        concat!("
        // Save old registers, and load new ones
        mov [rdi + {off_rbx}], rbx
        mov rbx, [rsi + {off_rbx}]

        mov [rdi + {off_r12}], r12
        mov r12, [rsi + {off_r12}]

        mov [rdi + {off_r13}], r13
        mov r13, [rsi + {off_r13}]

        mov [rdi + {off_r14}], r14
        mov r14, [rsi + {off_r14}]

        mov [rdi + {off_r15}], r15
        mov r15, [rsi + {off_r15}]

        mov [rdi + {off_rbp}], rbp
        mov rbp, [rsi + {off_rbp}]

        mov [rdi + {off_rsp}], rsp
        mov rsp, [rsi + {off_rsp}]

        // push RFLAGS (can only be modified via stack)
        pushfq
        // pop RFLAGS into `self.rflags`
        pop QWORD PTR [rdi + {off_rflags}]

        // push `next.rflags`
        push QWORD PTR [rsi + {off_rflags}]
        // pop into RFLAGS
        popfq

        // When we return, we cannot even guarantee that the return address on the stack, points to
        // the calling function, `context::switch`. Thus, we have to execute this Rust hook by
        // ourselves, which will unlock the contexts before the later switch.

        // Note that switch_finish_hook will be responsible for executing `ret`.
        jmp {switch_hook}

        "),

        off_rflags = const(offset_of!(ContextRegisters, rflags)),

        off_rbx = const(offset_of!(ContextRegisters, rbx)),
        off_r12 = const(offset_of!(ContextRegisters, r12)),
        off_r13 = const(offset_of!(ContextRegisters, r13)),
        off_r14 = const(offset_of!(ContextRegisters, r14)),
        off_r15 = const(offset_of!(ContextRegisters, r15)),
        off_rbp = const(offset_of!(ContextRegisters, rbp)),
        off_rsp = const(offset_of!(ContextRegisters, rsp)),

        switch_hook = sym post_switch_context,
        options(noreturn),
    )
}

pub unsafe extern "C" fn post_switch_context() {
    let percpu = PercpuBlock::current();
    let switch_result = percpu.context_switch.switch_result.take();

    CONTEXT_SWITCH_LOCK.store(false, Ordering::SeqCst);

    if let Some(result) = switch_result {
        let cmp = match (&result.prev_ctx.addrsp, &result.next_ctx.addrsp) {
            (Some(ref p), Some(ref n)) => Arc::ptr_eq(p, n),
            (Some(_), None) | (None, Some(_)) => false,
            (None, None) => true
        };

        if cmp { return }

        let next_ctx_guard = result.next_ctx;
        if let Some(addrsp) = &next_ctx_guard.addrsp {
            let mut write = addrsp.acquire_write();
            write.validate();
        }
    }
}