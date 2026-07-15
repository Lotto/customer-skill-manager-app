use std::time::Duration;

/// Compute an exponential backoff schedule.
///
/// Returns one delay per retry: `base * 2^n`, clamped to `cap`. With
/// `base = 500ms`, `cap = 30s`, `retries = 5` this yields
/// `0.5s, 1s, 2s, 4s, 8s`. The scheduler sleeps `delays[n]` before attempt
/// `n + 1`. Kept pure so the retry policy is unit-testable without any I/O.
pub fn backoff_delays(retries: u32, base: Duration, cap: Duration) -> Vec<Duration> {
    (0..retries)
        .map(|n| {
            let factor = 1u64.checked_shl(n).unwrap_or(u64::MAX);
            let ms = base.as_millis().saturating_mul(factor as u128);
            let capped = ms.min(cap.as_millis());
            Duration::from_millis(capped as u64)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_when_no_retries() {
        assert!(backoff_delays(0, Duration::from_millis(500), Duration::from_secs(30)).is_empty());
    }

    #[test]
    fn doubles_until_cap() {
        let d = backoff_delays(6, Duration::from_millis(500), Duration::from_secs(4));
        assert_eq!(
            d,
            vec![
                Duration::from_millis(500),
                Duration::from_millis(1000),
                Duration::from_millis(2000),
                Duration::from_millis(4000), // hits cap
                Duration::from_millis(4000), // stays capped
                Duration::from_millis(4000),
            ]
        );
    }

    #[test]
    fn does_not_overflow_on_large_retry_counts() {
        // 2^64 would overflow a naive shift; ensure we saturate to the cap.
        let d = backoff_delays(70, Duration::from_millis(500), Duration::from_secs(30));
        assert_eq!(d.len(), 70);
        assert!(d.iter().all(|x| *x <= Duration::from_secs(30)));
        assert_eq!(*d.last().unwrap(), Duration::from_secs(30));
    }
}
