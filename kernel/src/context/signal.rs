use libvdso::flag::{SIGKILL, SIGSTOP};

#[derive(Clone, Copy, Debug)]
pub struct SignalState {
    /// Bitset of pending signals.
    pub pending: u64,
    /// Bitset of procmasked signals.
    pub procmask: u64,
}

impl SignalState {
    pub fn deliverable(&self) -> u64 {
        const CANT_BLOCK: u64 = (1 << (SIGKILL - 1)) | (1 << (SIGSTOP - 1));
        self.pending & (CANT_BLOCK | !self.procmask)
    }
}