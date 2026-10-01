//! The tasting menu: an uncertainty-driven calibration queue (spec §49.5, M45-19).
//!
//! ## What this is for
//!
//! §49.1's second problem: a new reader has no weights, so their feed is engine
//! order rather than taste. §49.5's answer is a queue of samples — a summary plus
//! a 300-word passage — rated in seconds with a required reason tag.
//!
//! ## Why selection is by uncertainty and not by popularity
//!
//! The reader's attention is the scarcest input in the system, and §47.2's whole
//! quality depends on it. A queue that samples uniformly spends that attention on
//! works whose outcome the current weights already predict. Picking the item the
//! model is *least* sure about is what makes each minute teach the most.
//!
//! ## Why the selector's confidence is stored
//!
//! `uncertainty_at_draw` is written on the response. Without it, "picked by
//! uncertainty" is a claim about the selector; with it, an evaluation can ask
//! whether the queue actually chose uncertain items. §49.5 makes that an
//! acceptance criterion, and a criterion needs a number to check.
//!
//! ## Determinism
//!
//! §47.9's requirement transfers: two calls on the same state must agree. So the
//! queue is ordered by (uncertainty, work_id) — never by a hash map's iteration
//! order, and never randomly. The *randomness* in §47.3 belongs to exploration
//! slots, which are logged; this queue is not exploration, and randomness here
//! would be an unlogged nondeterminism.

use crate::{Backend, Database};
use anyhow::Result;

/// The bottom of the uncertainty range.
///
/// §49.5's queue ranks by uncertainty, so a work at exactly 0.0 would sort last
/// permanently and never be offered again. Flooring keeps every work reachable.
pub const UNCERTAINTY_FLOOR: f64 = 0.01;

/// §49.5's passage length. A sample, not a summary: prose style is visible in an
/// excerpt, which is why 80k words need not be read to label them.
pub const SAMPLE_WORDS: i64 = 300;

/// Reasons a reader may give. §49.5 makes the reason required and enumerated,
/// with free text as an optional addition rather than the signal itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TastingReason {
    Prose,
    Characters,
    Pacing,
    TropeExecution,
    /// "not for me: ___" — the enum slot the free text fills in.
    NotForMe,
}

impl TastingReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prose => "prose",
            Self::Characters => "characters",
            Self::Pacing => "pacing",
            Self::TropeExecution => "trope_execution",
            Self::NotForMe => "not_for_me",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "prose" => Some(Self::Prose),
            "characters" => Some(Self::Characters),
            "pacing" => Some(Self::Pacing),
            "trope_execution" => Some(Self::TropeExecution),
            "not_for_me" => Some(Self::NotForMe),
            _ => None,
        }
    }
}

/// The reader's verdict. Never absent: §49.5's schema makes a bare rating
/// unrepresentable, because a rating without a reason teaches almost nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TastingVerdict {
    Like,
    Dislike,
}

impl TastingVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Like => "like",
            Self::Dislike => "dislike",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "like" => Some(Self::Like),
            "dislike" => Some(Self::Dislike),
            _ => None,
        }
    }

    /// The sign this verdict applies to a dimension weight.
    ///
    /// §49.5: a decline is a *negative carrying its reason*, not a discarded
    /// sample. An active-learning queue that throws away the items it guessed
    /// wrong learns only from what it already knew.
    pub fn sign(self) -> f64 {
        match self {
            Self::Like => 1.0,
            Self::Dislike => -1.0,
        }
    }
}

/// One item offered to a reader.
#[derive(Debug, Clone, PartialEq)]
pub struct TastingSample {
    pub id: String,
    pub work_id: String,
    pub sample_offset: i64,
    /// The selector's confidence in its own pick, lower meaning less certain.
    pub uncertainty: f64,
}

/// A candidate pair, before it becomes a sample.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub work_id: String,
    /// 0.0 = entirely uncertain, 1.0 = entirely certain.
    pub uncertainty: f64,
}

/// Order candidates for the queue: most uncertain first.
///
/// §49.5's "picked by uncertainty, not randomly and not by popularity". Ties on
/// uncertainty break on `work_id` so the queue is a function of the data — §47.9's
/// determinism requirement does not stop applying because a new feature reads the
/// weights, and a queue whose order depends on a hash seed is untestable.
pub fn order_by_uncertainty(mut candidates: Vec<Candidate>) -> Vec<Candidate> {
    // DESCENDING on uncertainty. The first draft sorted ascending, which is a
    // perfectly reasonable-looking sort that puts the *most certain* item first --
    // the exact opposite of the mechanism. The queue would then lead with the works
    // the model already understands, which is where the reader's time is worth the
    // least. Ascending felt wrong on inspection and the test said so immediately.
    candidates.sort_by(|a, b| {
        b.uncertainty
            .partial_cmp(&a.uncertainty)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.work_id.cmp(&b.work_id))
    });
    candidates
}

/// Take up to `limit` items for one session.
///
/// §49.5: sampling is bounded per session, because a calibration queue that can
/// consume a whole reading session is a chore, and a chore gets abandoned.
pub fn take_for_session(
    candidates: Vec<Candidate>,
    limit: usize,
    already_seen_this_session: usize,
) -> Vec<Candidate> {
    let remaining = limit.saturating_sub(already_seen_this_session);
    order_by_uncertainty(candidates)
        .into_iter()
        .take(remaining)
        .collect()
}

/// How confident the selector is about a work, given the reader's weights.
///
/// Deliberately simple and deliberately explicable: §49.5's queue exists to feed
/// a human's attention into a taste profile, and a score nobody can explain is a
/// score nobody can debug when it selects badly.
///
/// The value is `1 - |agreement|`, where agreement is how strongly the reader's
/// weight for the work's dimensions points one way or the other. A work whose
/// tags the reader has never weighed lands near 0.5 — maximum uncertainty, which
/// is where a cold-start reader's whole library should sit.
pub fn uncertainty_for(weights: &crate::ranking::TagWeights, dimensions: &[String]) -> f64 {
    if dimensions.is_empty() {
        // A work with no countable dimensions (§49.2) is not uncertain, it is
        // unmeasurable. Treating it as maximally uncertain would fill a cold-start
        // reader's queue with works the ranker cannot score at all.
        return 1.0;
    }
    let mut total = 0.0;
    let mut counted = 0usize;
    for dimension in dimensions {
        if weights.has(dimension) {
            total += weights.weight_of(dimension).abs();
            counted += 1;
        }
    }
    if counted == 0 {
        return 0.5;
    }
    let mean = total / counted as f64;
    // Squash so one enormous weight cannot drive confidence to exactly 1.0 and
    // make the work permanently unpickable. 10_000bp is the arena's own scale.
    let confidence = (mean / 10_000.0).min(1.0);
    // Floored, not clamped to 0. An enormous weight drives confidence to exactly
    // 1.0, so `1.0 - confidence` is exactly 0.0, and a work at uncertainty 0.0
    // sorts last forever: the reader would never be offered it again, however much
    // they came to like it. `UNCERTAINTY_FLOOR` is the bottom of the range, so the
    // queue can always reach its least-certain-but-one work.
    (1.0 - confidence).clamp(UNCERTAINTY_FLOOR, 1.0)
}

/// Record a response. The reason is required by the type, not merely by a check.
pub async fn record_response(
    db: &Database,
    sample_id: &str,
    account_id: &str,
    verdict: TastingVerdict,
    reason: TastingReason,
    free_text: Option<&str>,
    uncertainty_at_draw: f64,
    session_id: &str,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO tasting_responses
            (id, sample_id, account_id, verdict, reason, free_text,
             uncertainty_at_draw, session_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO tasting_responses
            (id, sample_id, account_id, verdict, reason, free_text,
             uncertainty_at_draw, session_id, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(sample_id)
                .bind(account_id)
                .bind(verdict.as_str())
                .bind(reason.as_str())
                .bind(free_text)
                .bind(uncertainty_at_draw)
                .bind(session_id)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(sample_id)
                .bind(account_id)
                .bind(verdict.as_str())
                .bind(reason.as_str())
                .bind(free_text)
                .bind(uncertainty_at_draw)
                .bind(session_id)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(id)
}

/// How many samples a reader has already seen in one session.
///
/// §49.5's per-session bound, counted rather than tracked client-side so it cannot
/// be bypassed by a reader who reloads the page.
pub async fn session_count(db: &Database, account_id: &str, session_id: &str) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM tasting_responses WHERE account_id = ? AND session_id = ?",
        "SELECT COUNT(*)::bigint FROM tasting_responses WHERE account_id = $1 AND session_id = $2",
    );
    let n: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(account_id)
                .bind(session_id)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(account_id)
                .bind(session_id)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ranking::TagWeights;

    fn cand(work: &str, uncertainty: f64) -> Candidate {
        Candidate {
            work_id: work.to_owned(),
            uncertainty,
        }
    }

    /// §49.5: the queue is ordered by uncertainty, not by popularity or arrival.
    #[test]
    fn the_queue_is_ordered_most_uncertain_first() {
        let ordered = order_by_uncertainty(vec![
            cand("w-certain", 0.1),
            cand("w-unsure", 0.9),
            cand("w-middle", 0.5),
        ]);
        let ids: Vec<&str> = ordered.iter().map(|c| c.work_id.as_str()).collect();
        assert_eq!(ids, vec!["w-unsure", "w-middle", "w-certain"]);
    }

    /// §47.9's determinism, inherited. A queue whose order depends on how the
    /// candidates were collected is a queue that cannot be tested.
    #[test]
    fn the_queue_does_not_depend_on_input_order() {
        let forward = vec![cand("a", 0.4), cand("b", 0.4), cand("c", 0.1)];
        let mut reversed = forward.clone();
        reversed.reverse();
        assert_eq!(
            order_by_uncertainty(forward),
            order_by_uncertainty(reversed)
        );
    }

    /// Ties on uncertainty must still be a total order, or §47.9 fails on equal
    /// confidence — which is the *common* case for a cold-start reader, whose
    /// every work sits at 0.5.
    #[test]
    fn equal_uncertainty_breaks_on_work_id() {
        let ordered = order_by_uncertainty(vec![cand("z", 0.5), cand("a", 0.5)]);
        assert_eq!(ordered[0].work_id, "a");
        assert_eq!(ordered[1].work_id, "z");
    }

    /// A NaN uncertainty must not panic the sort or become a preferred item.
    #[test]
    fn a_nan_uncertainty_does_not_panic() {
        let ordered = order_by_uncertainty(vec![cand("a", f64::NAN), cand("b", 0.5)]);
        assert_eq!(ordered.len(), 2);
    }

    /// §49.5: sampling is bounded per session.
    #[test]
    fn the_queue_is_bounded_per_session() {
        let candidates = vec![cand("a", 0.9), cand("b", 0.8), cand("c", 0.7)];
        assert_eq!(take_for_session(candidates.clone(), 3, 0).len(), 3);
        assert_eq!(take_for_session(candidates.clone(), 3, 2).len(), 1);
        assert!(
            take_for_session(candidates, 3, 5).is_empty(),
            "a session that has already had its full quota gets nothing more"
        );
    }

    /// The bound must not underflow: `limit - seen` in usize wraps to enormous on
    /// a tampered session id.
    #[test]
    fn an_over_full_session_does_not_wrap() {
        let candidates = vec![cand("a", 0.9)];
        assert!(take_for_session(candidates, 3, 99).is_empty());
    }

    /// A cold-start reader has no weights, so every work sits at maximum
    /// uncertainty — which is what §49.5 wants: the whole library is worth
    /// sampling.
    #[test]
    fn a_reader_with_no_weights_sits_at_maximum_uncertainty() {
        let weights = TagWeights {
            weights: Vec::new(),
        };
        let u = uncertainty_for(&weights, &["space".to_owned()]);
        assert!((u - 0.5).abs() < 1e-9, "got {u}");
    }

    /// A strongly-weighted dimension makes the work *certain*, and certainty
    /// means it drops off the queue. This is the whole point of the mechanism: the
    /// reader's time goes where the model is wrong.
    #[test]
    fn a_strong_weight_lowers_uncertainty() {
        let weights = TagWeights {
            weights: vec![("space".to_owned(), 9_000.0)],
        };
        let u = uncertainty_for(&weights, &["space".to_owned()]);
        assert!(u < 0.5, "a 9000bp weight should read as confident, got {u}");
    }

    /// §49.2 meets §49.5: a work whose dimensions were all filtered out has
    /// nothing to be uncertain *about*. Filling a cold-start queue with
    /// unmeasurable works would be the queue fighting its own purpose.
    #[test]
    fn a_work_with_no_countable_dimensions_is_not_uncertain() {
        let weights = TagWeights {
            weights: Vec::new(),
        };
        assert_eq!(uncertainty_for(&weights, &[]), 1.0);
    }

    /// An enormous weight must not produce certainty exactly 1.0, or the work
    /// becomes permanently unpickable and the reader never sees it again.
    #[test]
    fn an_enormous_weight_does_not_pin_uncertainty_to_zero() {
        let weights = TagWeights {
            weights: vec![("space".to_owned(), 10_000_000.0)],
        };
        let u = uncertainty_for(&weights, &["space".to_owned()]);
        assert!(
            u > 0.0,
            "certainty exactly 0 would make it unpickable, got {u}"
        );
    }

    /// §49.5: a decline is a negative, not a deletion. The sign is what makes it
    /// trainable.
    #[test]
    fn a_dislike_is_a_negative_not_an_absence() {
        assert_eq!(TastingVerdict::Like.sign(), 1.0);
        assert_eq!(TastingVerdict::Dislike.sign(), -1.0);
    }

    #[test]
    fn a_reason_and_verdict_round_trip_through_their_database_spelling() {
        for reason in [
            TastingReason::Prose,
            TastingReason::Characters,
            TastingReason::Pacing,
            TastingReason::TropeExecution,
            TastingReason::NotForMe,
        ] {
            assert_eq!(TastingReason::parse(reason.as_str()), Some(reason));
        }
        for verdict in [TastingVerdict::Like, TastingVerdict::Dislike] {
            assert_eq!(TastingVerdict::parse(verdict.as_str()), Some(verdict));
        }
        assert_eq!(TastingReason::parse("vibes"), None);
        assert_eq!(TastingVerdict::parse("maybe"), None);
    }
}
