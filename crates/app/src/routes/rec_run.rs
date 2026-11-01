//! M63-01: `GET /api/v1/rec/run` — one recommendation engine, over a filtered
//! catalogue, with its arithmetic shown.
//!
//! The catalogue route answers "what is in this fandom, by this metric". This
//! route answers the rest of the anchor ask — *engine Y* — because the engines
//! cannot live on a public route: every strategy reads the caller's own history
//! (`seen_work_ids`) and the co-occurrence strategies read other readers'
//! shelves. So the filters are the catalogue's, the ordering is the engine's,
//! and the caller must be signed in.
//!
//! # The ordering rule
//!
//! **The engine decides the head; the metric decides the tail.** Ids the engine
//! ranked come first in its order, and candidates it did not rank follow in the
//! catalogue's metric order. Without that rule the three engines would be
//! incomparable — each would return a different *set* — and comparing them is
//! the whole point of making engine a per-call choice.
//!
//! # The engines
//!
//! | `engine=` | what orders the head |
//! |---|---|
//! | `catalogue` | nothing: pure metric order (the same page `/catalogue` returns) |
//! | `legacy` | the discovery feed's own blend (`domain::discovery::blend` over the candidate engines) |
//! | `rrf` | the pluggable registry's RRF blend over every enabled strategy |
//! | *strategy name* | that one strategy, the way a reader's stored preference narrows it |

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use darkhold_db::catalogue::{self, CatalogueQuery, CatalogueRow, CatalogueSort};
use darkhold_db::rec_strategy::RecContext;
use darkhold_domain::AppError;

/// How many filtered rows the engine is allowed to see.
///
/// The engine ranks a *pool*, not a page: ranking only the first twenty would
/// make the engine's opinion depend on the metric's ordering, which is the thing
/// being compared. 500 is the largest pool the catalogue's own `limit` allows and
/// keeps a runaway strategy from ranking the whole 141k catalogue.
const MAX_CANDIDATES: i64 = 500;

/// How many ids of each strategy's own ranking the explanation carries.
const EXPLAIN_HEAD: usize = 5;

/// Mount the engine route.
pub fn router() -> Router<AppState> {
    Router::new().route("/rec/run", get(run))
}

/// The query string.
#[derive(Debug, Deserialize)]
pub struct RecRunParams {
    fandom: Option<String>,
    source: Option<String>,
    q: Option<String>,
    #[serde(default)]
    min_likes: Option<i64>,
    /// Window on the source's clock: when the author last touched it.
    updated: Option<String>,
    /// Window on our clock: when the crawler last rechecked it.
    refreshed: Option<String>,
    /// Window on entry age: when it entered this catalogue.
    added: Option<String>,
    /// The catalogue metric, used to order what the engine did not rank.
    sort: Option<String>,
    /// `catalogue`, `legacy`, `rrf`, or a registered strategy name.
    engine: Option<String>,
    /// Include each strategy's contribution.
    ///
    /// Read as text rather than as a bool because `explain=1` is what a shell
    /// types and `serde` refuses it for a `bool` — the axum extractor answers
    /// `400 Failed to deserialize query string`, which reads as a query-syntax
    /// error rather than as the accepted spellings of a flag.
    #[serde(default)]
    explain: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

/// One catalogue row, with where the engine put it.
#[derive(Debug, Serialize)]
pub struct ScoredRow {
    #[serde(flatten)]
    row: CatalogueRow,
    /// Sum of this id's RRF contributions across strategies. Absent when the
    /// engine did not rank it, which is different from ranking it at zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    rec_score: Option<f64>,
    /// 1-based position in the engine's own ranking, when it ranked it.
    #[serde(skip_serializing_if = "Option::is_none")]
    rec_rank: Option<usize>,
}

/// One strategy's contribution to the blend.
#[derive(Debug, Serialize)]
pub struct StrategyReport {
    name: String,
    /// How many ids the strategy returned in total.
    produced: usize,
    /// How many of those were in the caller's filtered candidate set. A strategy
    /// that produces hundreds and matches none is a finding, and it is invisible
    /// in the blended page.
    matched: usize,
    /// The head of its own ranking, `(work_id, RRF score)`, restricted to the
    /// candidate set.
    top: Vec<ScorePair>,
}

/// One `(id, score)` pair, named for the wire rather than as a tuple.
#[derive(Debug, Serialize)]
pub struct ScorePair {
    id: String,
    score: f64,
}

/// What the route answers.
#[derive(Debug, Serialize)]
pub struct RecRunPage {
    engine: String,
    sort: String,
    /// Rows the filters matched (before any cap).
    candidates: usize,
    /// The pool the engine ranked — `min(candidates, 500)`.
    ranked_pool: usize,
    items: Vec<ScoredRow>,
    /// The quality metric this page was ranked by, when `sort=` named one.
    #[serde(skip_serializing_if = "Option::is_none")]
    metric: Option<String>,
    /// Engaged rows the metric could score, and how many rows the filters
    /// matched. The pair is the gate: `ranked N of M` — with a plain column sort
    /// there is nothing to gate, so both are absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    ranked: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<i64>,
    /// Present only with `explain=1`, and always empty for `engine=catalogue`,
    /// which runs no strategies — an empty array there would read as "every
    /// strategy produced nothing".
    #[serde(skip_serializing_if = "Option::is_none")]
    strategies: Option<Vec<StrategyReport>>,
}

async fn run(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Query(params): Query<RecRunParams>,
) -> ApiResult<Json<RecRunPage>> {
    // `sort=` is now two vocabularies in one parameter: a stored column
    // (`likes`, `views`, …) or a quality metric (`source_pct`, `quality`, …).
    // The metric wins where it applies, because a metric cannot be expressed as
    // an `ORDER BY` — it is a percentile over the filtered population (M63-03).
    let sort_word = params
        .sort
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let requested_metric = sort_word.and_then(darkhold_domain::quality::Metric::parse);
    if let Some(metric) = requested_metric {
        if let darkhold_domain::quality::Support::Blocked(reason) = metric.support() {
            return Err(invalid(format!(
                "sort: \"{}\" is not available yet — {reason}",
                metric.as_str()
            )));
        }
    }
    let metric = requested_metric.filter(|metric| {
        matches!(
            metric.support(),
            darkhold_domain::quality::Support::Ranked | darkhold_domain::quality::Support::Triage
        )
    });
    let sort = match (metric, sort_word) {
        (Some(_), _) => CatalogueSort::Likes,
        (None, Some(raw)) => {
            CatalogueSort::parse(raw).ok_or_else(|| invalid(format!("unknown sort: {raw}")))?
        }
        (None, None) => CatalogueSort::Likes,
    };
    let limit = params.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return Err(invalid("limit must be between 1 and 100"));
    }

    // One parser for the windows, shared with the catalogue route and the CLI
    // (M63-01). A window typed at the terminal must mean exactly what it means
    // in a URL, so the CLI sends the raw text and this is the only decoder.
    let now = time::OffsetDateTime::now_utc();
    let window = |raw: &Option<String>, field: &str| match raw {
        None => Ok(None),
        Some(text) => darkhold_domain::window::parse(text, now)
            .map(Some)
            .map_err(|e| invalid(format!("{field}: {e}"))),
    };

    // The fandom name is resolved against the same list the catalogue route
    // uses, so an unknown one is a 422 here too rather than an empty pool.
    let fandom = match params
        .fandom
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => None,
        Some(raw) => {
            let known = catalogue::fandoms(state.db(), 500).await?;
            match known.iter().find(|f| f.name.eq_ignore_ascii_case(raw)) {
                Some(found) => Some(found.name.clone()),
                None => {
                    return Err(invalid(format!(
                        "fandom: no fandom called \"{raw}\" ({} fandoms known)",
                        known.len()
                    )))
                }
            }
        }
    };

    let query = CatalogueQuery {
        source: params.source.clone().filter(|s| !s.is_empty()),
        min_likes: params.min_likes,
        q: params.q.clone().filter(|s| !s.is_empty()),
        sort: Some(sort),
        limit: Some(MAX_CANDIDATES),
        offset: Some(0),
        updated: window(&params.updated, "updated")?,
        refreshed: window(&params.refreshed, "refreshed")?,
        added: window(&params.added, "added")?,
        fandom: fandom.clone(),
    };
    // A metric sort ranks the whole filtered population, so the candidate pool is
    // the metric's own order; the pool is then cut to `MAX_CANDIDATES` for the
    // engines, which need a bounded set to blend.
    let (candidates, metric_scores, gate) = match metric {
        Some(metric) => {
            let page = darkhold_db::metrics::ranked(
                state.db(),
                &query,
                metric,
                darkhold_db::metrics::CANDIDATE_CAP as usize,
                0,
            )
            .await?;
            let mut scores: std::collections::HashMap<String, f64> =
                std::collections::HashMap::new();
            let mut rows = Vec::new();
            for item in page.items {
                scores.insert(item.row.id.clone(), item.score);
                rows.push(item.row);
            }
            rows.truncate(MAX_CANDIDATES as usize);
            (rows, Some(scores), Some((page.ranked, page.total)))
        }
        None => (catalogue::search(state.db(), &query).await?, None, None),
    };
    let candidate_ids: std::collections::HashSet<String> =
        candidates.iter().map(|row| row.id.clone()).collect();

    let requested = params
        .engine
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| state.config().discovery.rec_mode.as_str().to_owned());

    let account_id = user.account_id.to_string();
    let explain = truthy(params.explain.as_deref());

    // The engine's ranking, as `(work_id, score)` in its own order.
    let (engine, ranking, strategies) = match requested.as_str() {
        "catalogue" => {
            // A metric sort *is* a ranking, so its scores travel as `rec_score`
            // rather than being thrown away and re-derived from the position. A
            // plain column sort ranks nothing — the page is in the catalogue's
            // order — and reporting `rec_rank` for it would claim otherwise.
            let ranking: Vec<(String, f64)> = match metric_scores.as_ref() {
                Some(scores) => candidates
                    .iter()
                    .map(|row| (row.id.clone(), scores.get(&row.id).copied().unwrap_or(0.0)))
                    .collect(),
                None => Vec::new(),
            };
            ("catalogue".to_owned(), ranking, None)
        }
        "legacy" => {
            // The discovery feed's blend, restricted to the candidate set. It is
            // the same computation the feed serves, so a reader comparing
            // `--engine legacy` against the site is comparing like with like.
            let engines = crate::routes::discovery::build_candidate_engines(
                &state,
                Some(&account_id),
                user.pseud_id.as_ref().map(|p| p.as_uuid()),
                MAX_CANDIDATES,
            )
            .await?;
            let blended = darkhold_domain::discovery::blend(&engines);
            let ranking: Vec<(String, f64)> = blended
                .into_iter()
                .filter(|c| candidate_ids.contains(&c.work_id.to_canonical_string()))
                .map(|c| {
                    (
                        c.work_id.to_canonical_string(),
                        // The blend's own score, not a rank-derived one: it is
                        // what the feed used to order them, and the order of this
                        // vector is what `positions` turns into `rec_rank`.
                        c.score as f64,
                    )
                })
                .collect::<Vec<_>>();
            ("legacy".to_owned(), ranking, None)
        }
        other => {
            let registry = match other {
                "rrf" | "pluggable" => crate::rec_engine::build_registry(&state.config().discovery),
                name => {
                    let full = crate::rec_engine::build_registry(&state.config().discovery);
                    match full.only(name) {
                        Some(narrowed) => narrowed,
                        None => {
                            return Err(invalid(format!(
                                "engine: \"{name}\" is not enabled on this instance; \
                                 enabled strategies: {}",
                                full.names().join(", ")
                            )))
                        }
                    }
                }
            };
            let seen = darkhold_db::reading::seen_work_ids(state.db(), &account_id)
                .await
                .unwrap_or_default();
            let report = registry
                .generate_traced(
                    state.db(),
                    RecContext {
                        account_id: account_id.clone(),
                        seen,
                        cap: candidates.len().max(limit as usize),
                    },
                )
                .await
                .map_err(|e| ApiError(AppError::Internal(e)))?;

            // The blend's score per id, so the printed score is the sum the
            // engine actually used rather than a reconstruction of it.
            let mut scores: std::collections::HashMap<String, f64> =
                std::collections::HashMap::new();
            for contribution in &report.per_strategy {
                for (id, score) in &contribution.ranked {
                    if candidate_ids.contains(id) {
                        *scores.entry(id.clone()).or_insert(0.0) += score;
                    }
                }
            }
            let ranking: Vec<(String, f64)> = report
                .blended
                .iter()
                .filter(|id| candidate_ids.contains(*id))
                .map(|id| (id.clone(), scores.get(id).copied().unwrap_or(0.0)))
                .collect();

            let strategies = explain.then(|| {
                report
                    .per_strategy
                    .iter()
                    .map(|c| {
                        let within: Vec<(String, f64)> = c
                            .ranked
                            .iter()
                            .filter(|(id, _)| candidate_ids.contains(id))
                            .cloned()
                            .collect();
                        StrategyReport {
                            name: c.name.clone(),
                            produced: c.produced,
                            matched: within.len(),
                            top: within
                                .into_iter()
                                .take(EXPLAIN_HEAD)
                                .map(|(id, score)| ScorePair { id, score })
                                .collect(),
                        }
                    })
                    .collect()
            });
            (other.to_owned(), ranking, strategies)
        }
    };

    // The ordering rule: the engine's ids first, in its order; everything else in
    // the metric order the catalogue gave us.
    let positions: std::collections::HashMap<&str, (usize, f64)> = ranking
        .iter()
        .enumerate()
        .map(|(rank, (id, score))| (id.as_str(), (rank + 1, *score)))
        .collect();
    let mut ordered: Vec<&CatalogueRow> = candidates
        .iter()
        .filter(|row| positions.contains_key(row.id.as_str()))
        .collect();
    ordered.sort_by_key(|row| positions[row.id.as_str()].0);
    ordered.extend(
        candidates
            .iter()
            .filter(|row| !positions.contains_key(row.id.as_str())),
    );

    let items: Vec<ScoredRow> = ordered
        .into_iter()
        .take(limit as usize)
        .map(|row| {
            let hit = positions.get(row.id.as_str());
            ScoredRow {
                row: row.clone(),
                rec_score: hit.map(|(_, score)| *score),
                rec_rank: hit.map(|(rank, _)| *rank),
            }
        })
        .collect();

    Ok(Json(RecRunPage {
        engine,
        sort: metric.map_or_else(|| sort.as_str().to_owned(), |m| m.as_str().to_owned()),
        metric: metric.map(|m| m.as_str().to_owned()),
        ranked: gate.map(|(ranked, _)| ranked),
        total: gate.map(|(_, total)| total),
        candidates: candidates.len(),
        ranked_pool: ranking.len(),
        items,
        strategies,
    }))
}

/// The accepted spellings of a flag: `1`, `true`, `yes`, `on`, and a bare
/// `?explain`.
fn truthy(raw: Option<&str>) -> bool {
    matches!(
        raw.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("" | "1" | "true" | "yes" | "on")
    )
}

/// A query string the route cannot honour: a `422` that names the field.
fn invalid(message: impl Into<String>) -> ApiError {
    AppError::Validation {
        message: message.into(),
        field_errors: std::collections::BTreeMap::new(),
    }
    .into()
}
