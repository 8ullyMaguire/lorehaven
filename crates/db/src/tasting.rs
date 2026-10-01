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
use sqlx::Row;

// ## The column types in this module, because they are not uniform
//
// This module cost four runs of "operator does not exist: text = uuid" and its
// mirror "uuid = text" to get right, and the reason is that 0099 declared
// `tasting_samples` with **three different types in one table**:
//
//   | column                 | SQLite | PostgreSQL |
//   |------------------------|--------|------------|
//   | `id`                   | TEXT   | **TEXT**   |
//   | `work_id`              | TEXT   | **UUID**   |
//   | `account_id`           | TEXT   | **TEXT**   |
//   | `tasting_responses.id` | TEXT   | TEXT       |
//   | `tasting_responses.sample_id` | TEXT | TEXT  |
//   | `arena_weights.account_id`    | TEXT | **UUID** |
//
// So a cast is needed **per column, not per table**: `work_id` and
// `arena_weights.account_id` take `::uuid`, and nothing else in this module may.
// `sample_id` and `account_id` on the responses table take a bare `$n` — adding
// `::uuid` there is the error this comment exists to stop being re-introduced.
// SQLite is dynamically typed, so every one of these mistakes is invisible on the
// engine half the test suite runs by default, which is why the second-dialect
// gate is not optional for this module.

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

/// The full-scale weight, on the scale §47.2's ranker actually stores.
///
/// **This was `10_000.0`, and it was wrong.** The comment claimed "10_000bp is the
/// arena's own scale", but `weights_from_elos` (`crates/domain/src/
/// taste_vector.rs:743`) normalises every reader's weights to **sum 1.0**, so a
/// dimension the reader has rated strongly sits at ~1.0, not ~10 000. Dividing a
/// 1.0 by 10 000 put every calibrated reader's confidence at 0.0001 and every
/// work's uncertainty at 0.9999 — the floor of the range, for every work, always.
///
/// The consequence was not a slightly-off score. §49.5's queue orders by
/// uncertainty, so a range that is constant across the whole library is a queue
/// that falls through to the `work_id` tie-break: the sampling queue became
/// "the lexicographically first N works", for every reader, forever. A verified
/// probe through the production `weights_from_elos` printed
/// `STORED_WEIGHT=1 UNCERTAINTY=0.9999` for a 1900-Elo dimension after 40
/// matches.
///
/// `1.0` is the honest full scale because it is what the arena's normalisation
/// defines, and the squash is now relative to the reader's own strongest
/// dimension rather than an absolute constant — see `strongest_weight`.
///
/// It doubles as the **magnitude bound** on a weight, and that is a second
/// consequence of the same fact: the arena's normaliser produces values in
/// `[0, 1]` summing to 1, so a weight outside that range cannot have come from a
/// ballot. `apply_response_to_weights` clamps to `±1.0` for the same reason the
/// selector squashes against 1.0 — an unbounded step would let a reader's taste
/// profile grow without limit, and nothing downstream would notice.
pub const WEIGHT_FULL_SCALE: f64 = 1.0;

/// The reader's largest weight by magnitude, or 0.0 when they have none.
///
/// Relative, so the scale question cannot recur: a reader whose weights normalise
/// to 0.4 (because they have weighed eight dimensions) and one whose normalise to
/// 0.9 (two dimensions) are both read against their own peak. Comparing every
/// weight to an absolute constant is what made the first version silently
/// constant across all of them.
fn strongest_weight(weights: &crate::ranking::TagWeights) -> f64 {
    weights
        .weights
        .iter()
        .map(|(_, weight)| weight.abs())
        .fold(0.0, f64::max)
}

/// How confident the selector is about a work, given the reader's weights.
///
/// Deliberately simple and deliberately explicable: §49.5's queue exists to feed
/// a human's attention into a taste profile, and a score nobody can explain is a
/// score nobody can debug when it selects badly.
///
/// The value is `1 - |agreement|`, where agreement is how strongly the reader's
/// weight for the work's dimensions points one way or the other, scaled against
/// the reader's own strongest weight. A work whose tags the reader has never
/// weighed lands near 0.5 — maximum uncertainty, which is where a cold-start
/// reader's whole library should sit.
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
    // Relative to this reader's own peak, so the squash cannot be defeated by a
    // scale change upstream, and clamped so an equal-peak work lands at exactly
    // 1.0 confidence rather than above it.
    let peak = strongest_weight(weights);
    if peak <= 0.0 {
        return 0.5;
    }
    let confidence = (mean / peak).min(WEIGHT_FULL_SCALE);
    // Floored, not clamped to 0. An enormous weight drives confidence to exactly
    // 1.0, so `1.0 - confidence` is exactly 0.0, and a work at uncertainty 0.0
    // sorts last forever: the reader would never be offered it again, however much
    // they came to like it. `UNCERTAINTY_FLOOR` is the bottom of the range, so the
    // queue can always reach its least-certain-but-one work.
    (1.0 - confidence).clamp(UNCERTAINTY_FLOOR, 1.0)
}

/// One reader's answer to one sample, as §49.5 receives it.
///
/// A struct rather than eight positional arguments. `record_response` took them
/// one by one and tripped clippy's `too_many_arguments` (8/7) — a warning that
/// was **already failing the build at HEAD**, because this crate's
/// `.cargo/config.toml` sets `warnings = "deny"`. The two trailing `&str`s are
/// both session-scoped and the two floats are both derived, so grouping them
/// fixes the gate and makes the call site read as the sentence it is rather than
/// as a list of seven strings in an order only the compiler checks.
#[derive(Debug, Clone)]
pub struct TastingResponse {
    pub sample_id: String,
    pub account_id: String,
    pub verdict: TastingVerdict,
    /// Required by the type, not merely by a check: §49.5 makes the reason the
    /// labelled signal, so it is unrepresentable to record a rating without one.
    pub reason: TastingReason,
    /// The optional "not for me: ___" text. Never the signal itself.
    pub free_text: Option<String>,
    /// The selector's own confidence at draw time, recorded rather than
    /// re-derived: it is the only way an evaluation can ask whether the queue
    /// actually chose uncertain items.
    pub uncertainty_at_draw: f64,
    pub session_id: String,
}

/// Record a response. The reason is required by the type, not merely by a check.
pub async fn record_response(db: &Database, response: &TastingResponse) -> Result<String> {
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
                .bind(&response.sample_id)
                .bind(&response.account_id)
                .bind(response.verdict.as_str())
                .bind(response.reason.as_str())
                .bind(response.free_text.as_deref())
                .bind(response.uncertainty_at_draw)
                .bind(&response.session_id)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(&response.sample_id)
                .bind(&response.account_id)
                .bind(response.verdict.as_str())
                .bind(response.reason.as_str())
                .bind(response.free_text.as_deref())
                .bind(response.uncertainty_at_draw)
                .bind(&response.session_id)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    // The sample is marked answered here, in the same function that inserted the
    // response, because migration 0100's partial unique index is expressed as
    // `WHERE answered_at IS NULL` -- the one-open-sample rule *is* this flag.
    // Writing it in the route instead would leave a window where a reader has two
    // open samples of one work and the index cannot object, because the index
    // only sees the flag. A `tasting_responses` row with an unanswered sample is
    // therefore a state this function cannot produce, which is the property
    // `a_response_and_its_samples_answered_flag_never_disagree` asserts.
    let mark = db.sql(
        "UPDATE tasting_samples SET answered_at = ? WHERE id = ? AND answered_at IS NULL",
        "UPDATE tasting_samples SET answered_at = $1 WHERE id = $2 AND answered_at IS NULL",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&mark)
                .bind(&now)
                .bind(&response.sample_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&mark)
                .bind(&now)
                .bind(&response.sample_id)
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

/// A work offered to the reader, with the coordinates §49.5 asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleOffer {
    /// The `tasting_samples` row this offer refers to.
    ///
    /// Carried here rather than looked up by the caller: the row was just
    /// inserted by this module, so reading it back is a round trip to learn
    /// something this function already had, and a second query for "the latest
    /// sample of this work" is a second place that has to agree about which
    /// sample is outstanding.
    pub sample_id: String,
    pub work_id: String,
    pub title: String,
    pub summary: String,
    /// Where the 300-word window starts, so the passage is reproducible from
    /// stored coordinates rather than re-drawn (§49.7).
    pub sample_offset: i64,
    /// The selector's confidence in its own pick: higher means less certain, and
    /// this is the number §49.5's "chosen by uncertainty" is checked against.
    pub uncertainty: f64,
    /// Why this was picked, for the UI and for an operator evaluating the queue.
    pub reason: &'static str,
}

/// How many items §49.5's per-session bound allows by default.
///
/// Small on purpose: "a calibration queue that can consume the whole of a reading
/// session is a chore, and a chore gets abandoned." Five is about ninety seconds
/// of rating.
pub const SESSION_LIMIT: usize = 5;

/// How many recent published works to consider as candidates.
///
/// Deliberately a pool rather than the whole instance. Uncertainty is computed
/// over each candidate's dimensions, so a pool of 200 is a queue that can
/// meaningfully reorder; a pool of 200 000 is a query nobody should run on a
/// reader's first page load. The bound is a pool and not a filter — the ordering
/// still decides what is offered, and a work outside the pool is deferred rather
/// than excluded.
///
/// Public because the route passes it to `candidate_works`, and a constant the
/// caller cannot read is a constant the caller will hard-code a second copy of.
pub const CANDIDATE_POOL: i64 = 200;

/// Build the queue for one reader in one session.
///
/// **This is the function §49.5 is about, and it is the one that was missing.**
/// The selector, the ordering and the session bound were all unit-tested in this
/// file and reachable from nowhere — the "unit test on a function nothing calls"
/// shape `docs/goal.md` names as a definition of not-complete. This is the door,
/// and it lives beside the selector rather than in the route so the ordering stays
/// a property of the domain and is testable without HTTP.
///
/// `candidates` is a parameter rather than a query for the reason
/// `rank_works`'s `dimensions_for` is: eligibility (§30.7, a trust question)
/// belongs to the caller that owns the trust decision, and §49.7 is explicit that
/// nothing in §49 changes who is *eligible*, only what the ranker is told.
///
/// `dimensions_for` is `+ Send + Sync` for the same reason as `rank_works`: held
/// across an `.await`, a bare `&dyn Fn` makes the future non-`Send` and axum
/// rejects the handler with an opaque `Handler` error naming neither the closure
/// nor the reason.
pub async fn build_queue(
    db: &Database,
    account_id: &str,
    session_id: &str,
    limit: usize,
    candidates: Vec<Candidate>,
    dimensions_for: &(dyn Fn(&str) -> Vec<String> + Send + Sync),
) -> Result<Vec<SampleOffer>> {
    let weights = crate::ranking::TagWeights::for_reader(db, account_id).await?;
    let seen = session_count(db, account_id, session_id).await?;
    let already = usize::try_from(seen.max(0)).unwrap_or(usize::MAX);

    // `Candidate.uncertainty` is an *input* to this function's caller-facing shape
    // and an *output* here, so it is overwritten unconditionally: passing a
    // hand-picked uncertainty in and having it survive would let a caller choose
    // the order §49.5 says the selector owns.
    let scored: Vec<Candidate> = candidates
        .into_iter()
        .map(|candidate| {
            let uncertainty = uncertainty_for(&weights, &dimensions_for(&candidate.work_id))
                .clamp(UNCERTAINTY_FLOOR, 1.0);
            Candidate {
                work_id: candidate.work_id,
                uncertainty,
            }
        })
        .collect();

    let chosen = take_for_session(scored, limit, already);
    if chosen.is_empty() {
        return Ok(Vec::new());
    }

    // One query for the batch. N+1 here would be a round trip per card for a
    // surface a reader sees once a session, which is the difference between a
    // calibration queue and a loading spinner.
    let ids: Vec<&str> = chosen.iter().map(|c| c.work_id.as_str()).collect();
    let titles = titles_and_summaries(db, &ids).await?;
    let cold = weights.weights.is_empty();

    let now = crate::identity::now_rfc3339();
    let mut offers = Vec::with_capacity(chosen.len());
    for candidate in chosen {
        // A candidate with no work row is not offerable. Skipped rather than
        // rendered blank: a blank card is a sample the reader is asked to rate and
        // cannot.
        let Some((title, summary)) = titles.get(&candidate.work_id).cloned() else {
            continue;
        };
        let sample_id = uuid::Uuid::new_v4().to_string();
        // The offset is 0 rather than a random window: the passage is served from
        // the work's own text on the client, and a sample whose start varies per
        // draw cannot be reproduced from the stored coordinate — §49.7 asks for
        // reproducibility, and a random offset is the opposite of that.
        if !insert_sample(
            db,
            &sample_id,
            &candidate.work_id,
            account_id,
            0,
            candidate.uncertainty,
            &now,
        )
        .await?
        {
            // The unique index refused a second sample for this (work, reader):
            // already offered, already answered or in flight. Skipped, so a reload
            // mid-queue does not re-offer what is on screen.
            continue;
        }
        offers.push(SampleOffer {
            sample_id,
            work_id: candidate.work_id,
            title,
            summary,
            sample_offset: 0,
            uncertainty: candidate.uncertainty,
            reason: if cold {
                "you have not weighed anything yet, so this is one of the first samples"
            } else {
                "least-certain work for the dimensions you have weighed"
            },
        });
    }
    Ok(offers)
}

/// Insert a `tasting_samples` row, reporting `false` when the unique index says
/// this reader already has a sample for this work.
///
/// `INSERT OR IGNORE` is the SQLite spelling and `ON CONFLICT DO NOTHING` the
/// PostgreSQL one. `DO UPDATE` is deliberately **not** used: an upsert that
/// overwrites would let a second draw in the same session reset a sample the
/// reader is currently rating, and the response would then point at a row whose
/// coordinates no longer match what is on their screen.
async fn insert_sample(
    db: &Database,
    id: &str,
    work_id: &str,
    account_id: &str,
    sample_offset: i64,
    uncertainty_at_draw: f64,
    now: &str,
) -> Result<bool> {
    // The uncertainty goes in with the sample, in the same statement that creates
    // it. §49.5's acceptance criterion is that the queue chose by uncertainty, and
    // a number written later — or supplied by the client — describes the selector
    // after the fact rather than when it acted.
    let sql = match db.backend() {
        Backend::Sqlite => {
            "INSERT OR IGNORE INTO tasting_samples
                (id, work_id, account_id, sample_offset, uncertainty_at_draw, created_at)
             VALUES (?, ?, ?, ?, ?, ?)"
        }
        Backend::Postgres => {
            "INSERT INTO tasting_samples
                (id, work_id, account_id, sample_offset, uncertainty_at_draw, created_at)
             VALUES ($1, $2::uuid, $3, $4, $5, $6)
             ON CONFLICT DO NOTHING"
        }
    };
    // The row count is taken *inside* each arm rather than off a shared value:
    // `SqliteQueryResult` and `PgQueryResult` are different types, so a `let`
    // binding across the two arms would not compile. Per `db-migration-integrity`,
    // each arm also gets its own placeholders -- `?` on SQLite, `$n::uuid` on
    // PostgreSQL -- and `sql_owned` renumbers but never adds casts.
    let rows_affected = match db.backend() {
        Backend::Sqlite => sqlx::query(sql)
            .bind(id)
            .bind(work_id)
            .bind(account_id)
            .bind(sample_offset)
            .bind(uncertainty_at_draw)
            .bind(now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(sql)
            .bind(id)
            .bind(work_id)
            .bind(account_id)
            .bind(sample_offset)
            .bind(uncertainty_at_draw)
            .bind(now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(rows_affected == 1)
}

/// Titles and summaries for a batch of works, keyed by id.
///
/// A missing id is **absent** from the map rather than present-and-empty, so
/// `build_queue` can tell "no such work" from "a work with a blank summary".
async fn titles_and_summaries(
    db: &Database,
    ids: &[&str],
) -> Result<std::collections::HashMap<String, (String, String)>> {
    let mut out = std::collections::HashMap::with_capacity(ids.len());
    // Chunked because SQLite's default `SQLITE_MAX_VARIABLE_NUMBER` is 999 and a
    // queue could in principle be asked for more than that; 50 leaves room for the
    // placeholders to stay well under the limit on either engine.
    for chunk in ids.chunks(50) {
        let sqlite: Vec<String> = (1..=chunk.len()).map(|i| format!("?{i}")).collect();
        let postgres: Vec<String> = (1..=chunk.len()).map(|i| format!("${i}::uuid")).collect();
        let sql = format!(
            "SELECT id, title, summary FROM works \
             WHERE deleted_at IS NULL AND id IN ({})",
            sqlite.join(", ")
        );
        let sql_pg = format!(
            "SELECT id::text, title, summary FROM works \
             WHERE deleted_at IS NULL AND id IN ({})",
            postgres.join(", ")
        );
        let mut query = sqlx::query(&sql);
        let mut query_pg = sqlx::query(&sql_pg);
        for id in chunk {
            query = query.bind(*id);
            query_pg = query_pg.bind(*id);
        }
        let rows: Vec<(String, String, String)> = match db.backend() {
            Backend::Sqlite => query
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
                .into_iter()
                .map(|row| {
                    (
                        row.get::<String, _>(0),
                        row.get::<String, _>(1),
                        row.get::<String, _>(2),
                    )
                })
                .collect(),
            Backend::Postgres => query_pg
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
                .into_iter()
                .map(|row| {
                    (
                        row.get::<String, _>(0),
                        row.get::<String, _>(1),
                        row.get::<String, _>(2),
                    )
                })
                .collect(),
        };
        for (id, title, summary) in rows {
            out.insert(id, (title, summary));
        }
    }
    Ok(out)
}

/// The works a reader may be sampled from: published, public, not deleted, and
/// not already answered by this reader.
///
/// Excludes anything this reader has **ever** rated, not merely this session. A
/// reader who has told you they love a work has nothing left to teach you about
/// it, and §49.5's queue is a *calibration* queue rather than a recommendation
/// surface — offering a known work back would be a recommendation wearing a
/// calibration hat.
pub async fn candidate_works(db: &Database, account_id: &str, limit: i64) -> Result<Vec<String>> {
    let sql = db.sql(
        "SELECT w.id
         FROM works w
         WHERE w.lifecycle = 'published'
           AND w.visibility = 'public'
           AND w.deleted_at IS NULL
           AND w.id NOT IN (
               SELECT ts.work_id
               FROM tasting_responses tr
               JOIN tasting_samples ts ON ts.id = tr.sample_id
               WHERE ts.account_id = ?)
         ORDER BY w.published_at DESC, w.id ASC
         LIMIT ?",
        "SELECT w.id::text
         FROM works w
         WHERE w.lifecycle = 'published'
           AND w.visibility = 'public'
           AND w.deleted_at IS NULL
           AND w.id NOT IN (
               SELECT ts.work_id
               FROM tasting_responses tr
               JOIN tasting_samples ts ON ts.id = tr.sample_id
               WHERE ts.account_id = $1)
         ORDER BY w.published_at DESC, w.id ASC
         LIMIT $2",
    );
    let rows: Vec<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(account_id)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(account_id)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

/// Whether this reader has already answered this sample.
///
/// The `account_id` predicate is load-bearing and not redundant with the caller
/// having already checked ownership: the ownership check answers "is this your
/// sample", and this answers "have you answered it", which is a different
/// question that a double-submit makes reachable.
pub async fn has_responded(db: &Database, sample_id: &str, account_id: &str) -> Result<bool> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM tasting_responses WHERE sample_id = ? AND account_id = ?",
        "SELECT COUNT(*)::bigint FROM tasting_responses WHERE sample_id = $1 AND account_id = $2",
    );
    let n: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(sample_id)
                .bind(account_id)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(sample_id)
                .bind(account_id)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(n > 0)
}

/// One row of the "your samples" query, as the tuple `sqlx` decodes.
///
/// Named rather than inlined because a bare seven-element tuple in a `let`
/// annotation is unreadable at the call site and trips `type_complexity`; the
/// fields are then named in the mapping below, which is where a reader looks.
type ResponseRow = (String, String, String, String, Option<String>, f64, String);

/// One of a reader's recorded answers, for the "your samples" surface.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedResponse {
    pub work_id: String,
    pub title: String,
    pub verdict: String,
    pub reason: String,
    pub free_text: Option<String>,
    pub uncertainty_at_draw: f64,
    pub answered_at: String,
}

/// The reader's own answers, newest first.
///
/// Scoped by `account_id` in the query rather than filtered afterwards, so a
/// reader cannot see another's answers even if a future caller forgets the
/// filter. §49.5's acceptance clause requires a decline to appear in the
/// profile, so nothing here filters on verdict.
pub async fn responses_for(
    db: &Database,
    account_id: &str,
    limit: i64,
) -> Result<Vec<RecordedResponse>> {
    let sql = db.sql(
        "SELECT ts.work_id, COALESCE(w.title, ''), tr.verdict, tr.reason, tr.free_text,
                tr.uncertainty_at_draw, tr.created_at
         FROM tasting_responses tr
         JOIN tasting_samples ts ON ts.id = tr.sample_id
         LEFT JOIN works w ON w.id = ts.work_id
         WHERE tr.account_id = ?
         ORDER BY tr.created_at DESC, tr.id DESC
         LIMIT ?",
        "SELECT ts.work_id::text, COALESCE(w.title, ''), tr.verdict, tr.reason, tr.free_text,
                tr.uncertainty_at_draw, tr.created_at
         FROM tasting_responses tr
         JOIN tasting_samples ts ON ts.id = tr.sample_id
         LEFT JOIN works w ON w.id = ts.work_id
         WHERE tr.account_id = $1
         ORDER BY tr.created_at DESC, tr.id DESC
         LIMIT $2",
    );
    let rows: Vec<ResponseRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(
            |(work_id, title, verdict, reason, free_text, uncertainty_at_draw, answered_at)| {
                RecordedResponse {
                    work_id,
                    title,
                    verdict,
                    reason,
                    free_text,
                    uncertainty_at_draw,
                    answered_at,
                }
            },
        )
        .collect())
}

/// How much one tasting response moves a dimension, as a fraction of the
/// reader's own weight range.
///
/// **Relative, and deliberately small.** §49.5 calls this a calibration queue
/// the reader rates "in seconds", so a response is a weak signal next to a
/// completed read — it is a labelled opinion about one dimension, not an
/// engagement. Multiplying by an absolute constant would put that constant on the
/// same footing as `weights_from_elos`' own normalisation and re-introduce the
/// scale disagreement documented on `WEIGHT_FULL_SCALE`.
///
/// The step is asymmetric on purpose: a **like** nudges a dimension up from
/// wherever it is, and a **dislike** moves it down harder, because a decline is
/// the more informative half of an active-learning round. §49.5 keeps declines
/// precisely so the queue learns from what it got wrong; a queue where likes and
/// declines move the model equally would be discarding that asymmetry the moment
/// it reached the arithmetic.
pub const LIKE_STEP: f64 = 0.05;
pub const DISLIKE_STEP: f64 = 0.10;

/// Move the reader's weights from one response. Returns whether anything moved.
///
/// A `false` is an honest outcome, not a failure: a work with no countable tag has
/// no dimension to weigh, so the response is recorded and trains nothing. The
/// route reports that distinction rather than pretending every response shaped
/// the profile.
///
/// The reason tag decides *which* dimension moves, not merely how much. A reader
/// who says the prose is what put them off has made a statement about prose, and
/// moving every tag on the work equally would be the queue training on something
/// the reader did not say.
pub async fn apply_response_to_weights(
    db: &Database,
    account_id: &str,
    response: &TastingResponse,
) -> Result<bool> {
    // The work's own countable dimensions. §49.2's confirmed-and-capped filter,
    // so a response cannot weigh an unconfirmed tag — the same rule the ranker
    // reads weights under, and using a different one here would mean the queue
    // teaches the model about tags the feed then ignores.
    let work_id = match sample_work_id(db, &response.sample_id).await? {
        Some(id) => id,
        None => return Ok(false),
    };
    let tags = crate::tag_confirmation::gravity_contributing_tags(
        db,
        &work_id,
        crate::tag_confirmation::DEFAULT_CONTRIBUTION_CAP,
    )
    .await
    .map_err(|e| anyhow::anyhow!("countable tags unreadable: {e}"))?;
    if tags.is_empty() {
        return Ok(false);
    }

    // The reason names the dimension it is about. A reason that is not one of the
    // work's tags still moves that tag if the work carries it; a reason naming a
    // dimension this work does not carry moves nothing, because there is nothing
    // here to be evidence about.
    let step = match response.verdict {
        TastingVerdict::Like => LIKE_STEP,
        TastingVerdict::Dislike => -DISLIKE_STEP,
    };
    let target = response.reason.as_str();
    let Some(target_tag) = tags
        .iter()
        .find(|tag| tag.eq_ignore_ascii_case(target))
        .cloned()
    else {
        // The reader's reason is about a dimension this work does not carry. Not
        // an error and not a silent no-op: it means the answer is about the prose
        // and the work is tagged `space`, which is a real and common case, and
        // there is no dimension in common to move.
        return Ok(false);
    };

    let existing = crate::ranking::TagWeights::for_reader(db, account_id)
        .await
        .map_err(|e| anyhow::anyhow!("weights unreadable: {e}"))?;
    let current = existing.weight_of(&target_tag);
    // A weight is a fraction of a 1.0 total (see `WEIGHT_FULL_SCALE`), so the
    // step is applied against that scale and clamped to a non-negative total:
    // `weights_from_elos` divides by the total, and a negative weight would make
    // that total a subtraction.
    // **Signed**, and that is the point. The first draft clamped at 0.0, on the
    // reasoning that `weights_from_elos` normalises to a non-negative total. That
    // is a misreading of this column: `weights_from_elos` is the *arena's*
    // normaliser for a Plackett-Luce round, and the rest of the substrate already
    // treats these weights as signed — `score_candidate` takes `.abs()` and
    // `ranked_propensity` offsets every score by the minimum precisely "because a
    // single negative weight (arena weights are signed) would make the sum zero or
    // negative" (ranking.rs:787).
    //
    // Clamping at zero therefore broke the mechanism in exactly the case it exists
    // for. A cold-start reader has no weight for a dimension, so a **decline** on
    // it computed `0.0 - 0.10` and clamped back to `0.0`: the response was
    // recorded, the route reported success, and the dimension the reader had just
    // said they disliked was left at nothing. §49.5's clause is that a decline is
    // "recorded as a negative with its reason" — and a negative that is stored as
    // zero is not a negative.
    //
    // The step is applied on the signed scale and bounded in both directions. A
    // like can only raise a weight and a dislike can only lower it, so a reader
    // cannot accumulate a runaway magnitude by answering the same card twice —
    // which the one-answer-per-sample index also prevents, at the row level
    // rather than the arithmetic level.
    let next = if current.is_finite() {
        (current + step).clamp(-WEIGHT_FULL_SCALE, WEIGHT_FULL_SCALE)
    } else {
        // A non-finite stored weight is corrupt rather than merely extreme, so it
        // is re-seeded from zero rather than propagated: a NaN here would make
        // every later `weight_of`, every uncertainty score and every ranking
        // order NaN, which sorts unpredictably instead of failing.
        0.0
    };

    // A response that lands on the value already stored has not taught anything,
    // and writing it would increment `matches_played` for a no-op — which is the
    // evidence an evaluation would read as "this dimension is well-evidenced".
    //
    // The comparison is against `current`, not against zero: a dislike that drives
    // an already-negative weight further negative is a real change and is written.
    if (next - current).abs() < f64::EPSILON {
        return Ok(false);
    }

    // `matches_played` accumulates rather than restarts, so a reader's tenth
    // response on one dimension does not read as their first. It is read back
    // rather than assumed zero for exactly that reason.
    let (prior_elo, prior_matches) = existing_match_state(db, account_id, &target_tag).await?;

    crate::taste_vectors::update_arena_weights(
        db,
        account_id,
        &target_tag,
        next,
        // `elo_rating` is carried forward, never derived here. §0.4.2a's Elo
        // belongs to the arena's *pairwise* ballot, and a 300-word sample is not a
        // pairwise comparison; synthesising a plausible Elo from a non-pairwise
        // signal would put a number into the arena's own column that nothing can
        // explain and that `weights_from_elos` would then read as evidence.
        prior_elo,
        prior_matches.saturating_add(1),
    )
    .await
    .map_err(|e| anyhow::anyhow!("weight update failed: {e}"))?;
    Ok(true)
}

/// The uncertainty the selector recorded when it drew this sample.
///
/// `None` for a sample drawn before migration 0100, or for one whose row is gone.
/// `None` is **not** defaulted to the cold-start reading: inventing 0.5 for a row
/// whose real uncertainty nobody recorded would put a plausible number into an
/// evaluation that is supposed to measure the selector, and the measurement would
/// be confidently wrong.
pub async fn uncertainty_of_sample(db: &Database, sample_id: &str) -> Result<Option<f64>> {
    let sql = db.sql(
        "SELECT uncertainty_at_draw FROM tasting_samples WHERE id = ?",
        "SELECT uncertainty_at_draw FROM tasting_samples WHERE id = $1",
    );
    let row: Option<Option<f64>> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(sample_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(sample_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.flatten())
}

/// The account that owns a sample, or `None` when no such sample exists.
///
/// Ownership only — no verdict, no reason, nothing a route could accidentally
/// start trusting. The route needs one fact here ("is this sample yours") and
/// that is the whole question; everything else it needs it already has, from the
/// request it is answering.
pub async fn sample_owner(db: &Database, sample_id: &str) -> Result<Option<String>> {
    let sql = db.sql(
        "SELECT account_id FROM tasting_samples WHERE id = ?",
        "SELECT account_id FROM tasting_samples WHERE id = $1",
    );
    let row: Option<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(sample_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(sample_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row)
}

/// The stored `(elo_rating, matches_played)` for one of a reader's dimensions.
///
/// Defaults to the arena's own baseline `(1500.0, 0)` for a dimension with no row
/// yet, which is the same default `arena_weights` declares — so a first response
/// creates the row already consistent with the arena's arithmetic instead of
/// writing a value the arena would have to correct later.
async fn existing_match_state(
    db: &Database,
    account_id: &str,
    dimension: &str,
) -> Result<(f64, i64)> {
    // The `::uuid` cast here is **correct and load-bearing**, unlike the ones this
    // module had to remove from the `tasting_*` queries: `arena_weights.account_id`
    // is a real UUID column (0066 declares it `NOT NULL REFERENCES accounts(id)`),
    // while `tasting_samples.account_id` and `tasting_responses.account_id` are
    // plain TEXT on both engines (0099). Same shape, opposite type — which is why
    // the fix for the 500s was per-query and not a blanket one.
    let sql = db.sql(
        "SELECT elo_rating, matches_played FROM arena_weights
         WHERE account_id = ? AND dimension_key = ?",
        "SELECT elo_rating, matches_played FROM arena_weights
         WHERE account_id = $1::uuid AND dimension_key = $2",
    );
    let row: Option<(f64, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .bind(dimension)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .bind(dimension)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.unwrap_or((1500.0, 0)))
}

/// The work a sample was drawn from.
async fn sample_work_id(db: &Database, sample_id: &str) -> Result<Option<String>> {
    let sql = db.sql(
        "SELECT work_id FROM tasting_samples WHERE id = ?",
        "SELECT work_id::text FROM tasting_samples WHERE id = $1",
    );
    let row: Option<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(sample_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(sample_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row)
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

    /// A strong weight lowers uncertainty — **on the scale the ranker really
    /// stores**.
    ///
    /// This is the regression test for the defect above, and it goes through
    /// `weights_from_elos` rather than inventing a number, because the bug was
    /// precisely a disagreement between the scale this function assumed and the
    /// scale the production path produces. A hand-written `9000.0` is exactly the
    /// value that hid it: with the old `10_000.0` divisor that test passed, and
    /// the queue was still constant.
    #[test]
    fn a_strong_weight_lowers_uncertainty() {
        let elos = vec![lorehaven_domain::taste_vector::DimensionElo {
            dimension_key: "prose".to_owned(),
            elo_rating: 1900.0,
            matches_played: 40,
        }];
        let pairs = lorehaven_domain::taste_vector::weights_from_elos(&elos);
        let weights = TagWeights { weights: pairs };
        let u = uncertainty_for(&weights, &["prose".to_owned()]);
        assert!(
            u < 0.5,
            "a dimension the reader rated 1900 after 40 matches must not read as \
             ~maximum uncertainty (got {u}); the full scale is {WEIGHT_FULL_SCALE}"
        );
    }

    /// The scale is **relative**, and this is what makes it survive a change
    /// upstream: two readers with differently-shaped weight vectors must produce
    /// the same uncertainty for the same relative standing.
    ///
    /// With the old absolute divisor, a reader weighing two dimensions got the
    /// same 0.9999 as a reader weighing forty, and every work in both libraries
    /// sorted identically. The queue has to respond to the *distribution* of a
    /// reader's weights, not to their count.
    #[test]
    fn uncertainty_is_relative_to_the_readers_own_peak() {
        let narrow = TagWeights {
            weights: vec![("prose".to_owned(), 0.9), ("pacing".to_owned(), 0.1)],
        };
        let broad = TagWeights {
            weights: vec![
                ("prose".to_owned(), 0.35),
                ("pacing".to_owned(), 0.05),
                ("characters".to_owned(), 0.20),
                ("trope_execution".to_owned(), 0.15),
                ("angst".to_owned(), 0.10),
                ("humor".to_owned(), 0.08),
                ("slow_burn".to_owned(), 0.04),
                ("enemies_to_lovers".to_owned(), 0.03),
            ],
        };
        // In both, "prose" is the reader's strongest dimension and "pacing" a
        // weak one, so both must read as far more certain than uncertain.
        assert!(
            uncertainty_for(&narrow, &["prose".to_owned()]) < 0.5,
            "peak dimension of a 2-dimension reader"
        );
        assert!(
            uncertainty_for(&broad, &["prose".to_owned()]) < 0.5,
            "peak dimension of an 8-dimension reader"
        );
        // And the weak dimension stays more uncertain than the peak one in both.
        assert!(
            uncertainty_for(&narrow, &["pacing".to_owned()])
                > uncertainty_for(&narrow, &["prose".to_owned()]),
            "within one reader, a weak dimension is more uncertain than a peak one"
        );
        assert!(
            uncertainty_for(&broad, &["pacing".to_owned()])
                > uncertainty_for(&broad, &["prose".to_owned()]),
            "and the ordering survives a differently-shaped weight vector"
        );
    }

    /// The whole mechanism depends on uncertainty **varying** across the library.
    /// A selector whose every work reads the same value is not a selector: §49.5
    /// orders by uncertainty, so a constant range collapses to the `work_id`
    /// tie-break and the queue becomes "the first N ids in the instance".
    #[test]
    fn uncertainty_varies_across_a_realistic_library() {
        let weights = TagWeights {
            weights: vec![
                ("prose".to_owned(), 0.55),
                ("angst".to_owned(), 0.20),
                ("pacing".to_owned(), 0.10),
                ("humor".to_owned(), 0.08),
                ("slow_burn".to_owned(), 0.04),
                ("characters".to_owned(), 0.03),
            ],
        };
        let rated = uncertainty_for(&weights, &["prose".to_owned()]);
        let partly = uncertainty_for(&weights, &["prose".to_owned(), "humor".to_owned()]);
        let unr = uncertainty_for(&weights, &["vampires".to_owned()]);
        let spread = unr - rated;
        assert!(
            spread > 0.2,
            "an unrated work and the reader's peak dimension must differ measurably, \
             got {rated} vs {unr} (partly-rated {partly})"
        );
        assert!(
            unr > partly && partly > rated,
            "uncertainty must order unr > mixed > rated, got {unr}, {partly}, {rated}"
        );
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
