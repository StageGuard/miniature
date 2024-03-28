use x86_64::registers::model_specific::Msr;

#[inline]
pub unsafe fn rdmsr(reg: u32) -> u64 {
    Msr::new(reg).read()
}

#[inline]
pub unsafe fn wrmsr(reg: u32, value: u64) {
    Msr::new(reg).write(value)
}