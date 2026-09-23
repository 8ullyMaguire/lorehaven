//! Roadmap consensus: Elo math for the feature board (spec §44, ADR 0023).
//!
//! Pure functions only — storage lives in `lorehaven_db`, routes in
//! `lorehaven_app`. The math is ported from FicHub's `fichub-consensus`
//! crate (same author, MIT-licensed); the MaxDiff→matches translation is
//! spec §44.3.

/// K-factor for arena votes (spec §44.3).
pub const K_FACTOR: f64 = 32.0;

/// Starting Elo for a new card (spec §44.1).
pub const START_RATING: f64 = 1500.0;

/// Expected score of `rating_a` against `rating_b` (standard Elo logistic).
#[must_use]
pub fn expected_score(rating_a: f64, rating_b: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf((rating_b - rating_a) / 400.0))
}

/// One Elo update: `rating + K * (score - expected)`.
#[must_use]
pub fn elo_update(rating: f64, opponent_rating: f64, score: f64, k: f64) -> f64 {
    rating + k * (score - expected_score(rating, opponent_rating))
}

/// The result of translating one MaxDiff ballot into virtual 1v1 matches.
///
/// Per spec §44.3: best beats the two unchosen; worst loses to the two
/// unchosen; best-beats-worst is implied and NOT double-counted.
///
/// Returns (best_delta, worst_delta, [unchosen_deltas...]).
#[derive(Debug, Clone, PartialEq)]
pub struct MaxDiffOutcome {
    pub best_delta: f64,
    pub worst_delta: f64,
    pub unchosen_deltas: Vec<f64>,
}

/// Translate a MaxDiff (best/worst) ballot into Elo rating deltas.
///
/// Caller validates that best, worst, and unchosen are distinct.
#[must_use]
pub fn maxdiff_elo_updates(
    best_rating: f64,
    worst_rating: f64,
    unchosen_ratings: &[f64],
    k: f64,
) -> MaxDiffOutcome {
    // Best vs each unchosen: best wins (S=1), unchosen loses (S=0).
    let mut best_delta = 0.0;
    let mut unchosen_deltas = vec![0.0; unchosen_ratings.len()];
    for (i, &u_rating) in unchosen_ratings.iter().enumerate() {
        let e_best = expected_score(best_rating, u_rating);
        let e_u = expected_score(u_rating, best_rating);
        best_delta += k * (1.0 - e_best);
        unchosen_deltas[i] += k * (0.0 - e_u);
    }

    // Worst vs each unchosen: worst loses (S=0), unchosen wins (S=1).
    let mut worst_delta = 0.0;
    for (i, &u_rating) in unchosen_ratings.iter().enumerate() {
        let e_worst = expected_score(worst_rating, u_rating);
        let e_u = expected_score(u_rating, worst_rating);
        worst_delta += k * (0.0 - e_worst);
        unchosen_deltas[i] += k * (1.0 - e_u);
    }

    MaxDiffOutcome {
        best_delta,
        worst_delta,
        unchosen_deltas,
    }
}

/// Kanban stages (spec §44.2). Only `idea` is arena-eligible by default.
pub const STAGES: &[&str] = &[
    "idea",
    "up_next",
    "in_progress",
    "finished",
    "shipped",
    "medium_term",
    "long_term",
    "rejected",
];

/// Whether a stage's cards may appear in arena ballots (spec §44.2).
#[must_use]
pub fn stage_arena_eligible(stage: &str) -> bool {
    stage == "idea"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_score_is_symmetric() {
        let a = expected_score(1500.0, 1500.0);
        assert!((a - 0.5).abs() < 1e-9);
        let b = expected_score(1600.0, 1400.0);
        assert!(b > 0.5);
        assert!((b + expected_score(1400.0, 1600.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn equal_ratings_produce_symmetric_updates() {
        let unchosen = [1500.0, 1500.0];
        let out = maxdiff_elo_updates(1500.0, 1500.0, &unchosen, K_FACTOR);
        assert!(out.best_delta > 0.0, "best must gain");
        assert!(out.worst_delta < 0.0, "worst must lose");
        for d in out.unchosen_deltas {
            assert!((d).abs() < 1e-9, "unchosen at 1500 stays");
        }
    }

    #[test]
    fn zero_sum_across_the_ballot() {
        let unchosen = [1544.0, 1484.0];
        let out = maxdiff_elo_updates(1512.0, 1460.0, &unchosen, K_FACTOR);
        let total = out.best_delta + out.worst_delta + out.unchosen_deltas.iter().sum::<f64>();
        assert!(total.abs() < 1e-9, "ballot must be zero-sum, got {total}");
    }
}
