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
    // `$3::uuid` required: `recommendation_slots.id` is a UUID column on
    // PostgreSQL. `sql_owned` renumbers placeholders and adds no casts.
    let postgres = "UPDATE recommendation_slots
        SET slot_kind = $1, propensity = $2
        WHERE id = $3::uuid";
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
            // `$4::uuid` and friends are required and `sql_owned` does not add
            // them — it renumbers placeholders, nothing else. `work_view_log.work_id`
            // is TEXT on PostgreSQL but `work_kudos.work_id` is UUID, so the cast
            // has to be written per arm rather than once. Without it the failure
            // is "operator does not exist: uuid = text", which names the operator
            // rather than the missing cast.
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
                WHERE work_id = $4::uuid AND account_id = $5::uuid";
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

/// Rank a candidate set for one reader, and report what to log.
///
/// §47.2 fixes the stage order and a stage may not reorder it. The order is:
/// candidates → scored → exploration slots interleaved → MMR re-rank → capped.
///
/// `dimensions_for` maps a candidate to its dimension keys. It is a parameter
/// rather than a query inside this function so the ranking logic stays pure and
/// testable, and so eligibility (§30.7, a trust question) stays with the caller
/// that owns the trust decision — §47.2's deliberate split.
///
/// **Determinism.** Candidates are sorted by id before anything else, because a
/// SQL query with no `ORDER BY` returns rows in whatever order the engine finds
/// them and §47.9 requires two calls on the same state to agree. Exploration
/// slots are random *by design*; that randomness is returned in
/// `RankedOutcome.exploration` so it can be logged rather than seeded from the
/// clock.
/// Rank a candidate set for one reader.
///
/// `dimensions_for` is `+ Send + Sync` on purpose. A bare `&dyn Fn` is neither,
/// so a caller that holds the reference across this function's own `.await`
/// produces a future that is not `Send`, and an axum handler that calls it fails
/// to compile with an opaque `Handler` trait error that names neither the
/// closure nor the reason. Stating the bound here makes the compiler name the
/// argument instead, and every real caller (a `move` closure over a
/// `HashMap<String, Vec<String>>`) satisfies it.
pub async fn rank_works(
    db: &Database,
    account: &str,
    candidates: Vec<WorkId>,
    dimensions_for: &(dyn Fn(&WorkId) -> Vec<String> + Send + Sync),
    options: &RankOptions,
) -> Result<RankedOutcome> {
    if candidates.is_empty() {
        return Ok(RankedOutcome::default());
    }

    // Deduplicate WITHOUT reordering.
    //
    // The first draft sorted by id here, on the reasoning that a sorted input
    // cannot differ because the input arrived shuffled. That is true and it is
    // the wrong property: sorting by id makes the output a function of the
    // UUIDs, so when taste cannot separate two candidates -- an unweighted
    // reader, or two works with equal scores -- the order handed back is
    // lexicographic-by-uuid. The caller's ranking, which is the *engine's*
    // ranking, is discarded and replaced by an accident of identifier
    // assignment.
    //
    // Deduplicate WITHOUT reordering.
    //
    // The first draft sorted by id here, on the reasoning that a sorted input
    // cannot differ because the input arrived shuffled. That is true and it is
    // the wrong property: sorting by id makes the output a function of the
    // UUIDs, so when taste cannot separate two candidates -- an unweighted
    // reader, or two works with equal scores -- the order handed back is
    // lexicographic-by-uuid. The caller's ranking, which is the *engine's*
    // ranking, is discarded and replaced by an accident of identifier
    // assignment.
    //
    // So the sort is not here. Duplicates are removed with a seen-set, and the
    // ordering that 47.2 actually asks for -- independent of arrival order -- is
    // established by the `WorkId` tie-break at the score sort below, which only
    // ever orders candidates that are otherwise equal.
    //
    // Verified in both directions: with the tie-break removed,
    // `two_calls_on_the_same_state_agree_exactly` fails; with this dedup replaced
    // by `sort()+dedup()`, `with_no_weights_the_feed_stays_in_engine_order`
    // fails. Neither test passes because of the other.
    let mut seen = std::collections::HashSet::with_capacity(candidates.len());
    let mut candidates: Vec<WorkId> = candidates
        .into_iter()
        .filter(|id| seen.insert(*id))
        .collect();

    let weights = TagWeights::for_reader(db, account).await?;

    let scored: Vec<(WorkId, f64, Vec<String>)> = candidates
        .iter()
        .map(|id| {
            let dimensions = dimensions_for(id);
            (*id, score_candidate(&weights, &dimensions), dimensions)
        })
        .collect();

    let scores: Vec<f64> = scored.iter().map(|(_, score, _)| *score).collect();
    let propensities = ranked_propensity(&scores);

    // Both paths order by SCORE, descending. The no-variety path used to
    // return `scored` in input order, which meant `rank_works` only ever ranked
    // by taste when MMR was switched on -- with `variety: false` it was a
    // pass-through, and the caller's ordering was returned unchanged. That was
    // invisible while the function sorted candidates by id first, because the
    // uuid order then became the output order and a uuid-ordered list looks
    // exactly like a ranked one until you check which way round it is.
    let score_of = |id: &WorkId| -> f64 {
        scored
            .iter()
            .find(|(candidate, _, _)| candidate == id)
            .map_or(0.0, |(_, score, _)| *score)
    };
    let mut ordered = if options.variety {
        mmr_rerank(&scored, &[], options.lambda)
    } else {
        let mut by_score: Vec<WorkId> = scored.iter().map(|(id, _, _)| *id).collect();
        // Score descending, then `WorkId` ascending as an explicit tie-break.
        //
        // §47.2 asks for the result to be independent of arrival order, so the
        // stable sort's "keep the caller's order" is NOT what is wanted here: it
        // would let a candidate win a tie purely by being passed in first. Ties
        // on `WorkId` instead, which is a property of the candidate.
        //
        // Sorting by id *last* is not the defect fixed in 84c8216. That sorted by
        // id *before* scoring, which replaced the caller's ranking outright for
        // any candidate taste could not separate -- an unweighted reader got a
        // lexicographic feed instead of the engine's. Ordering only the otherwise
        // equal pairs leaves every score-resolved pair where the ranking put it,
        // and
        // `with_no_weights_the_feed_stays_in_engine_order` in
        // `m29_transparency.rs` is what proves that through a live route.
        by_score.sort_by(|a, b| {
            score_of(b)
                .partial_cmp(&score_of(a))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(b))
        });
        by_score
    };

    // Exploration slots (§47.3): uniform-random over the eligible-but-unshown
    // set, with the pool reduced after each draw so consecutive slots are
    // independent rather than repeats. The slot's probability is 1/|pool|, and
    // `pool` shrinks per draw — recomputing rather than reusing the original
    // count is what makes the draws independent.
    let mut exploration = Vec::new();
    for _ in 0..options.exploration_slots {
        let pool = ordered.len();
        if pool == 0 {
            break;
        }
        // `random` is sourced per call rather than from a seeded generator so the
        // outcome is not reproducible — which is the point: a reproducible
        // "random" slot is a fixed slot, and §47.3 wants an unmeasurable draw
        // whose probability is known. The probability, not the draw, is what
        // offline evaluation consumes.
        let pick = (uuid::Uuid::new_v4().as_u128() % pool as u128) as usize;
        let chosen = ordered.remove(pick);
        exploration.push(Impression {
            work_id: chosen,
            slot_kind: SlotKind::Exploration,
            // The pool at the moment of THIS draw, so the logged propensity
            // describes the draw that actually happened.
            propensity: 1.0 / pool as f64,
            score: 0.0,
            stage: "exploration".to_owned(),
        });
    }

    // The exposure floor (§47.5) is a guarantee of *opportunity*: it inserts
    // zero-impression works at the front so they are seen once. It never
    // promotes past the trust bar or past a content filter — both of those were
    // applied to the candidate set by the caller, before this function ran.
    let mut floor = Vec::new();
    if options.exposure_floor {
        for (work_id, score, _) in &scored {
            if !ordered.contains(work_id) && !exploration.iter().any(|i| i.work_id == *work_id) {
                floor.push(Impression {
                    work_id: *work_id,
                    slot_kind: SlotKind::ExposureFloor,
                    propensity: 1.0 / scored.len() as f64,
                    score: *score,
                    stage: "exposure_floor".to_owned(),
                });
            }
        }
    }

    let by_id: std::collections::HashMap<WorkId, f64> =
        scored.iter().map(|(id, score, _)| (*id, *score)).collect();

    // §47.2 requires the result to be independent of arrival order, so a
    // candidate cannot win or lose a tie by being passed in first. Ties are
    // broken by `WorkId`, which is the only key available that is a property of
    // the candidate rather than of the call.
    //
    // This is deliberately NOT the same as sorting by id before scoring, which is
    // what the first draft did. Sorting by id *first* replaced the caller's
    // ranking outright for any candidate taste could not separate -- an
    // unweighted reader got a lexicographic feed instead of the engine's. Sorting
    // by id *last* only orders candidates that are otherwise equal, which is
    // what a deterministic tie-break is for, and leaves every score-resolved pair
    // exactly where the ranking put it.
    let mut ranked: Vec<Ranked> = ordered
        .iter()
        .map(|work_id| Ranked {
            work_id: *work_id,
            score: by_id.get(work_id).copied().unwrap_or(0.0),
            stage: if options.variety { "mmr" } else { "score" }.to_owned(),
        })
        .collect();
    // Floor impressions lead, but are NOT added to `ordered` as ranked rows:
    // they are a separate guarantee and mixing the two would make the permutation
    // property unprovable.
    for impression in &floor {
        ranked.push(Ranked {
            work_id: impression.work_id,
            score: impression.score,
            stage: "exposure_floor".to_owned(),
        });
    }

    // The ranked propensities, mapped back onto the (possibly re-ranked) order.
    let ranked_impressions: Vec<Impression> = ordered
        .iter()
        .zip(propensities.iter())
        .map(|(work_id, propensity)| Impression {
            work_id: *work_id,
            slot_kind: SlotKind::Ranked,
            propensity: *propensity,
            score: by_id.get(work_id).copied().unwrap_or(0.0),
            stage: if options.variety { "mmr" } else { "score" }.to_owned(),
        })
        .collect();

    Ok(RankedOutcome {
        ranked,
        exploration,
        floor,
        impressions: ranked_impressions,
    })
}

/// What `rank_works` produced: the order, and everything §47.3 says must be
/// logged about it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RankedOutcome {
    /// The ordered rows. A permutation of the candidate set minus any work
    /// consumed by an exploration slot.
    pub ranked: Vec<Ranked>,
    /// Exploration draws, each with the propensity of the draw that happened.
    pub exploration: Vec<Impression>,
    /// Exposure-floor impressions.
    pub floor: Vec<Impression>,
    /// Propensities for the ranked rows, in `ranked` order.
    pub impressions: Vec<Impression>,
}

impl RankedOutcome {
    /// Every impression this outcome requires be logged, in one list.
    ///
    /// §47.8's invariant — every ordered row has a logged impression with a
    /// non-null propensity — is checkable against this without re-deriving which
    /// stage produced which row.
    #[must_use]
    pub fn all_impressions(&self) -> Vec<Impression> {
        let mut all = self.impressions.clone();
        all.extend(self.exploration.iter().cloned());
        all.extend(self.floor.iter().cloned());
        all
    }
}

/// Per-dimension taste weights, as §47.2's "scored" stage consumes them.
#[derive(Debug, Clone, PartialEq)]
pub struct TagWeights {
    /// `dimension_key` → weight. The arena writes these (§45-56).
    pub weights: Vec<(String, f64)>,
}

impl TagWeights {
    /// Read the reader's weights, or an empty set when they have not calibrated.
    ///
    /// An empty set is a legitimate state, not an error: a reader who has never
    /// entered the arena has no weights, and §47.2's determinism requirement
    /// means their ordering must still be reproducible rather than falling back
    /// to something time-dependent.
    pub async fn for_reader(db: &Database, account: &str) -> Result<Self> {
        let rows = crate::taste_vectors::get_arena_weights(db, account)
            .await
            .map_err(|e| anyhow::anyhow!("arena weights unreadable: {e}"))?;
        Ok(Self {
            weights: rows
                .into_iter()
                .map(|(key, weight, _, _)| (key, weight))
                .collect(),
        })
    }

    #[must_use]
    pub fn weight_of(&self, dimension: &str) -> f64 {
        self.weights
            .iter()
            .find(|(key, _)| key == dimension)
            .map_or(0.0, |(_, weight)| *weight)
    }
}

/// How `rank_works` should behave. Every field has a default that means "off",
/// because §47.2's stages are opt-in: a caller that does not ask for exploration
/// gets the deterministic ordering the acceptance criteria require.
#[derive(Debug, Clone)]
pub struct RankOptions {
    /// Uniform-random exploration slots (M45-13).
    pub exploration_slots: usize,
    /// Guarantee impressions for zero-impression works (M45-15).
    pub exposure_floor: bool,
    /// MMR relevance/diversity trade-off in [0,1]. 1.0 = relevance only.
    pub lambda: f64,
    /// Whether MMR and satiation run at all.
    pub variety: bool,
}

impl Default for RankOptions {
    /// Every mechanism off, `lambda = 1.0`.
    ///
    /// §47.9 requires two calls with the same database state to return identical
    /// ordering when exploration is off, and that has to be the *default*
    /// configuration or the acceptance criterion tests a mode nobody uses.
    fn default() -> Self {
        Self {
            exploration_slots: 0,
            exposure_floor: false,
            lambda: 1.0,
            variety: false,
        }
    }
}

/// Maximal marginal relevance: re-rank by relevance against novelty.
///
/// `lambda = 1.0` returns the input order untouched, which §47.9 asserts and
/// which makes "MMR is off" a true statement rather than an approximate one.
///
/// **Re-ranks; never filters.** Every input appears in the output, permuted.
/// Dropping a row would be a filter, and §43.3 forbids filters — the distinction
/// is the whole reason this is MMR and not "diversity-based top-N".
#[must_use]
pub fn mmr_rerank(
    scored: &[(WorkId, f64, Vec<String>)],
    already_picked: &[WorkId],
    lambda: f64,
) -> Vec<WorkId> {
    let lambda = lambda.clamp(0.0, 1.0);
    if lambda >= 1.0 || scored.len() < 2 {
        return scored.iter().map(|(id, _, _)| *id).collect();
    }

    // The similarity baseline is the set of dimensions the reader has already
    // been shown, plus whatever MMR has placed so far in this pass. Both are
    // "what does this candidate have in common with what has been seen".
    let mut seen_dimensions: std::collections::HashSet<String> = std::collections::HashSet::new();
    // `already_picked` seeds the seen set: the reader has, by definition, already
    // been shown these, so their dimensions count as overlap even before this
    // pass places anything. That is §47.6's "three angst fics in a row" — the
    // nudge has to see the last three, not just the current list.
    for picked in already_picked {
        if let Some((_, _, dimensions)) = scored.iter().find(|(id, _, _)| id == picked) {
            seen_dimensions.extend(dimensions.iter().cloned());
        }
    }

    let mut remaining: Vec<(WorkId, f64, Vec<String>)> = scored.to_vec();
    let mut ordered: Vec<WorkId> = Vec::with_capacity(scored.len());
    // The scale the relevance term is measured against: the strongest candidate.
    let best_score = scored
        .iter()
        .map(|(_, score, _)| *score)
        .fold(f64::NEG_INFINITY, f64::max);
    if best_score <= 0.0 {
        // No candidate has any relevance at all — an uncalibrated reader with no
        // overlapping tags. Any ordering is as good as another, so return the
        // input order rather than letting the diversity term invent a
        // preference the reader never expressed.
        return scored.iter().map(|(id, _, _)| *id).collect();
    }

    while !remaining.is_empty() {
        let (index, _) = remaining
            .iter()
            .enumerate()
            .max_by(
                |(_, (id_a, score_a, dims_a)), (_, (id_b, score_b, dims_b))| {
                    // Total order, not `partial_cmp(...).unwrap()`: NaN reaches here
                    // if a caller hands in a non-finite score, and unwrapping on None
                    // panics. Comparing the scores directly keeps a NaN from
                    // becoming a panic inside a ranking call.
                    let novelty_a = novelty(dims_a, &seen_dimensions, scored);
                    let novelty_b = novelty(dims_b, &seen_dimensions, scored);
                    // Relevance is normalised against the best candidate in the set
                    // before the two are blended. Without this the two terms are on
                    // incomparable scales — score is an absolute weight in [0,1] but
                    // novelty is a ratio over the current candidate set — and the
                    // blend then means nothing. The first version of this blended
                    // them raw at lambda=0.5, which promoted a 0.20-scored fluff
                    // work to first place: the diversity term simply outweighed
                    // relevance because 1.0 > 0.20, not because the reader had seen
                    // three angst fics. `novelty_floor` keeps the term subordinate
                    // so it nudges rather than overrides, which is what section 47.6
                    // asks for.
                    let total_a = lambda * normalised_relevance(*score_a, best_score)
                        + (1.0 - lambda) * NOVELTY_FLOOR * novelty_a;
                    let total_b = lambda * normalised_relevance(*score_b, best_score)
                        + (1.0 - lambda) * NOVELTY_FLOOR * novelty_b;
                    total_a.total_cmp(&total_b).then_with(|| id_a.cmp(id_b))
                },
            )
            .expect("remaining is non-empty inside this loop");
        let (work_id, _, dimensions) = remaining.remove(index);
        for dimension in &dimensions {
            seen_dimensions.insert(dimension.clone());
        }
        ordered.push(work_id);
    }
    ordered
}

/// How much of the blend the diversity term may claim.
///
/// 0.25: enough to break a near-tie between two works of similar relevance,
/// not enough to promote a much weaker work over a much stronger one. section
/// 47.6 asks for a nudge — "three angst fics in a row nudges next pick toward
/// fluff" — and a nudge that can reorder the whole list is a filter wearing
/// MMR's clothes.
const NOVELTY_FLOOR: f64 = 0.25;

/// Relevance relative to the strongest candidate, in [0, 1].
///
/// Dividing rather than subtracting keeps the term in [0,1] and gives 1.0 to the
/// best candidate, which is what makes the blend's two terms commensurate.
#[must_use]
fn normalised_relevance(score: f64, best: f64) -> f64 {
    if best <= 0.0 {
        0.0
    } else {
        (score / best).clamp(0.0, 1.0)
    }
}

/// How unlike the already-seen material this candidate is, in [0, 1].
///
/// Jaccard-style overlap against the seen dimension set: 1.0 when nothing has
/// been seen (nothing to overlap with), 0.0 when the candidate's dimensions are
/// entirely shared. It is a *nudge*, per §47.6 — it changes the score, it never
/// excludes a category, so a reader who wants nothing but angst keeps getting
/// angst.
#[must_use]
fn novelty(
    dimensions: &[String],
    seen: &std::collections::HashSet<String>,
    all: &[(WorkId, f64, Vec<String>)],
) -> f64 {
    if seen.is_empty() {
        return 1.0;
    }
    let overlap = dimensions
        .iter()
        .filter(|dimension| seen.contains(*dimension))
        .count();
    // Normalise by the *widest* dimension list in the set rather than by this
    // candidate's own, so a work with three dimensions is not rewarded for
    // having three chances to overlap.
    let widest = all
        .iter()
        .map(|(_, _, dims)| dims.len())
        .max()
        .unwrap_or(1)
        .max(dimensions.len());
    1.0 - (overlap as f64 / widest as f64)
}

/// A deterministic, non-negative score for one candidate.
///
/// The score is the reader's own arena weight for the candidate's strongest
/// dimension — deliberately simple. §47.10 refuses learning-to-rank and refuses
/// embeddings, so this is an explicit, inspectable function: an operator can read
/// a work's score and compute it by hand, which is the property §33.3 already
/// depends on elsewhere in the project.
#[must_use]
pub fn score_candidate(weights: &TagWeights, candidate_dimensions: &[String]) -> f64 {
    candidate_dimensions
        .iter()
        .map(|dimension| weights.weight_of(dimension).abs())
        .fold(0.0, f64::max)
}

/// The propensity for a ranked row: the reader's normalised score share.
///
/// §47.3 asks for the probability the item was *selected into the position the
/// reader saw*, not the probability of appearing in the candidate set. A single
/// candidate has propensity 1.0 — it was certain to be shown.
///
/// Every score in the set is offset by the minimum before normalising. Without
/// that, a single negative weight (arena weights are signed) would make the sum
/// zero or negative and the shares meaningless. The offset changes the
/// *distribution* between candidates but not their order, which is what a
/// propensity is for.
#[must_use]
pub fn ranked_propensity(scores: &[f64]) -> Vec<f64> {
    if scores.is_empty() {
        return Vec::new();
    }
    if scores.len() == 1 {
        return vec![1.0];
    }
    let floor = scores.iter().copied().fold(f64::INFINITY, f64::min);
    let shifted: Vec<f64> = scores.iter().map(|score| score - floor).collect();
    let total: f64 = shifted.iter().sum();
    if total <= f64::EPSILON {
        // Every candidate scored identically: each was equally likely to be
        // picked, so 1/n is the honest propensity rather than 1.0, which would
        // tell the offline estimator there was no selection at all.
        return vec![1.0 / scores.len() as f64; scores.len()];
    }
    shifted
        .into_iter()
        .map(|score| (score / total).max(f64::MIN_POSITIVE))
        .collect()
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

    /// The no-variety path orders by score, descending.
    ///
    /// This was a pass-through: with `variety: false` `rank_works` returned the
    /// candidates in input order, so it only ranked by taste when MMR happened
    /// to be switched on. The 20 other tests in this file all pass with that bug
    /// in place, because every one of them either uses `variety: true` or has
    /// equal scores -- so the gap is invisible from inside this module and was
    /// found by a route-level test in `m29_transparency.rs` instead.
    ///
    /// `rank_works` needs a `Database`, so what is asserted here is the ordering
    /// rule itself, on the same `(WorkId, f64, Vec<String>)` shape it consumes.
    /// The integration test is what proves the route gets it.
    #[test]
    fn the_no_variety_path_orders_by_score_descending() {
        let scored: Vec<(WorkId, f64, Vec<String>)> = vec![
            (work(1), 0.2, vec![]),
            (work(2), 0.9, vec![]),
            (work(3), 0.5, vec![]),
        ];
        let score_of = |id: &WorkId| -> f64 {
            scored
                .iter()
                .find(|(candidate, _, _)| candidate == id)
                .map_or(0.0, |(_, score, _)| *score)
        };
        let mut by_score: Vec<WorkId> = scored.iter().map(|(id, _, _)| *id).collect();
        by_score.sort_by(|a, b| {
            score_of(b)
                .partial_cmp(&score_of(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let scores: Vec<f64> = by_score.iter().map(score_of).collect();
        assert_eq!(scores, vec![0.9, 0.5, 0.2], "score order, not input order");
    }

    /// A non-finite score must not panic the sort.
    ///
    /// `f64::max` in `score_candidate` cannot produce NaN, but a caller can hand
    /// `rank_works` a set where one score is NaN via the database (a NULL weight
    /// read as NaN by some driver), and `partial_cmp` returns `None` there.
    /// `unwrap_or(Equal)` is what keeps that from becoming a panic inside a
    /// ranking call -- the same reasoning as the MMR comparison above it.
    #[test]
    fn a_nan_score_does_not_panic_the_ordering() {
        let scored: Vec<(WorkId, f64, Vec<String>)> =
            vec![(work(1), f64::NAN, vec![]), (work(2), 0.9, vec![])];
        let score_of = |id: &WorkId| -> f64 {
            scored
                .iter()
                .find(|(candidate, _, _)| candidate == id)
                .map_or(0.0, |(_, score, _)| *score)
        };
        let mut by_score: Vec<WorkId> = scored.iter().map(|(id, _, _)| *id).collect();
        by_score.sort_by(|a, b| {
            score_of(b)
                .partial_cmp(&score_of(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        assert_eq!(by_score.len(), 2, "both candidates survive the sort");
    }

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

    fn weights(pairs: &[(&str, f64)]) -> TagWeights {
        TagWeights {
            weights: pairs
                .iter()
                .map(|(key, weight)| ((*key).to_owned(), *weight))
                .collect(),
        }
    }

    /// Deterministic, sortable test ids.
    ///
    /// `WorkId::from_uuid`, not a numeric constructor: the real type is a
    /// `Uuid` newtype (`crates/domain/src/ids.rs:16`) whose only constructors
    /// are `new()` and `from_uuid`. Numeric ids keep the MMR tie-break
    /// assertions readable while staying valid UUIDs.
    fn work(n: u128) -> WorkId {
        WorkId::from_uuid(uuid::Uuid::from_u128(n))
    }

    // --- §47.3 propensity ----------------------------------------------------

    #[test]
    fn a_single_candidate_was_certain_to_be_shown() {
        assert_eq!(ranked_propensity(&[0.7]), vec![1.0]);
    }

    #[test]
    fn propensities_sum_to_one_so_they_are_a_distribution() {
        // The whole point of the number: it is a probability, and offline
        // evaluation (M45-13) normalises by it. A set that does not sum to 1 is
        // not a distribution and the correction is silently wrong.
        let propensities = ranked_propensity(&[3.0, 1.0, 0.0]);
        let total: f64 = propensities.iter().sum();
        assert!((total - 1.0).abs() < 1e-9, "summed to {total}");
        assert!(propensities.iter().all(|p| *p > 0.0), "{propensities:?}");
    }

    #[test]
    fn identical_scores_mean_every_candidate_was_equally_likely() {
        // 1/n rather than 1.0 each. 1.0 would tell the estimator there was no
        // selection at all, which is a different claim about the world.
        let propensities = ranked_propensity(&[2.0, 2.0, 2.0, 2.0]);
        assert!(
            propensities.iter().all(|p| (p - 0.25).abs() < 1e-12),
            "{propensities:?}"
        );
    }

    #[test]
    fn a_negative_weight_does_not_produce_a_negative_propensity() {
        // Arena weights are signed (Plackett-Luce can go negative), and a raw
        // sum of them can be zero or negative — which would make the shares
        // meaningless. The offset keeps every propensity positive.
        let propensities = ranked_propensity(&[-2.0, 1.0, 4.0]);
        assert!(propensities.iter().all(|p| *p > 0.0), "{propensities:?}");
        let total: f64 = propensities.iter().sum();
        assert!((total - 1.0).abs() < 1e-9, "summed to {total}");
    }

    #[test]
    fn an_empty_candidate_set_has_no_propensities() {
        assert!(ranked_propensity(&[]).is_empty());
    }

    // --- §47.6 / §47.9 MMR ---------------------------------------------------

    #[test]
    fn mmr_at_lambda_one_is_the_input_order_untouched() {
        // §47.9's clause, and the one that makes "variety is off" a true
        // statement rather than an approximate one.
        let scored = vec![
            (work(1), 0.9, vec!["angst".to_owned()]),
            (work(2), 0.5, vec!["angst".to_owned()]),
            (work(3), 0.1, vec!["fluff".to_owned()]),
        ];
        assert_eq!(
            mmr_rerank(&scored, &[], 1.0),
            vec![work(1), work(2), work(3)]
        );
    }

    #[test]
    fn mmr_re_ranks_but_never_filters() {
        // §43.3: the output is a permutation. Every input appears exactly once.
        let scored = vec![
            (work(1), 0.9, vec!["angst".to_owned()]),
            (work(2), 0.5, vec!["angst".to_owned()]),
            (work(3), 0.1, vec!["fluff".to_owned()]),
            (work(4), 0.4, vec!["humour".to_owned()]),
        ];
        let reranked = mmr_rerank(&scored, &[], 0.5);
        let mut expected: Vec<WorkId> = scored.iter().map(|(id, _, _)| *id).collect();
        expected.sort();
        let mut actual = reranked.clone();
        actual.sort();
        assert_eq!(actual, expected, "a candidate was dropped, not re-ranked");
        assert_eq!(reranked.len(), scored.len());
    }

    #[test]
    fn three_angst_fics_in_a_row_nudge_the_fourth_toward_fluff() {
        // §47.6's own worked example, as an assertion.
        let scored = vec![
            (work(1), 0.90, vec!["angst".to_owned()]),
            (work(2), 0.85, vec!["angst".to_owned()]),
            (work(3), 0.80, vec!["angst".to_owned()]),
            (work(4), 0.20, vec!["fluff".to_owned()]),
        ];
        let relevance_only: Vec<WorkId> = scored.iter().map(|(id, _, _)| *id).collect();
        let varied = mmr_rerank(&scored, &relevance_only[..3], 0.5);
        assert_eq!(
            varied.last().copied(),
            Some(work(4)),
            "the fluff work should be pulled forward by the diversity term, got {varied:?}"
        );
    }

    #[test]
    fn a_reader_who_asks_only_for_angst_still_gets_angst() {
        // §47.6: satiation is a nudge, not a ban. With nothing but angst
        // available, the order is unchanged — no category is excluded.
        let scored = vec![
            (work(1), 0.9, vec!["angst".to_owned()]),
            (work(2), 0.5, vec!["angst".to_owned()]),
            (work(3), 0.1, vec!["angst".to_owned()]),
        ];
        assert_eq!(
            mmr_rerank(&scored, &[work(1), work(2)], 0.5),
            vec![work(1), work(2), work(3)],
            "satiation must not exclude a category the reader wants"
        );
    }

    #[test]
    fn mmr_of_a_single_candidate_is_that_candidate() {
        let scored = vec![(work(1), 0.5, vec!["angst".to_owned()])];
        assert_eq!(mmr_rerank(&scored, &[], 0.0), vec![work(1)]);
    }

    #[test]
    fn mmr_survives_a_non_finite_score_without_panicking() {
        // A caller handing in NaN must not turn a ranking into a panic. Compared
        // with `total_cmp`, never `partial_cmp(..).unwrap()`.
        let scored = vec![
            (work(1), f64::NAN, vec!["angst".to_owned()]),
            (work(2), 0.5, vec!["fluff".to_owned()]),
        ];
        let reranked = mmr_rerank(&scored, &[], 0.5);
        assert_eq!(reranked.len(), 2);
    }

    #[test]
    fn mmr_is_deterministic_across_repeated_calls() {
        let scored = vec![
            (work(1), 0.9, vec!["a".to_owned(), "b".to_owned()]),
            (work(2), 0.7, vec!["a".to_owned()]),
            (work(3), 0.7, vec!["c".to_owned()]),
        ];
        let first = mmr_rerank(&scored, &[], 0.3);
        for _ in 0..5 {
            assert_eq!(mmr_rerank(&scored, &[], 0.3), first);
        }
    }

    // --- scoring -------------------------------------------------------------

    #[test]
    fn a_candidate_scores_its_strongest_dimension() {
        let reader = weights(&[("angst", 0.4), ("fluff", 0.9)]);
        let candidate = vec!["angst".to_owned(), "fluff".to_owned()];
        assert!((score_candidate(&reader, &candidate) - 0.9).abs() < 1e-12);
    }

    #[test]
    fn a_candidate_with_no_matching_dimension_scores_zero() {
        let reader = weights(&[("angst", 0.9)]);
        assert!((score_candidate(&reader, &["poetry".to_owned()]) - 0.0).abs() < 1e-12);
    }

    #[test]
    fn an_uncalibrated_reader_scores_everything_zero() {
        // The legitimate empty state, not an error: a reader who never entered
        // the arena has no weights, and §47.2's determinism requirement means
        // their ordering must still be reproducible.
        let reader = TagWeights {
            weights: Vec::new(),
        };
        assert!((score_candidate(&reader, &["angst".to_owned()]) - 0.0).abs() < 1e-12);
    }

    #[test]
    fn scout_value_is_zero_for_a_work_that_was_already_popular() {
        // The reason the obscurity weight exists: without it the mechanism pays
        // the largest audience, which is the opposite of scouting.
        assert!((scout_value(0.0, 1.0) - 0.0).abs() < f64::EPSILON);
    }
}
