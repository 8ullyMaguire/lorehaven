//! Tag confirmation and the gravity contribution cap (spec §49.2, M45-16).
//!
//! ## What this module decides
//!
//! Which of a work's tags are allowed to move a reader's ranking. §49.2's rule
//! is that only reader- or wrangler-confirmed tags count, capped per work, and
//! that a reader-flagged inaccurate tag stops counting immediately rather than at
//! wrangling-queue resolution.
//!
//! ## Where it is applied, and why it is not in the ranker
//!
//! §49.7 requires the filter to be applied **where the weights are read**, not by
//! cleaning the stored weights. Cleaning them would mean a newly-confirmed tag
//! retroactively rewrites a profile that was calibrated without it, and the
//! reader would be unable to explain why their feed changed. So this module
//! offers a filtered read, and `rank_works` is untouched — §47.2's contract holds
//! and §49 decides only what the ranker is *told*.
//!
//! ## The cap is derived, never stored
//!
//! Contribution is `min(counted, cap)`, a projection over the existing rows.
//! Storing the surviving set would add a fifth place for gravity to be wrong and
//! would need recomputing on every confirmation. When the cap bites, the
//! survivors are chosen by `node_id` so the result is a function of the data
//! rather than of the order the database returned.

use crate::{Backend, Database};
use anyhow::Result;

/// The default per-work contribution cap (§49.2, instance config §38.6).
pub const DEFAULT_CONTRIBUTION_CAP: i64 = 5;

/// How a work's tag was confirmed (migration 0099).
///
/// `Unconfirmed` is the state every pre-0099 row is in, which is correct: those
/// tags were author-applied and nothing recorded a reader agreeing with them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagConfirmation {
    /// Applied by an author, or inferred. Displayed, searchable, not counted.
    Unconfirmed,
    /// A reader confirmed this tag describes the work.
    Reader,
    /// A wrangler confirmed it (Milestone 29's queue).
    Wrangler,
    /// Flagged as inaccurate. Excluded at once, and queued for wrangling.
    Inaccurate,
}

impl TagConfirmation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unconfirmed => "unconfirmed",
            Self::Reader => "reader",
            Self::Wrangler => "wrangler",
            Self::Inaccurate => "inaccurate",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "unconfirmed" => Some(Self::Unconfirmed),
            "reader" => Some(Self::Reader),
            "wrangler" => Some(Self::Wrangler),
            "inaccurate" => Some(Self::Inaccurate),
            _ => None,
        }
    }

    /// Whether a tag in this state contributes to gravity.
    ///
    /// `Inaccurate` is false for a reason §49.2 spells out: the reader who
    /// noticed should not keep paying for the tag while a queue works through it.
    pub fn counts_toward_gravity(self) -> bool {
        matches!(self, Self::Reader | Self::Wrangler)
    }
}

/// A tag on a work, with the state that decides whether it counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkTag {
    pub node_id: String,
    pub canonical: String,
    pub confirmation: TagConfirmation,
}

/// Record a confirmation against one work-tag pair.
///
/// Returns `false` when the pair does not exist. That is not an error: §49.2
/// changes what counts toward gravity for tags a work *has*, and inventing a
/// row here would let a confirmation attach to a tag the work does not carry.
/// A caller that means to add the tag adds it through `taxonomy` first.
pub async fn confirm(
    db: &Database,
    work_id: &str,
    node_id: &str,
    confirmation: TagConfirmation,
    flagged_by: Option<&str>,
) -> Result<bool> {
    // The states differ only in whether they record WHO flagged it. One statement
    // with a conditional, not two near-duplicate UPDATEs: a duplicated query is a
    // second thing that can drift out of step with the first, and the drift would
    // be invisible until a re-confirmed tag kept claiming a flagger who had since
    // agreed with it.
    //
    // CASE is portable -- SQLite and PostgreSQL evaluate it identically, so there
    // is no dialect branch here to get wrong, and nothing numeric is involved, so
    // PostgreSQL's int4/bigint traps do not apply.
    let sql = db.sql(
        "UPDATE work_tags
            SET confirmation = ?,
                flagged_by = CASE WHEN ? = 'inaccurate' THEN ? ELSE NULL END,
                flagged_at = CASE WHEN ? = 'inaccurate' THEN CURRENT_TIMESTAMP ELSE NULL END
          WHERE work_id = ? AND node_id = ?",
        "UPDATE work_tags
            SET confirmation = $1,
                flagged_by = CASE WHEN $1 = 'inaccurate' THEN $2 ELSE NULL END,
                flagged_at = CASE WHEN $1 = 'inaccurate' THEN CURRENT_TIMESTAMP ELSE NULL END
          WHERE work_id = $3::uuid AND node_id = $4",
    );
    let state = confirmation.as_str();
    let who = flagged_by.unwrap_or("unknown");
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(state)
            .bind(state)
            .bind(who)
            .bind(state)
            .bind(work_id)
            .bind(node_id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(state)
            .bind(who)
            .bind(work_id)
            .bind(node_id)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// Every tag on a work, with its confirmation state, ordered by `node_id`.
///
/// Ordered rather than unordered because the cap picks survivors by `node_id`,
/// and a caller that displays this list should see the same order on both
/// engines.
pub async fn tags_with_confirmation(db: &Database, work_id: &str) -> Result<Vec<WorkTag>> {
    let sql = db.sql(
        "SELECT wt.node_id, tn.canonical, wt.confirmation
           FROM work_tags wt
           JOIN taxonomy_nodes tn ON tn.id = wt.node_id
          WHERE wt.work_id = ? AND tn.kind = 'tag'
          ORDER BY wt.node_id ASC",
        "SELECT wt.node_id, tn.canonical, wt.confirmation
           FROM work_tags wt
           JOIN taxonomy_nodes tn ON tn.id = wt.node_id
          WHERE wt.work_id = $1::uuid AND tn.kind = 'tag'
          ORDER BY wt.node_id ASC",
    );
    // `query_as` into a tuple rather than `query` into `Row`, because the two
    // arms below otherwise return different row types and the match does not
    // type-check. This is the shape `taxonomy::tag_names_for_work` already uses.
    let rows: Vec<(String, String, String)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    let mut out = Vec::with_capacity(rows.len());
    for (node_id, canonical, state) in rows {
        // A confirmation value the CHECK constraint should have prevented. Skipped
        // rather than defaulted to Unconfirmed: a value this code does not
        // understand must not silently become "does not count", because that would
        // drop a tag out of every reader's ranking with no error anywhere.
        let Some(confirmation) = TagConfirmation::parse(&state) else {
            continue;
        };
        out.push(WorkTag {
            node_id,
            canonical,
            confirmation,
        });
    }
    Ok(out)
}

/// The tag names of a work that count toward gravity, capped at `cap`.
///
/// This is what the discovery route feeds `rank_works` instead of
/// `taxonomy::tag_names_for_work`. Returning canonical names (lowercased by the
/// caller, as before) keeps the ranker's dimension keys unchanged, so §47.2's
/// contract and every test written against it still hold.
pub async fn gravity_contributing_tags(
    db: &Database,
    work_id: &str,
    cap: i64,
) -> Result<Vec<String>> {
    Ok(
        apply_contribution_cap(tags_with_confirmation(db, work_id).await?, cap)
            .into_iter()
            .map(|t| t.canonical)
            .collect(),
    )
}

/// Apply the per-work cap to a tag set.
///
/// Pure, so the cap's behaviour is testable without a database and so both
/// dialects provably share it — the cap is a rule, not a query.
///
/// Sorting by `node_id` before truncating is what makes the cap deterministic.
/// Without it, which tags survive a cap of 5 on a 50-tag work would depend on the
/// order the engine returned rows in, and §49.8 requires the same answer on both
/// engines.
pub fn apply_contribution_cap(mut tags: Vec<WorkTag>, cap: i64) -> Vec<WorkTag> {
    let counted: Vec<WorkTag> = tags
        .drain(..)
        .filter(|t| t.confirmation.counts_toward_gravity())
        .collect();
    if cap < 0 {
        return Vec::new();
    }
    let mut counted = counted;
    counted.sort_by(|a, b| a.node_id.cmp(&b.node_id));
    counted.truncate(cap as usize);
    counted
}
#[cfg(test)]
mod tests {
    use super::*;

    fn tag(node: &str, state: TagConfirmation) -> WorkTag {
        WorkTag {
            node_id: node.to_owned(),
            canonical: node.to_owned(),
            confirmation: state,
        }
    }

    /// §49.7: a tag a reader never confirmed never moves their ranking. The
    /// whole point of the column, so it is the first test rather than a
    /// consequence of the cap.
    #[test]
    fn an_unconfirmed_tag_contributes_nothing() {
        let tags = vec![tag("space", TagConfirmation::Unconfirmed)];
        assert!(apply_contribution_cap(tags, DEFAULT_CONTRIBUTION_CAP).is_empty());
    }

    /// §49.2's immediate-exclusion rule. The reader who flagged it must not keep
    /// paying for it while the wrangling queue works through it.
    #[test]
    fn a_flagged_inaccurate_tag_contributes_nothing() {
        let tags = vec![
            tag("space", TagConfirmation::Reader),
            tag("knitting", TagConfirmation::Inaccurate),
        ];
        let kept = apply_contribution_cap(tags, DEFAULT_CONTRIBUTION_CAP);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].node_id, "space");
    }

    /// §49.8: with the cap at 5, the sixth onward contributes nothing.
    #[test]
    fn the_cap_binds_at_the_configured_limit() {
        let tags: Vec<WorkTag> = (0..12)
            .map(|i| tag(&format!("t{i:02}"), TagConfirmation::Reader))
            .collect();
        let kept = apply_contribution_cap(tags, 5);
        assert_eq!(kept.len(), 5, "cap is on contribution, not on display");
    }

    /// Which tags survive a cap must not depend on the order the database
    /// returned rows in, or §49.8's "same answer on both engines" fails for a
    /// reason that has nothing to do with the two engines disagreeing.
    #[test]
    fn the_cap_survivors_do_not_depend_on_input_order() {
        let forward: Vec<WorkTag> = (0..10)
            .map(|i| tag(&format!("t{i:02}"), TagConfirmation::Reader))
            .collect();
        let mut reversed = forward.clone();
        reversed.reverse();
        assert_eq!(
            apply_contribution_cap(forward, 3),
            apply_contribution_cap(reversed, 3),
            "same tags, different arrival order, different survivors"
        );
    }

    /// An unconfirmed tag must not consume a cap slot. Otherwise a work could
    /// push its real tags out of gravity by padding with unconfirmed ones, which
    /// is the tag-stuffing attack §49.2 exists to close, arriving through the cap.
    ///
    /// The filler ids sort BEFORE the real ones on purpose. The first version used
    /// `u*` for filler and `r*` for real, and `r` < `u`, so the real tags came
    /// first and the test passed even with the confirmation filter deleted. It was
    /// green against broken code, which is the failure mode `goal.md` calls out:
    /// a test that cannot fail on the defect it names is decoration.
    #[test]
    fn unconfirmed_tags_do_not_consume_cap_slots() {
        let tags: Vec<WorkTag> = (0..5)
            .map(|i| tag(&format!("a-filler{i}"), TagConfirmation::Unconfirmed))
            .chain(
                (0..3)
                    .map(|i| tag(&format!("z-real{i}"), TagConfirmation::Reader))
                    .collect::<Vec<_>>(),
            )
            .collect();
        let kept = apply_contribution_cap(tags, 3);
        assert_eq!(kept.len(), 3, "all three confirmed tags survive a cap of 3");
        assert!(kept
            .iter()
            .all(|t| t.confirmation == TagConfirmation::Reader));
    }

    #[test]
    fn a_wrangler_confirmation_counts_and_a_reader_one_does_not_stop_it() {
        assert!(TagConfirmation::Wrangler.counts_toward_gravity());
        assert!(TagConfirmation::Reader.counts_toward_gravity());
        assert!(!TagConfirmation::Unconfirmed.counts_toward_gravity());
        assert!(!TagConfirmation::Inaccurate.counts_toward_gravity());
    }

    /// Round-tripping the database spelling. A drift here would write a value the
    /// CHECK constraint rejects at runtime rather than at compile time.
    #[test]
    fn a_confirmation_round_trips_through_its_database_spelling() {
        for state in [
            TagConfirmation::Unconfirmed,
            TagConfirmation::Reader,
            TagConfirmation::Wrangler,
            TagConfirmation::Inaccurate,
        ] {
            assert_eq!(TagConfirmation::parse(state.as_str()), Some(state));
        }
        assert_eq!(TagConfirmation::parse("nonsense"), None);
    }

    /// A cap of zero is a legitimate operator setting, not a mistake: it is how
    /// an instance turns gravity off without uninstalling the ranker.
    #[test]
    fn a_zero_cap_contributes_nothing_and_does_not_panic() {
        let tags = vec![tag("space", TagConfirmation::Reader)];
        assert!(apply_contribution_cap(tags, 0).is_empty());
    }
}
