//! M15 — Daily/monthly cap evaluation over usage counters.

/// Default daily action caps by tier (spec §20.3).
pub const DEFAULT_CAPS: &[(&str, i64)] = &[
    ("daily_login", 1),
    ("read_chapter", 10),
    ("react", 20),
    ("daily_total", 50),
];

/// Check whether an action is within its daily cap.
pub fn within_daily_cap(count: i64, cap: i64) -> bool {
    count < cap
}

/// Check whether a total daily spend is within the tier ceiling.
pub fn within_tier_ceiling(total: i64, tier: &str) -> bool {
    let ceiling = match tier {
        "reader" => 50,
        "author" => 75,
        "curator" => 100,
        "patron" => 100,
        _ => 50, // default fallback
    };
    total <= ceiling
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_cap() {
        assert!(within_daily_cap(5, 10));
        assert!(within_daily_cap(9, 10));
    }

    #[test]
    fn at_cap_boundary() {
        assert!(!within_daily_cap(10, 10));
    }

    #[test]
    fn over_cap() {
        assert!(!within_daily_cap(11, 10));
    }

    #[test]
    fn tier_ceilings() {
        assert!(within_tier_ceiling(50, "reader"));
        assert!(!within_tier_ceiling(51, "reader"));
        assert!(within_tier_ceiling(75, "author"));
        assert!(within_tier_ceiling(100, "curator"));
    }
}
