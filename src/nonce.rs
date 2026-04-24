use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a 4-hex-char nonce for a given PID.
/// Mixes PID, wall-clock nanoseconds, and a monotonic counter
/// so concurrent spawns at the same nanosecond still differ.
pub fn generate(pid: u64) -> String {
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;

    // Splitmix64-style bit mixing
    let mut v = pid
        .wrapping_mul(0x9e3779b97f4a7c15)
        .wrapping_add(nanos)
        .wrapping_add(count.wrapping_mul(0x6c62272e07bb0142));
    v ^= v >> 30;
    v = v.wrapping_mul(0xbf58476d1ce4e5b9);
    v ^= v >> 27;
    v = v.wrapping_mul(0x94d049bb133111eb);
    v ^= v >> 31;

    format!("{:04x}", v & 0xffff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonces_are_four_hex_chars() {
        for pid in 1..=10 {
            let n = generate(pid);
            assert_eq!(n.len(), 4, "nonce should be 4 chars, got: {}", n);
            assert!(
                n.chars().all(|c| c.is_ascii_hexdigit()),
                "non-hex char in nonce: {}",
                n
            );
        }
    }

    #[test]
    fn nonces_differ_between_calls() {
        let nonces: Vec<_> = (1..=20).map(generate).collect();
        let unique: std::collections::HashSet<_> = nonces.iter().collect();
        // Birthday bound: P(any collision in 20 draws from 65536) ≈ 0.3%
        assert!(
            unique.len() > 15,
            "too many collisions in nonces: {:?}",
            nonces
        );
    }
}
