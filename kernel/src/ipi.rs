use crate::cpu::LogicalCpuId;
use crate::acpi::local_apic::LOCAL_APIC;

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum IpiKind {
    Wakeup = 0x40,
    Switch = 0x42,
    Pit = 0x43,
}

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum IpiTarget {
    Current = 1,
    All = 2,
    Other = 3,
}

#[inline(always)]
pub fn ipi(kind: IpiKind, target: IpiTarget) {
    let icr = (target as u64) << 18 | 1 << 14 | (kind as u64);
    unsafe { LOCAL_APIC.set_icr(icr) };
}

#[inline(always)]
pub fn ipi_single(kind: IpiKind, target: LogicalCpuId) {
    unsafe {
        LOCAL_APIC.ipi(u32::from(target.0), kind);
    }
}