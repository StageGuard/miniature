use core::sync::atomic::AtomicUsize;
use crate::context::list::context_storage_mut;
use crate::mem::aligned_box::AlignedBox;
use crate::context::signal::SignalState;
use crate::context::status::{HardBlockedReason, Status};
use crate::cpu::{LogicalCpuId, PercpuBlock};
use crate::int_like;
use crate::mem::PAGE_SIZE;
use crate::mem::user_addr_space::ArcRwLockUserAddrSpace;

pub mod list;
pub mod switch;
mod status;
mod signal;

int_like!(ContextId, AtomicContextId, usize, AtomicUsize);

// A task context, identifies either a process control lock or task control block
pub struct Context {
    // the unique id of this context
    pub id: ContextId,
    // if the context is running
    pub running: bool,
    // underlying cpu id if running
    pub cpu_id: Option<LogicalCpuId>,
    // is the context in syscall_module
    pub inside_syscall: bool,
    // kernel stack
    pub kstack: Option<AlignedBox<[u8; 64 * PAGE_SIZE], { PAGE_SIZE }>>,
    // context status
    pub status: Status,
    // signal state
    pub signal: SignalState,
    // registers
    pub regs: ContextRegisters,
    // All contexts except kmain will primarily live in userspace, and enter the kernel only when
    // interrupts or syscall occur. This flag is set for all contexts but kmain.
    pub userspace: bool,
    // address space
    pub addrsp: Option<ArcRwLockUserAddrSpace>,
}

impl Context {
    pub fn new(id: ContextId) -> Self {
        Context {
            id,
            running: false,
            cpu_id: None,
            inside_syscall: false,
            kstack: None,
            status: Status::HardBlocked { reason: HardBlockedReason::NotYetStarted },
            signal: SignalState {
                pending: 0,
                procmask: !0
            },
            regs: ContextRegisters::new(),
            userspace: false,
            addrsp: None
        }
    }
    /// Block the context, and return true if it was runnable before being blocked
    pub fn soft_block(&mut self, reason: &'static str) -> bool {
        if self.status.is_runnable() {
            self.status = Status::SoftBlocked { reason };
            true
        } else {
            false
        }
    }

    pub fn hard_block(&mut self, reason: HardBlockedReason) -> bool {
        if self.status.is_runnable() {
            self.status = Status::HardBlocked { reason };
            true
        } else {
            false
        }
    }

    /// Unblock context, and return true if it was blocked before being marked runnable
    pub fn unblock(&mut self) -> bool {
        if self.unblock_no_ipi() {
            if let Some(cpu_id) = self.cpu_id {
                if cpu_id != PercpuBlock::current().cpu_id {
                    // Send IPI if not on current CPU
                    //ipi(IpiKind::Wakeup, IpiTarget::Other);
                }
            }

            true
        } else {
            false
        }
    }

    /// Unblock context without IPI, and return true if it was blocked before being marked runnable
    pub fn unblock_no_ipi(&mut self) -> bool {
        if self.status.is_soft_blocked() {
            self.status = Status::Runnable;
            true
        } else {
            false
        }
    }
}

#[repr(C)]
#[derive(Clone, Debug)]
pub struct ContextRegisters {
    /// RFLAGS register
    rflags: usize,
    /// RBX register
    rbx: usize,
    /// R12 register
    r12: usize,
    /// R13 register
    r13: usize,
    /// R14 register
    r14: usize,
    /// R15 register
    r15: usize,
    /// Base pointer
    rbp: usize,
    /// Stack pointer
    pub rsp: usize,
    /// FSBASE.
    ///
    /// NOTE: Same fsgsbase behavior as with gsbase.
    pub fsbase: usize,
    /// GSBASE.
    ///
    /// NOTE: Without fsgsbase, this register will strictly be equal to the register value when
    /// running. With fsgsbase, this is neither saved nor restored upon every syscall (there is no
    /// need to!), and thus it must be re-read from the register before copying this struct.
    pub gsbase: usize,
    userspace_io_allowed: bool,
}

impl ContextRegisters {
    pub fn new() -> ContextRegisters {
        ContextRegisters {
            rflags: 0,
            rbx: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rbp: 0,
            rsp: 0,
            fsbase: 0,
            gsbase: 0,
            userspace_io_allowed: false,
        }
    }

    pub fn set_stack(&mut self, address: usize) {
        self.rsp = address;
    }
}

pub fn context_id() -> ContextId {
    PercpuBlock::current().context_switch.context_id()
}

pub fn init_context() {
    let percpu = PercpuBlock::current();
    let mut contexts = context_storage_mut();
    let id = ContextId::from(percpu.cpu_id.0 as usize);

    let context_lock = contexts.insert_context(id)
        .expect("failed to initialize first context");
    let mut context = context_lock.write();

    context.signal.procmask = 0;
    context.status = Status::Runnable;
    context.running = true;
    context.cpu_id = Some(percpu.cpu_id);

    unsafe {
        percpu.context_switch.set_context_id(context.id);
        percpu.context_switch.set_idle_id(context.id);
    }
}