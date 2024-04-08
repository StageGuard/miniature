
#[derive(Clone, Debug)]
pub enum Status {
    Runnable,
    SoftBlocked {
        reason: &'static str,
    },
    HardBlocked {
        reason: HardBlockedReason
    },
    Stopped(usize),
    Existed(usize),
}

#[derive(Clone, Debug)]
pub enum HardBlockedReason {
    NotYetStarted,
}

impl Status {
    pub fn is_runnable(&self) -> bool {
        matches!(self, Self::Runnable)
    }
    pub fn is_soft_blocked(&self) -> bool {
        matches!(self, Self::SoftBlocked { .. })
    }
}