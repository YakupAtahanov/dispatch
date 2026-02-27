use std::sync::atomic::{AtomicU64, Ordering};

/// Global PID counter for the current dispatch session.
/// PIDs are internal identifiers, not OS-level PIDs.
static NEXT_PID: AtomicU64 = AtomicU64::new(1);

pub fn next_pid() -> u64 {
    NEXT_PID.fetch_add(1, Ordering::Relaxed)
}

/// Reset PID counter (useful for testing).
#[cfg(test)]
pub fn reset() {
    NEXT_PID.store(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pids_increment() {
        reset();
        assert_eq!(next_pid(), 1);
        assert_eq!(next_pid(), 2);
        assert_eq!(next_pid(), 3);
    }
}
