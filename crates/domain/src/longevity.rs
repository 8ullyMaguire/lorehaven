//! Longevity signals: half-life scoring and interaction tiers (spec §41).
//!
//! Pure functions, no I/O.

/// Compute half-life score in basis points.
///
/// The half-life of a work is the ratio of readers who started it recently
/// to those who started it in its first window, expressed in basis points.
/// A score of 10000 means the work is being read as much as ever; 5000 means
/// it's at half its initial pace; 0 means no recent readers.
///
/// Returns 0 if `first_window_starts` is 0 (can't compute a ratio), clamps
/// the result to 0..=10000.
pub fn half_life_bp(recent_starts: i64, first_window_starts: i64) -> i64 {
    if first_window_starts <= 0 {
        return 0;
    }
    let ratio = recent_starts as f64 / first_window_starts as f64;
    let bp = (ratio * 10_000.0).round() as i64;
    bp.clamp(0, 10_000)
}

/// Apply half-life as a silent ranking multiplier (spec §16.3).
///
/// The multiplier is `1.0 + bp / 20000.0`, so:
/// - 10000 bp → 1.5× (evergreen, boosted)
/// - 5000 bp → 1.25× (moderate decay)
/// - 0 bp → 1.0× (no signal, neutral)
///
/// **The silent contract**: this function must not modify any field except
/// `score`. It does not add fields, does not change `reason`, does not add
/// annotations. The reordering is invisible to the reader.
pub fn apply_half_life(
    candidates: &mut [crate::discovery::Candidate],
    half_life_of: &dyn Fn(&crate::ids::WorkId) -> Option<i64>,
) {
    for candidate in candidates.iter_mut() {
        if let Some(bp) = half_life_of(&candidate.work_id) {
            let multiplier = 1.0 + (bp as f64 / 20_000.0);
            candidate.score = (candidate.score as f64 * multiplier) as i64;
        }
    }
    candidates.sort_by_key(|c| -c.score);
}

/// Warmth delta for an interaction type (spec §41.2).
///
/// Basis points added to the reader-author warmth score.
pub fn warmth_delta(action: WarmthAction) -> i64 {
    match action {
        WarmthAction::ReadChapter => 100,
        WarmthAction::FinishWork => 500,
        WarmthAction::React => 200,
        WarmthAction::Comment => 400,
    }
}

/// Actions that accumulate warmth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarmthAction {
    ReadChapter,
    FinishWork,
    React,
    Comment,
}

/// Default warmth thresholds in basis points (spec §41.2).
pub const DEFAULT_WARMTH_THRESHOLDS: WarmthThresholds = WarmthThresholds {
    lurk: 0,
    react: 200,
    comment: 1000,
    create: 3000,
};

/// Thresholds for interaction tiers. warmth_bp >= threshold qualifies.
#[derive(Debug, Clone, Copy)]
pub struct WarmthThresholds {
    pub lurk: i64,
    pub react: i64,
    pub comment: i64,
    pub create: i64,
}

impl Default for WarmthThresholds {
    fn default() -> Self {
        DEFAULT_WARMTH_THRESHOLDS
    }
}

/// Compute the tier for a given warmth score and threshold set.
pub fn tier_for(warmth_bp: i64, thresholds: &WarmthThresholds) -> &'static str {
    if warmth_bp >= thresholds.create {
        "create"
    } else if warmth_bp >= thresholds.comment {
        "comment"
    } else if warmth_bp >= thresholds.react {
        "react"
    } else {
        "lurk"
    }
}

use serde::{Deserialize, Serialize};

/// Aggregate tier counts for an author panel (spec §41.2).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TierAggregates {
    pub lurk: i64,
    pub react: i64,
    pub comment: i64,
    pub create: i64,
    pub total: i64,
}

impl Default for TierAggregates {
    fn default() -> Self {
        Self {
            lurk: 0,
            react: 0,
            comment: 0,
            create: 0,
            total: 0,
        }
    }
}

/// Compute aggregate tier counts from a list of (tier, count) pairs.
pub fn aggregate_tiers(tiers: &[(String, i64)]) -> TierAggregates {
    let mut agg = TierAggregates::default();
    for (tier, count) in tiers {
        match tier.as_str() {
            "lurk" => agg.lurk += count,
            "react" => agg.react += count,
            "comment" => agg.comment += count,
            "create" => agg.create += count,
            _ => {}
        }
    }
    agg.total = agg.lurk + agg.react + agg.comment + agg.create;
    agg
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_life_zero_first_window() {
        assert_eq!(half_life_bp(10, 0), 0);
        assert_eq!(half_life_bp(0, 0), 0);
    }

    #[test]
    fn half_life_equal_windows() {
        assert_eq!(half_life_bp(100, 100), 10000);
    }

    #[test]
    fn half_life_half_decay() {
        assert_eq!(half_life_bp(50, 100), 5000);
    }

    #[test]
    fn half_life_no_recent_readers() {
        assert_eq!(half_life_bp(0, 100), 0);
    }

    #[test]
    fn half_life_clamps_above_max() {
        // Recent starts > first window (evergreen growth)
        assert_eq!(half_life_bp(200, 100), 10000);
    }

    #[test]
    fn half_life_silent_contract() {
        use crate::discovery::Candidate;
        use crate::ids::WorkId;

        let mut candidates = vec![
            Candidate {
                work_id: WorkId::new(),
                score: 100,
                reason: "tags".into(),
            },
            Candidate {
                work_id: WorkId::new(),
                score: 100,
                reason: "tags".into(),
            },
        ];

        let id_0 = candidates[0].work_id.clone();
        let reason_0 = candidates[0].reason.clone();

        apply_half_life(&mut candidates, &|id| {
            if *id == id_0 {
                Some(10000) // 1.5×
            } else {
                Some(0) // 1.0×
            }
        });

        // Candidate 0 should now be first (boosted)
        assert_eq!(candidates[0].work_id, id_0);
        assert_eq!(candidates[0].score, 150);
        assert_eq!(candidates[0].reason, reason_0);
        // Candidate 1 should be second (unchanged score)
        assert_eq!(candidates[1].score, 100);
    }

    #[test]
    fn warmth_deltas() {
        assert_eq!(warmth_delta(WarmthAction::ReadChapter), 100);
        assert_eq!(warmth_delta(WarmthAction::FinishWork), 500);
        assert_eq!(warmth_delta(WarmthAction::React), 200);
        assert_eq!(warmth_delta(WarmthAction::Comment), 400);
    }

    #[test]
    fn tier_boundaries() {
        let t = WarmthThresholds::default();
        assert_eq!(tier_for(0, &t), "lurk");
        assert_eq!(tier_for(199, &t), "lurk");
        assert_eq!(tier_for(200, &t), "react");
        assert_eq!(tier_for(999, &t), "react");
        assert_eq!(tier_for(1000, &t), "comment");
        assert_eq!(tier_for(2999, &t), "comment");
        assert_eq!(tier_for(3000, &t), "create");
        assert_eq!(tier_for(99999, &t), "create");
    }

    #[test]
    fn aggregate_tiers_sums() {
        let tiers = vec![
            ("lurk".to_string(), 10),
            ("react".to_string(), 5),
            ("comment".to_string(), 3),
            ("create".to_string(), 1),
        ];
        let agg = aggregate_tiers(&tiers);
        assert_eq!(agg.lurk, 10);
        assert_eq!(agg.react, 5);
        assert_eq!(agg.comment, 3);
        assert_eq!(agg.create, 1);
        assert_eq!(agg.total, 19);
    }
}
