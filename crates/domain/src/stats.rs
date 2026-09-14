//! M19 — Statistics domain: k-anonymity, aggregation definitions, gap reporting.

/// Minimum count threshold for per-entity statistics (k-anonymity).
pub const K_ANON_FLOOR: i64 = 5;

/// Apply k-anonymity floor: counts below the floor are merged into "other".
pub fn apply_k_anonymity(key: &str, count: i64) -> (String, i64) {
    if count < K_ANON_FLOOR && !key.is_empty() {
        ("other".to_string(), count)
    } else {
        (key.to_string(), count)
    }
}

/// Check whether a statistic can be computed (has sufficient data).
pub fn stat_available(count: i64) -> bool {
    count >= K_ANON_FLOOR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k_anonymity_merges_small_counts() {
        let (key, _) = apply_k_anonymity("work-123", 3);
        assert_eq!(key, "other");
    }

    #[test]
    fn k_anonymity_keeps_large_counts() {
        let (key, count) = apply_k_anonymity("work-123", 10);
        assert_eq!(key, "work-123");
        assert_eq!(count, 10);
    }

    #[test]
    fn stat_available_at_floor() {
        assert!(stat_available(K_ANON_FLOOR));
        assert!(!stat_available(K_ANON_FLOOR - 1));
    }
}
