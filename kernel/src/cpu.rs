
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