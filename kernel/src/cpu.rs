use core::cell::Cell;
use crate::context::switch::ContextSwitchPercpu;
use crate::gdt::pcr;

// represents a physical cpu
#[derive(Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct LogicalCpuId(pub u8);

impl LogicalCpuId {
    pub(crate) const BSP: LogicalCpuId = LogicalCpuId(0);
}

impl core::fmt::Debug for LogicalCpuId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "[logical cpu #{}]", self.0)
    }
}
impl core::fmt::Display for LogicalCpuId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

pub struct PercpuBlock {
    pub cpu_id: LogicalCpuId,
    pub context_switch: ContextSwitchPercpu,
    pub inside_syscall: Cell<bool>
}

impl PercpuBlock {
    pub fn current() -> &'static Self {
        unsafe { &*core::ptr::addr_of!((*pcr()).percpu) }
    }
}