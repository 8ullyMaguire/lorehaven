//! The works ladder — the arena's ratings, stored (spec §12.4a).
//!
//! `docs/spec-amendments/arena-work-ladder.md` records why the arena rates works rather than the
//! reader's dimensions; the short version is that a comparison between two works is informative
//! without a taxonomy, and this catalogue has no dimension tags at all (0 of 30,981).
//!
//! # One ballot is one rating period per work
//!
//! ## Not the `rating` table
//!
//! `rating` already exists and holds a **reader's** judgement of a work — `account_id`, `pseud_id`,
//! `stars`, `is_public` — one row per reader per work. This table holds the **catalogue's**
//! aggregate judgement of a work: a Glicko-2 rating with no reader attached, built from pairwise
//! comparisons that readers never see as numbers. They answer different questions ("what did this
//! reader think" and "how good is this work, as far as the instance can tell"), and a ranking that
//! reads one when it meant the other is a bug that will look like a tuning problem. Nothing here
//! reads `rating`, and nothing in `rating`'s path writes here.
//!
//! The name is close, and deliberately so: both are ratings about works. `crates/db/src/taste_vectors.rs`
//! carries a note from before this table existed, warning that a query naming `work_ratings` could
//! only ever fail — that note is history, and this is the table it was warning about.
//!
//! The page asks for best and worst of four cards, which is five comparisons:
//!
//! ```text
//! best    beats middle1, middle2, worst
//! middle1, middle2 beat worst
//! ```
//!
//! Glicko-2 takes **every comparison from one period in a single call** — that is what its
//! signature exists for, and applying one comparison at a time inflates ratings. So each work
//! loads once, updates once with its own opponent list, and is written once. The `matches`
//! column therefore counts *comparisons*, not ballots: a ballot contributes 3 to the best card,
//! 2 to each middle and 3 to the worst.
//!
//! # Everything happens inside one transaction
//!
//! Both sides of a comparison have to be computed from the values read in the same transaction,
//! or the pair drifts: two concurrent ballots over overlapping cards would otherwise each read
//! the pre-ballot rating and the second write would erase the first. The transaction is the
//! reason the loads are not a separate helper.
//!
//! # A damaged row is reset, not propagated
//!
//! One non-finite rating makes every later period it participates in non-finite, permanently,
//! because its opponents then become non-finite too. `load` checks `Rating::is_finite` and falls
//! back to the initial rating with a warning, so a single bad write costs one work's history
//! instead of the ladder's.

use crate::{Backend, Database};
use anyhow::{Result, anyhow};
use darkhold_taste::arena::glicko::{Opponent, Outcome, Rating};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// A work's stored rating, on the Glicko scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StoredRating {
    /// Rating (`MU_INIT` 1500 for a work nobody has compared).
    pub mu: f64,
    /// Rating deviation: how much the rating is still in doubt.
    pub phi: f64,
    /// Volatility.
    pub sigma: f64,
    /// Comparisons applied, not ballots.
    pub matches: i64,
}

impl StoredRating {
    /// The rating of a work nobody has compared.
    #[must_use]
    pub fn initial() -> Self {
        let rating = Rating::initial();
        Self {
            mu: rating.mu,
            phi: rating.phi,
            sigma: rating.sigma,
            matches: 0,
        }
    }

    /// The engine's rating, for the maths.
    #[must_use]
    pub fn rating(&self) -> Rating {
        Rating {
            mu: self.mu,
            phi: self.phi,
            sigma: self.sigma,
        }
    }

    /// Whether this row is arithmetically usable.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.matches >= 0 && self.rating().is_finite()
    }
}

/// What one ballot applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BallotOutcome {
    /// Works whose rating was written.
    pub works: usize,
    /// Comparisons recorded across all of them (5 for a full four-card ballot).
    pub comparisons: usize,
}

impl BallotOutcome {
    /// A ballot that had nothing to apply.
    #[must_use]
    pub fn nothing() -> Self {
        Self {
            works: 0,
            comparisons: 0,
        }
    }
}

/// The outcome of one work's comparison with one opponent, or `None` when they were not
/// compared.
///
/// Two middles are **not** compared to each other: the reader ranked neither above the other, and
/// inventing an order between them would be recording a judgement nobody made.
#[must_use]
pub fn outcome_for(work: &str, opponent: &str, best: &str, worst: &str) -> Option<Outcome> {
    if work == opponent {
        return None;
    }
    if work == best {
        return Some(Outcome::Win);
    }
    if opponent == best {
        return Some(Outcome::Loss);
    }
    if opponent == worst {
        return Some(Outcome::Win);
    }
    if work == worst {
        return Some(Outcome::Loss);
    }
    None
}

/// The works a ballot covers, in a stable order: best, the middles as given, worst.
///
/// Duplicates are dropped and anything equal to best or worst is skipped, because the request
/// carries ids from a page and a repeated card is a client fault rather than a second
/// comparison.
#[must_use]
pub fn ballot_works<'a>(best: &'a str, worst: &'a str, middles: &'a [String]) -> Vec<&'a str> {
    let mut works: Vec<&str> = Vec::with_capacity(middles.len() + 2);
    works.push(best);
    for middle in middles {
        let middle = middle.as_str();
        if middle.is_empty() || middle == best || middle == worst {
            continue;
        }
        if !works.contains(&middle) {
            works.push(middle);
        }
    }
    if !worst.is_empty() && !works.contains(&worst) {
        works.push(worst);
    }
    works
}

/// Load one work's rating, defaulting to the initial rating for a work never compared.
pub async fn load(db: &Database, work_id: &str) -> Result<StoredRating> {
    let row: Option<(f64, f64, f64, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as("SELECT mu, phi, sigma, matches FROM work_ratings WHERE work_id = ?")
                .bind(work_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            // `$1::uuid` and not `$1`: work_id is UUID on PostgreSQL and TEXT on SQLite, and a
            // text bind against a uuid column fails with "operator does not exist: uuid = text"
            // — which names the operator rather than the missing cast. The cast is per arm
            // because SQLite has no uuid type to cast to.
            sqlx::query_as(
                "SELECT mu, phi, sigma, matches FROM work_ratings WHERE work_id = $1::uuid",
            )
            .bind(work_id)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    Ok(stored_from_row(work_id, row))
}

/// Apply one ballot: five comparisons, one period per work, one transaction.
///
/// `weight` is the voter's influence, 0..=1; pass `1.0` for an ordinary reader. A weight of
/// zero is *not* "no influence" in the rating maths — see `glicko::Opponent::weighted` — so a
/// caller that wants no influence must not call this at all.
///
/// Returns how many works and comparisons were written. A ballot whose best and worst are the
/// same work, or missing, applies nothing and says so rather than erroring: an unusable ballot
/// is a client fault the route has already answered, not a server failure.
pub async fn record_ballot(
    db: &Database,
    best: &str,
    worst: &str,
    middles: &[String],
    weight: f64,
) -> Result<BallotOutcome> {
    if best.is_empty() || worst.is_empty() || best == worst {
        return Ok(BallotOutcome::nothing());
    }

    let works = ballot_works(best, worst, middles);
    if works.len() < 2 {
        return Ok(BallotOutcome::nothing());
    }

    let rated_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| String::from("1970-01-01T00:00:00Z"));

    // Per work: (new rating, comparisons in this period).
    let mut written_works = 0usize;
    let mut total_comparisons = 0usize;

    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let mut tx = pool.begin().await?;
            // Load the whole ballot's snapshot FIRST, so every comparison in this period is
            // scored against the ratings as they stood when it began. Loading per work inside
            // the write loop would score the second work against the first work's NEW rating,
            // and the pair's symmetry would be lost.
            let mut snapshot: Vec<(&str, StoredRating)> = Vec::with_capacity(works.len());
            for work in &works {
                let row: Option<(f64, f64, f64, i64)> = sqlx::query_as(
                    "SELECT mu, phi, sigma, matches FROM work_ratings WHERE work_id = ?",
                )
                .bind(*work)
                .fetch_optional(&mut *tx)
                .await?;
                snapshot.push((*work, stored_from_row(*work, row)));
            }
            for (work, stored) in &snapshot {
                let opponents = opponents_for(work, &snapshot, best, worst, weight);
                let comparisons = opponents.len() as i64;
                let updated = stored.rating().update(&opponents);
                let written = StoredRating {
                    mu: updated.mu,
                    phi: updated.phi,
                    sigma: updated.sigma,
                    matches: stored.matches + comparisons,
                };
                sqlx::query(
                    "INSERT INTO work_ratings (work_id, mu, phi, sigma, matches, rated_at)
                     VALUES (?, ?, ?, ?, ?, ?)
                     ON CONFLICT (work_id) DO UPDATE SET
                         mu = excluded.mu, phi = excluded.phi, sigma = excluded.sigma,
                         matches = excluded.matches, rated_at = excluded.rated_at",
                )
                .bind(*work)
                .bind(written.mu)
                .bind(written.phi)
                .bind(written.sigma)
                .bind(written.matches)
                .bind(&rated_at)
                .execute(&mut *tx)
                .await?;
                written_works += 1;
                total_comparisons += comparisons as usize;
            }
            tx.commit().await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let mut tx = pool.begin().await?;
            let mut snapshot: Vec<(&str, StoredRating)> = Vec::with_capacity(works.len());
            for work in &works {
                let row: Option<(f64, f64, f64, i64)> = sqlx::query_as(
                    "SELECT mu, phi, sigma, matches FROM work_ratings WHERE work_id = $1::uuid",
                )
                .bind(*work)
                .fetch_optional(&mut *tx)
                .await?;
                snapshot.push((*work, stored_from_row(*work, row)));
            }
            for (work, stored) in &snapshot {
                let opponents = opponents_for(work, &snapshot, best, worst, weight);
                let comparisons = opponents.len() as i64;
                let updated = stored.rating().update(&opponents);
                let written = StoredRating {
                    mu: updated.mu,
                    phi: updated.phi,
                    sigma: updated.sigma,
                    matches: stored.matches + comparisons,
                };
                // Each statement numbers its own placeholders from $1: sqlx does not continue
                // numbering across statements, and PostgreSQL rejects a $7 in a six-bind
                // statement as "there is no parameter $7".
                sqlx::query(
                    "INSERT INTO work_ratings (work_id, mu, phi, sigma, matches, rated_at)
                     VALUES ($1::uuid, $2, $3, $4, $5, $6)
                     ON CONFLICT (work_id) DO UPDATE SET
                         mu = EXCLUDED.mu, phi = EXCLUDED.phi, sigma = EXCLUDED.sigma,
                         matches = EXCLUDED.matches, rated_at = EXCLUDED.rated_at",
                )
                .bind(*work)
                .bind(written.mu)
                .bind(written.phi)
                .bind(written.sigma)
                .bind(written.matches)
                .bind(&rated_at)
                .execute(&mut *tx)
                .await?;
                written_works += 1;
                total_comparisons += comparisons as usize;
            }
            tx.commit().await?;
        }
    }

    Ok(BallotOutcome {
        works: written_works,
        comparisons: total_comparisons,
    })
}

/// Decode a stored row, resetting a damaged one to the initial rating.
fn stored_from_row(work: &str, row: Option<(f64, f64, f64, i64)>) -> StoredRating {
    let Some((mu, phi, sigma, matches)) = row else {
        return StoredRating::initial();
    };
    let stored = StoredRating {
        mu,
        phi,
        sigma,
        matches,
    };
    if stored.is_finite() {
        stored
    } else {
        tracing::warn!(
            work,
            "work rating is not finite; resetting to the initial rating rather than spreading a \
             non-finite value to every later opponent"
        );
        StoredRating::initial()
    }
}

/// One work's opponents for this ballot, taken from the period's snapshot.
///
/// The opponent's **rating comes from the snapshot**, not from the work being written: a rating
/// period is scored against the ratings as they stood when it began. Passing a placeholder
/// (0.0, as a draft of this function did) makes every comparison an expected win for a
/// 1500-rated work, so nothing moves and the ladder silently records zero information.
fn opponents_for(
    work: &str,
    snapshot: &[(&str, StoredRating)],
    best: &str,
    worst: &str,
    weight: f64,
) -> Vec<Opponent> {
    let mut opponents = Vec::with_capacity(snapshot.len().saturating_sub(1));
    for (opponent_id, opponent) in snapshot {
        let Some(outcome) = outcome_for(work, opponent_id, best, worst) else {
            continue;
        };
        opponents.push(Opponent::weighted(
            opponent.mu,
            opponent.phi,
            outcome,
            weight,
        ));
    }
    opponents
}

/// The leaderboard: highest-rated works first.
///
/// Ordered by `mu` and then by `phi` ascending, because two works at the same rating are not
/// equally known and the more certain one is the better answer. Nothing here filters by
/// `matches`: a work rated once at 1600 is shown, with its wide `phi` in the row, rather than
/// hidden behind an arbitrary floor.
pub async fn top_rated(db: &Database, limit: i64) -> Result<Vec<(String, StoredRating)>> {
    if limit <= 0 {
        return Err(anyhow!("limit must be positive"));
    }
    let rows: Vec<(String, f64, f64, f64, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(
                "SELECT work_id, mu, phi, sigma, matches FROM work_ratings
                 ORDER BY mu DESC, phi ASC LIMIT ?",
            )
            .bind(limit)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        Backend::Postgres => {
            sqlx::query_as(
                "SELECT work_id::text, mu, phi, sigma, matches FROM work_ratings
                 ORDER BY mu DESC, phi ASC LIMIT $1",
            )
            .bind(limit)
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|(work_id, mu, phi, sigma, matches)| {
            (
                work_id,
                StoredRating {
                    mu,
                    phi,
                    sigma,
                    matches,
                },
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn middles(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    #[test]
    fn a_four_card_ballot_is_five_comparisons() {
        let cards = middles(&["m1", "m2"]);
        let works = ballot_works("best", "worst", &cards);
        assert_eq!(works, vec!["best", "m1", "m2", "worst"]);

        // best: beats both middles and the worst
        let best: Vec<Outcome> = works
            .iter()
            .filter_map(|opponent| outcome_for("best", opponent, "best", "worst"))
            .collect();
        assert_eq!(best.len(), 3);

        // a middle: lost to best, beat worst — and is NOT compared to the other middle
        assert_eq!(
            outcome_for("m1", "best", "best", "worst"),
            Some(Outcome::Loss)
        );
        assert_eq!(
            outcome_for("m1", "worst", "best", "worst"),
            Some(Outcome::Win)
        );
        assert_eq!(outcome_for("m1", "m2", "best", "worst"), None);

        // worst: lost to everything it met
        let worst: Vec<Outcome> = works
            .iter()
            .filter_map(|opponent| outcome_for("worst", opponent, "best", "worst"))
            .collect();
        assert_eq!(worst, vec![Outcome::Loss, Outcome::Loss, Outcome::Loss]);

        // 3 + 2 + 2 + 3
        let total: usize = works
            .iter()
            .map(|work| {
                works
                    .iter()
                    .filter(|opponent| outcome_for(work, opponent, "best", "worst").is_some())
                    .count()
            })
            .sum();
        assert_eq!(total, 10, "a four-card ballot is ten directed comparisons");
    }

    #[test]
    fn a_ballot_with_only_best_and_worst_is_one_comparison_each() {
        let works = ballot_works("best", "worst", &[]);
        assert_eq!(works, vec!["best", "worst"]);
        assert_eq!(
            outcome_for("best", "worst", "best", "worst"),
            Some(Outcome::Win)
        );
        assert_eq!(
            outcome_for("worst", "best", "best", "worst"),
            Some(Outcome::Loss)
        );
    }

    #[test]
    fn a_repeated_card_is_not_a_second_comparison() {
        // The request carries ids from a page; a client that names the best card as a middle
        // must not give it two wins over the same work.
        let cards = middles(&["best", "m1", "m1", "worst"]);
        let works = ballot_works("best", "worst", &cards);
        assert_eq!(works, vec!["best", "m1", "worst"]);
    }

    #[test]
    fn the_initial_rating_is_the_engines_initial_rating() {
        let stored = StoredRating::initial();
        assert_eq!(stored.matches, 0);
        assert_eq!(stored.rating(), Rating::initial());
        assert!(stored.is_finite());
    }

    #[test]
    fn a_non_finite_row_is_not_finite() {
        for stored in [
            StoredRating {
                mu: f64::NAN,
                phi: 350.0,
                sigma: 0.06,
                matches: 1,
            },
            StoredRating {
                mu: 1500.0,
                phi: f64::INFINITY,
                sigma: 0.06,
                matches: 1,
            },
            StoredRating {
                mu: 1500.0,
                phi: 350.0,
                sigma: 0.06,
                matches: -1,
            },
        ] {
            assert!(!stored.is_finite(), "{stored:?} should not be finite");
        }
    }
}
