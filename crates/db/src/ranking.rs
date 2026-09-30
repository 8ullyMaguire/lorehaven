//! Ranking substrate — spec §47.
//!
//! This module exists because §47.3 cannot be retrofitted. An impression whose
//! selection probability was not recorded cannot be recovered afterwards,
//! because the counterfactual that would have been logged no longer exists — so
//! the log is a precondition of the impression rather than a side effect of it,
//! and everything here is shaped to keep it that way.
//!
//! The pipeline in §47.2 is fixed, and a stage may not reorder it. That is not a
//! style preference: the propensity log only describes offline evaluation if it
//! describes the distribution that actually produced the output.

use crate::{sql_owned, Backend, Database, Result};
use lorehaven_domain::WorkId;

/// Why a work was shown, and from which pool.
///
/// Not cosmetic. Offline evaluation (M45-13) corrects for selection bias using
/// `propensity`, so a `Ranked` impression and an `Exploration` impression are
/// not interchangeable samples: they were drawn from different populations and
/// weighting them identically would bias the estimate in a direction nothing
/// downstream can detect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// Chosen by score from the aligned set.
    Ranked,
    /// Uniform-random from the eligible-but-unshown set (§47.3).
    Exploration,
    /// Guarantee of opportunity for a zero-impression work (§47.5).
    ExposureFloor,
}

impl SlotKind {
    /// The database spelling.
    ///
    /// The single source for the string that migration 0098's CHECK constraint
    /// accepts, so the two cannot drift apart.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ranked => "ranked",
            Self::Exploration => "exploration",
            Self::ExposureFloor => "exposure_floor",
        }
    }
}

/// One recorded impression.
///
/// Every ordered row of a ranking result is one of these, and §47.8 asserts the
/// pairing — a `Ranked` with no `Impression` row behind it is a bug, not a
/// default.
#[derive(Debug, Clone, PartialEq)]
pub struct Impression {
    pub work_id: WorkId,
    pub slot_kind: SlotKind,
    /// Probability this reader would have been shown this work, in (0, 1].
    pub propensity: f64,
    pub score: f64,
    /// Which stage placed the row.
    pub stage: String,
}

/// An ordered row plus what justifies its position.
#[derive(Debug, Clone, PartialEq)]
pub struct Ranked {
    pub work_id: WorkId,
    pub score: f64,
    pub stage: String,
}

/// Write the impression for a single row.
///
/// Separate from `rank_works` so the transaction boundary is explicit. §47.3
/// requires the log and the impression to commit together, and a caller that
/// can simply forget to log cannot satisfy that.
///
/// `slot` must already be a persisted `recommendation_slots` row (§33.3 records
/// it): this writes the two columns migration 0098 added to that table —
/// `slot_kind` and `propensity` — rather than inserting a row of its own. The
/// reason is that `recommendation_slots` is already the record of what was
/// served, and a parallel `impressions` table would be a second opinion about
/// the same event with nothing keeping the two in step.
pub async fn log_impression(db: &Database, slot_id: &str, impression: &Impression) -> Result<()> {
    let sqlite = "UPDATE recommendation_slots
        SET slot_kind = ?, propensity = ?
        WHERE id = ?";
    let postgres = "UPDATE recommendation_slots
        SET slot_kind = $1, propensity = $2
        WHERE id = $3";
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(sqlite)
                .bind(impression.slot_kind.as_str())
                .bind(impression.propensity)
                .bind(slot_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            let sql = sql_owned(db, sqlite.to_owned(), postgres.to_owned());
            sqlx::query(&sql)
                .bind(impression.slot_kind.as_str())
                .bind(impression.propensity)
                .bind(slot_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Whether an interaction was earned by ranking or arrived through an incentive.
///
/// §47.4: ranking counts only `Earned`. This is the retrofit-critical half of
/// the module — an incentive introduced after the fact cannot be subtracted from
/// an interaction already recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionKind {
    Earned,
    Incentivized,
}

impl InteractionKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Earned => "earned",
            Self::Incentivized => "incentivized",
        }
    }

    /// Sources that are `Incentivized` **by definition** (§47.4).
    ///
    /// By definition, not "usually": the reader was routed to the work by the
    /// incentive rather than by the ranking, which is the same condition.
    ///
    /// The default is deliberately `Earned` — the kind that is *not* discounted.
    /// That makes adding a new incentive a visible act in this match rather than
    /// a silent omission, which is the failure the clause exists to prevent: a
    /// new reading-club feature that nobody remembered to add here would inflate
    /// ranking exactly as silently as if it had.
    #[must_use]
    pub fn for_source(source: &str) -> Self {
        match source {
            "reading_club" | "topic_subscription" | "bounty" | "taste_notification" => {
                Self::Incentivized
            }
            _ => Self::Earned,
        }
    }
}

/// The primary key of one already-logged interaction row.
///
/// A struct rather than a single `&str`, because the two tables do not share a
/// key shape and a single string invites binding the same value into three
/// unrelated columns — which compiles, runs, and updates zero rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionRow {
    /// `work_view_log`, keyed by (work_id, viewer_hash, viewed_at).
    View {
        work_id: String,
        viewer_hash: String,
        viewed_at: String,
    },
    /// `work_kudos`, keyed by (work_id, account_id).
    Kudos { work_id: String, account_id: String },
}

/// Record an interaction's kind and obscurity on an already-logged read or
/// kudos (`work_view_log` / `work_kudos`, migration 0068).
///
/// `obscurity_at_read` is a parameter rather than something computed here.
/// Measuring it inside this function from a later query would be the §47.7
/// defect: the weight has to be the one in force when the engagement happened,
/// because a work that was obscure then and popular now must still score as an
/// obscure read. Recomputing would pay whoever arrived early, which is
/// measurable and would be gamed within a day.
///
/// The update targets the existing tables rather than a new `interactions` table
/// because §47.4's distinction has to land on the rows that already exist. A
/// single new table would leave every historical read invisible to ranking, which
/// is the same class of bug as the missing propensity column: a record that
/// exists in one place and not the other.
pub async fn record_interaction(
    db: &Database,
    row: &InteractionRow,
    source: &str,
    obscurity_at_read: f64,
) -> Result<InteractionKind> {
    let kind = InteractionKind::for_source(source);
    // The table name is chosen by matching the enum, so no caller-supplied
    // string ever reaches an identifier position.
    match row {
        InteractionRow::View {
            work_id,
            viewer_hash,
            viewed_at,
        } => {
            let sqlite = "UPDATE work_view_log
                SET kind = ?, source = ?, obscurity_at_read = ?
                WHERE work_id = ? AND viewer_hash = ? AND viewed_at = ?";
            let postgres = "UPDATE work_view_log
                SET kind = $1, source = $2, obscurity_at_read = $3
                WHERE work_id = $4 AND viewer_hash = $5 AND viewed_at = $6";
            match db.backend() {
                Backend::Sqlite => {
                    sqlx::query(sqlite)
                        .bind(kind.as_str())
                        .bind(source)
                        .bind(obscurity_at_read)
                        .bind(work_id)
                        .bind(viewer_hash)
                        .bind(viewed_at)
                        .execute(db.sqlite_pool().expect("sqlite"))
                        .await?;
                }
                Backend::Postgres => {
                    let sql = sql_owned(db, sqlite.to_owned(), postgres.to_owned());
                    sqlx::query(&sql)
                        .bind(kind.as_str())
                        .bind(source)
                        .bind(obscurity_at_read)
                        .bind(work_id)
                        .bind(viewer_hash)
                        .bind(viewed_at)
                        .execute(db.postgres_pool().expect("postgres"))
                        .await?;
                }
            }
        }
        InteractionRow::Kudos {
            work_id,
            account_id,
        } => {
            let sqlite = "UPDATE work_kudos
                SET kind = ?, source = ?, obscurity_at_read = ?
                WHERE work_id = ? AND account_id = ?";
            let postgres = "UPDATE work_kudos
                SET kind = $1, source = $2, obscurity_at_read = $3
                WHERE work_id = $4 AND account_id = $5";
            match db.backend() {
                Backend::Sqlite => {
                    sqlx::query(sqlite)
                        .bind(kind.as_str())
                        .bind(source)
                        .bind(obscurity_at_read)
                        .bind(work_id)
                        .bind(account_id)
                        .execute(db.sqlite_pool().expect("sqlite"))
                        .await?;
                }
                Backend::Postgres => {
                    let sql = sql_owned(db, sqlite.to_owned(), postgres.to_owned());
                    sqlx::query(&sql)
                        .bind(kind.as_str())
                        .bind(source)
                        .bind(obscurity_at_read)
                        .bind(work_id)
                        .bind(account_id)
                        .execute(db.postgres_pool().expect("postgres"))
                        .await?;
                }
            }
        }
    }
    Ok(kind)
}

/// Scout value: credit for engaging with a work that later earns a high rating,
/// weighted by how obscure it was at the time of engagement (§47.7).
///
/// Deliberately a **pure function** and deliberately *not* a ranking input. It
/// computes curation credit; it does not influence `rank_works`. A reader's
/// recommendations do not improve because they scouted well, and wiring this
/// into ranking closes a loop where being scouted raises reach, which raises the
/// score that pays the scout.
#[must_use]
pub fn scout_value(obscurity_at_read: f64, later_rating: f64) -> f64 {
    // NaN is filtered BEFORE the clamp, and it has to be: `f64::clamp` propagates
    // NaN rather than clamping it (verified — `f64::NAN.clamp(0.0, 1.0) == NaN`,
    // while `5.0.clamp(0.0, 1.0) == 1.0`). So a single NaN rating would produce a
    // NaN credit value, and summing NaN into a ledger poisons every total it
    // touches — a number that is not merely wrong but unreadable, and which no
    // later comparison would catch.
    //
    // NaN becomes 0.0 rather than 1.0. Under-crediting a single engagement loses
    // one payout; over-crediting mints credit that was never earned, and minted
    // credit is the thing the closed loop in §17 is built to prevent.
    fn sanitised(value: f64) -> f64 {
        if value.is_nan() {
            0.0
        } else {
            value.clamp(0.0, 1.0)
        }
    }
    sanitised(obscurity_at_read) * sanitised(later_rating)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_kind_round_trips_through_its_database_spelling() {
        // Migration 0098's CHECK constraint is the other end of this: if a
        // spelling drifts, the INSERT fails at runtime rather than logging a row
        // the offline evaluation cannot classify.
        for kind in [
            SlotKind::Ranked,
            SlotKind::Exploration,
            SlotKind::ExposureFloor,
        ] {
            assert!(["ranked", "exploration", "exposure_floor"].contains(&kind.as_str()));
        }
    }

    #[test]
    fn the_four_incentive_sources_are_incentivized_by_definition() {
        for source in [
            "reading_club",
            "topic_subscription",
            "bounty",
            "taste_notification",
        ] {
            assert_eq!(
                InteractionKind::for_source(source),
                InteractionKind::Incentivized,
                "{source} is routed by an incentive, not by the ranking"
            );
        }
    }

    #[test]
    fn an_unlisted_source_is_earned_and_that_is_the_deliberate_default() {
        // The default is the kind that is NOT discounted, so a forgotten source
        // inflates ranking. That is the choice §47.4 makes on purpose: a new
        // incentive must be added here visibly rather than be discounted by
        // accident. Asserted so the default cannot be flipped quietly.
        assert_eq!(
            InteractionKind::for_source("ranking"),
            InteractionKind::Earned
        );
        assert_eq!(InteractionKind::for_source(""), InteractionKind::Earned);
    }

    #[test]
    fn scout_value_is_clamped_at_both_ends() {
        assert!((scout_value(0.0, 1.0) - 0.0).abs() < f64::EPSILON);
        assert!((scout_value(1.0, 1.0) - 1.0).abs() < f64::EPSILON);
        // Out-of-range inputs clamp rather than propagate, so one bad rating
        // cannot mint unbounded credit.
        assert!((scout_value(-5.0, 1.0) - 0.0).abs() < f64::EPSILON);
        assert!((scout_value(1.0, 9.0) - 1.0).abs() < f64::EPSILON);
        // NaN must become 0.0, and this is why it is a separate assertion from the
        // range clamps above: `f64::clamp` propagates NaN, so a clamp-only
        // implementation returns NaN here, and a NaN summed into a credit ledger
        // poisons every total it touches. Written as an equality rather than
        // `is_finite()` so the expected direction is pinned: under-credit, never
        // over-credit.
        assert_eq!(
            scout_value(f64::NAN, 1.0),
            0.0,
            "a NaN obscurity must cost the engagement its credit, not mint one"
        );
        assert_eq!(scout_value(1.0, f64::NAN), 0.0);
        assert_eq!(scout_value(f64::NAN, f64::NAN), 0.0);
        // Infinity is clamped normally, not treated as NaN.
        assert!((scout_value(f64::INFINITY, 1.0) - 1.0).abs() < f64::EPSILON);
        assert!((scout_value(f64::NEG_INFINITY, 1.0) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn scout_value_is_zero_for_a_work_that_was_already_popular() {
        // The reason the obscurity weight exists: without it the mechanism pays
        // the largest audience, which is the opposite of scouting.
        assert!((scout_value(0.0, 1.0) - 0.0).abs() < f64::EPSILON);
    }
}
