//! The public catalogue: search the fanfiction index by source and quality.
//!
//! ```text
//! GET /catalogue?source=sb&sort=quality&q=winter&min_likes=100&limit=20&offset=0
//! ```
//!
//! # Why this route is public and unauthenticated
//!
//! Every row is metadata about a story that lives somewhere else — title, author,
//! summary, word count, and the endorsement count the source itself publishes.
//! No story text is stored here, and no reader's shelf, tag or reading status is
//! reachable through it: `library_items` is per-account in the *writer's* sense,
//! and this route reads the catalogue accounts only. The estate rule that keeps
//! the catalogue metadata-only is what makes an unauthenticated read safe; if it
//! ever stopped being metadata-only, this route would have to stop being public.
//!
//! # The metric vocabulary
//!
//! `sort=` takes either a stored column (`likes`, `aggregate`, `views`, `words`,
//! `updated`) or a quality metric (`quality`, `source_pct`, `source_z`,
//! `engagement_density`, `consensus`, `crosspost`, `words_maturity`,
//! `source_coverage`, `cohort_pct`, `recency`, `unmeasured`) — see
//! [`darkhold_domain::quality`] for the maths of each.
//!
//! The distinction matters: a stored column is an `ORDER BY`, and ordering by
//! `likes` rewards whichever archive is largest. A metric is a percentile
//! computed *within the row's own source* before anything is compared, which is
//! what makes the ranking independent of archive size. A metric page therefore
//! answers with `ranked` of `total` — on the live catalogue, `1247 of 141317`,
//! because only 1,247 rows carry an endorsement count at all — and the rest of
//! the page is metadata triage, labelled `measured: false`.
//!
//! `likes` remains the default so that nothing changes for a caller that did not
//! ask, and `aggregate` still adds the same story's likes from every other
//! source plus the kudos it earned on darkhold.

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use darkhold_db::catalogue::{
    self, CatalogueQuery, CatalogueRow, CatalogueSort, FandomCount, SourceCount,
};
use darkhold_domain::AppError;

/// A query string the route cannot honour: a `422` that names the field.
fn invalid(message: impl Into<String>) -> ApiError {
    AppError::Validation {
        message: message.into(),
        field_errors: std::collections::BTreeMap::new(),
    }
    .into()
}

/// The query string, in the shape axum reads it.
#[derive(Debug, Deserialize)]
pub struct CatalogueParams {
    source: Option<String>,
    #[serde(default)]
    min_likes: Option<i64>,
    q: Option<String>,
    sort: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    offset: Option<i64>,
    /// Window on the source's clock: when the author last touched it.
    updated: Option<String>,
    /// Window on our clock: when the crawler last rechecked it.
    refreshed: Option<String>,
    /// Window on entry age: when it entered this catalogue.
    added: Option<String>,
    /// Restrict to one fandom, e.g. `naruto`. Case-insensitive; an unknown name
    /// is a `422` that names the field and suggests the nearest known fandom.
    fandom: Option<String>,
}

/// One row of a metric-ranked page.
#[derive(Debug, Serialize)]
pub struct MetricItem {
    #[serde(flatten)]
    row: CatalogueRow,
    /// 0..=1 for every ranked metric.
    score: f64,
    /// False when the metric could not measure the row and metadata triage
    /// ordered it. The two blocks are never mixed in a way that hides this.
    measured: bool,
    /// The components of the score, so a ranking can be argued with.
    signals: darkhold_domain::quality::Signals,
    /// 1-based position on this page.
    rank: usize,
    /// The source's last-update date, which [`CatalogueRow`] does not carry.
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
}

/// What a metric-ranked page answers: the page, plus the gate.
///
/// `ranked` of `total` is the line that keeps a thin ranking from reading as a
/// verdict: on the live catalogue it is `1247 of 141317`, because only 1,247 rows
/// carry any endorsement count at all.
#[derive(Debug, Serialize)]
pub struct MetricPage {
    items: Vec<MetricItem>,
    sort: String,
    metric: String,
    /// Engaged rows the metric could score, across the whole filtered set.
    ranked: i64,
    /// Rows the filters matched.
    total: i64,
    limit: i64,
    offset: i64,
    sources: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fandom: Option<String>,
}

/// What the route answers.
#[derive(Debug, Serialize)]
pub struct CataloguePage {
    items: Vec<CatalogueRow>,
    sort: String,
    limit: i64,
    offset: i64,
    /// The sources present in this page, so a UI can label the filter honestly.
    sources: Vec<String>,
    /// The fandom this page answered for, spelled as the catalogue has it.
    ///
    /// Present only when the request carried `fandom=`. Echoing the resolved
    /// spelling back is what lets a caller that typed `naruto` show `Naruto`.
    #[serde(skip_serializing_if = "Option::is_none")]
    fandom: Option<String>,
    /// How many catalogue rows carry any fandom tag at all, and how many rows
    /// there are.
    ///
    /// Together they are the coverage line. `--fandom naruto` returning 265 rows
    /// is a complete answer only if the caller learns that 94.6% of the
    /// catalogue has no fandom metadata — otherwise a short page reads as a bad
    /// ranking instead of a missing tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    fandom_tagged_rows: Option<i64>,
    /// Of those tagged rows, how many publish a like count at all.
    ///
    /// The second half of the coverage line, and the one that explains a page of
    /// zeros: on the live catalogue the fandom tags come from one source whose
    /// rows carry no endorsements, so an ordered page is ordered by nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    fandom_tagged_with_likes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    catalogue_rows: Option<i64>,
}

/// Mount the catalogue's read routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/catalogue", get(search))
        .route("/catalogue/sources", get(sources))
        .route("/catalogue/fandoms", get(fandoms))
        .route("/catalogue/export", get(export_manifest))
        .route("/catalogue/export.sqlite", get(export_sqlite))
}

async fn search(
    State(state): State<AppState>,
    Query(params): Query<CatalogueParams>,
) -> ApiResult<Response> {
    // A quality metric is not a column, so it cannot be an `ORDER BY`: it is a
    // percentile computed over the filtered population, and the store ranks it in
    // memory (M63-03). The vocabulary is tried first so a metric name is never
    // mistaken for an unknown sort.
    let sort_word = params.sort.as_deref().map(str::trim).unwrap_or("");
    if let Some(metric) = darkhold_domain::quality::Metric::parse(sort_word) {
        use darkhold_domain::quality::Support;
        match metric.support() {
            Support::Blocked(reason) => {
                return Err(invalid(format!(
                    "sort: \"{}\" is not available yet — {reason}",
                    metric.as_str()
                )))
            }
            Support::Raw => return Ok(Json(search_raw(state, params).await?).into_response()),
            Support::Ranked | Support::Triage => {
                let limit = params.limit.unwrap_or(20);
                if !(1..=100).contains(&limit) {
                    return Err(invalid("limit must be between 1 and 100"));
                }
                let offset = params.offset.unwrap_or(0).max(0);
                let query = build_query(&state, &params).await?;
                let page = darkhold_db::metrics::ranked(
                    state.db(),
                    &query,
                    metric,
                    limit as usize,
                    offset as usize,
                )
                .await?;
                let mut sources: Vec<String> =
                    page.items.iter().map(|i| i.row.source_key.clone()).collect();
                sources.sort();
                sources.dedup();
                return Ok(Json(MetricPage {
                    items: page
                        .items
                        .into_iter()
                        .map(|item| MetricItem {
                            row: item.row,
                            score: item.score,
                            measured: item.measured,
                            signals: item.signals,
                            rank: item.rank,
                            updated_at: item.updated_at,
                        })
                        .collect(),
                    sort: metric.as_str().to_owned(),
                    metric: metric.as_str().to_owned(),
                    ranked: page.ranked,
                    total: page.total,
                    limit,
                    offset,
                    sources,
                    fandom: query.fandom.clone(),
                })
                .into_response());
            }
        }
    }
    Ok(Json(search_raw(state, params).await?).into_response())
}

/// The catalogue as a plain `ORDER BY` page — every sort the store can do in SQL.
async fn search_raw(
    state: AppState,
    params: CatalogueParams,
) -> ApiResult<CataloguePage> {
    let sort = match params.sort.as_deref() {
        None => CatalogueSort::Likes,
        Some(raw) => CatalogueSort::parse(raw)
            .ok_or_else(|| invalid(format!("unknown sort: {raw}")))?,
    };
    let limit = params.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return Err(invalid("limit must be between 1 and 100"));
    }
    let fandom = resolve_fandom(&state, params.fandom.as_deref()).await?;
    let (tagged, rankable, total) = match fandom.is_some() {
        true => {
            let coverage = catalogue::fandom_coverage(state.db()).await?;
            (
                Some(coverage.tagged),
                Some(coverage.with_likes),
                Some(catalogue::total_rows(state.db()).await?),
            )
        }
        false => (None, None, None),
    };
    let mut query = build_query(&state, &params).await?;
    query.sort = Some(sort);
    query.limit = Some(limit);
    let items = catalogue::search(state.db(), &query).await?;
    let mut sources: Vec<String> = items.iter().map(|row| row.source_key.clone()).collect();
    sources.sort();
    sources.dedup();
    Ok(CataloguePage {
        items,
        sort: sort.as_str().to_owned(),
        limit,
        offset: query.offset.unwrap_or(0),
        sources,
        fandom: query.fandom.clone(),
        fandom_tagged_rows: tagged,
        fandom_tagged_with_likes: rankable,
        catalogue_rows: total,
    })
}

/// Resolve the request into a [`CatalogueQuery`]: the fandom name against the
/// list the catalogue knows, and the three clocks through the one window parser
/// the CLI shares.
async fn build_query(
    state: &AppState,
    params: &CatalogueParams,
) -> ApiResult<CatalogueQuery> {
    let now = time::OffsetDateTime::now_utc();
    let window = |raw: &Option<String>, field: &str| match raw {
        None => Ok(None),
        Some(text) => darkhold_domain::window::parse(text, now)
            .map(Some)
            .map_err(|e| invalid(format!("{field}: {e}"))),
    };
    Ok(CatalogueQuery {
        source: params.source.clone(),
        min_likes: params.min_likes,
        q: params.q.clone(),
        sort: Some(CatalogueSort::Likes),
        limit: params.limit,
        offset: Some(params.offset.unwrap_or(0)),
        updated: window(&params.updated, "updated")?,
        refreshed: window(&params.refreshed, "refreshed")?,
        added: window(&params.added, "added")?,
        fandom: resolve_fandom(state, params.fandom.as_deref()).await?,
    })
}

/// The fandom this request names, spelled as the catalogue spells it.
///
/// An unknown name is a `422` that suggests the nearest known one: falling
/// through to the store would answer 200 with an empty list, which reads as
/// "this fandom has nothing" rather than "you spelled it differently".
async fn resolve_fandom(
    state: &AppState,
    raw: Option<&str>,
) -> ApiResult<Option<String>> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let known = catalogue::fandoms(state.db(), FANDOM_LIST_LIMIT).await?;
    match known.iter().find(|f| f.name.eq_ignore_ascii_case(raw)) {
        Some(found) => Ok(Some(found.name.clone())),
        None => {
            let hint = suggest(raw, &known)
                .map(|name| format!("; did you mean \"{name}\"?"))
                .unwrap_or_else(|| ".".to_owned());
            Err(invalid(format!(
                "fandom: no fandom called \"{raw}\"{hint} ({} fandoms known)",
                known.len()
            )))
        }
    }
}

/// How many fandoms the suggestion list is built from.
///
/// 500 covers the live catalogue's 233 with room for the tag backfill; a name
/// outside the top 500 is still an exact match, it just cannot be *suggested*.
const FANDOM_LIST_LIMIT: i64 = 500;

/// Every fandom the catalogue holds, largest first.
async fn fandoms(State(state): State<AppState>) -> ApiResult<Json<Vec<FandomCount>>> {
    Ok(Json(
        catalogue::fandoms(state.db(), FANDOM_LIST_LIMIT).await?,
    ))
}

/// The nearest known fandom to what someone typed.
///
/// Prefix and substring first, because those are what a truncated name looks
/// like (`narut`), then edit distance, because those are what a typo looks like
/// (`naruot`). Returns `None` rather than a bad guess: a suggestion nobody
/// recognises costs more attention than no suggestion.
fn suggest<'a>(raw: &str, known: &'a [FandomCount]) -> Option<&'a str> {
    let needle = raw.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    if let Some(hit) = known
        .iter()
        .filter(|f| f.name.to_lowercase().starts_with(&needle))
        .min_by_key(|f| f.name.len())
    {
        return Some(hit.name.as_str());
    }
    if let Some(hit) = known
        .iter()
        .filter(|f| f.name.to_lowercase().contains(&needle))
        .min_by_key(|f| f.name.len())
    {
        return Some(hit.name.as_str());
    }
    let budget = (needle.chars().count() / 3).max(2);
    known
        .iter()
        .map(|f| (edit_distance(&needle, &f.name.to_lowercase()), f))
        .filter(|(distance, _)| *distance <= budget)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, f)| f.name.as_str())
}

/// Levenshtein distance, insertion/deletion/substitution, no transposition.
///
/// Written out rather than pulled in: it is twelve lines, it is used only for a
/// suggestion, and a dependency for it would be the largest thing in the crate's
/// tree for the smallest feature in it.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current[j + 1] = (previous[j] + cost)
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

/// The distinct source keys the catalogue holds, with how many rows each has.
async fn sources(State(state): State<AppState>) -> ApiResult<Json<Vec<SourceCount>>> {
    let rows = catalogue::sources(state.db()).await?;
    Ok(Json(rows))
}

// ===== the downloadable snapshot =====
//
// `GET /catalogue/export.sqlite` and its manifest. The file is built once per
// catalogue fingerprint and cached on disk, because it is the same bytes for
// everyone: rebuilding per request would turn a free download into a denial of
// service on a $5 VPS.

/// Where snapshots live, under the configured storage root.
const EXPORT_DIR: &str = "catalogue-export";

/// How many fingerprints to keep. Two is enough for a client that started a
/// download just as the catalogue changed; older files are dead weight.
const EXPORT_KEEP: usize = 2;

/// Serialises builds inside this process, so ten simultaneous first requests
/// produce one snapshot rather than ten.
fn build_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// The manifest: what the file is, how to use it, and what is deliberately not
/// in it.
#[derive(Debug, Serialize)]
pub struct ExportManifest {
    #[serde(flatten)]
    summary: darkhold_db::catalogue_export::ExportSummary,
    /// The route that serves the file itself.
    url: String,
    /// The file name a download gets.
    filename: String,
    /// Schema, table by table, so a client can write SQL without downloading
    /// first.
    tables: Vec<ExportTable>,
    /// The privacy boundary, in the manifest as well as in the file's `meta`
    /// table — a download someone forgot they had should still be able to
    /// explain itself.
    privacy: String,
    /// Queries that answer the questions the route cannot: the point of a local
    /// database is the SQL nobody wrote for you.
    examples: Vec<ExportExample>,
}

/// One table in the snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct ExportTable {
    name: &'static str,
    columns: &'static [&'static str],
    /// One sentence on what a row means.
    means: &'static str,
}

/// One example query.
#[derive(Debug, Clone, Serialize)]
pub struct ExportExample {
    asks: &'static str,
    sql: &'static str,
}

const EXPORT_TABLES: &[ExportTable] = &[
    ExportTable {
        name: "works",
        columns: &[
            "work_key",
            "source_key",
            "source_work_key",
            "title",
            "author_text",
            "summary",
            "language",
            "word_count",
            "status",
            "source_url",
            "source_updated_at",
            "likes",
            "views",
            "on_darkhold",
            "total_likes",
            "source_count",
            "catalogue_updated_at",
        ],
        means: "One row per story per source. `likes` is the source's own endorsement count \
                (likes, kudos, followers) normalised by the crawler; `total_likes` sums the same \
                story across every source it appears on; `on_darkhold` says the catalogue row was \
                materialised into a work on this instance.",
    },
    ExportTable {
        name: "sources",
        columns: &["source_key", "works", "likes", "newest_update"],
        means: "One row per source: how many works it contributes, their likes in total, and the \
                newest catalogue update for that source.",
    },
    ExportTable {
        name: "meta",
        columns: &["key", "value"],
        means: "How the file was built: schema version, row and work counts, the catalogue \
                fingerprint, the build instant, and the privacy statement.",
    },
];

const EXPORT_EXAMPLES: &[ExportExample] = &[
    ExportExample {
        asks: "Naruto fics updated in the last week, best first",
        sql: "SELECT title, author_text, likes, total_likes, catalogue_updated_at \
              FROM works \
              WHERE lower(title) LIKE '%naruto%' OR lower(summary) LIKE '%naruto%' \
              ORDER BY catalogue_updated_at DESC",
    },
    ExportExample {
        asks: "Hidden gems: strong likes on low views, complete works only",
        sql: "SELECT title, source_key, likes, views, ROUND(1.0 * likes / views, 3) AS likes_per_view \
              FROM works \
              WHERE status = 'complete' AND views > 500 \
              ORDER BY likes_per_view DESC LIMIT 50",
    },
    ExportExample {
        asks: "Which fandoms gained the most catalogue entries (by source)",
        sql: "SELECT source_key, COUNT(*) AS works, SUM(likes) AS likes, MAX(catalogue_updated_at) \
              FROM works GROUP BY source_key ORDER BY works DESC",
    },
    ExportExample {
        asks: "Quality per thousand words rather than raw engagement",
        sql: "SELECT title, word_count, total_likes, \
                     ROUND(1000.0 * total_likes / NULLIF(word_count, 0), 2) AS likes_per_1k_words \
              FROM works WHERE word_count > 20000 AND total_likes > 100 \
              ORDER BY likes_per_1k_words DESC LIMIT 100",
    },
];

fn export_privacy() -> String {
    "Story metadata only. No account, pseudonym, email, import record or reading history is in \
     this file, and no column identifies a reader of this instance. Counts are exact rather than \
     suppressed: the k-anonymity floor protects aggregates about people, and there are none here."
        .to_owned()
}

fn snapshot_path(state: &AppState, fingerprint: &str) -> std::path::PathBuf {
    let short = &fingerprint[..16.min(fingerprint.len())];
    state
        .config()
        .storage
        .root
        .join(EXPORT_DIR)
        .join(format!("catalogue-{short}.sqlite"))
}

fn sidecar_path(snapshot: &std::path::Path) -> std::path::PathBuf {
    snapshot.with_extension("json")
}

/// The cached snapshot for the catalogue's current fingerprint, building it if
/// this fingerprint has never been built.
async fn snapshot(
    state: &AppState,
) -> ApiResult<(std::path::PathBuf, darkhold_db::catalogue_export::ExportSummary)> {
    let fingerprint = darkhold_db::catalogue_export::fingerprint(state.db()).await?;
    let path = snapshot_path(state, &fingerprint);
    if let Some(summary) = read_sidecar(&path).await {
        return Ok((path, summary));
    }

    let _guard = build_lock().lock().await;
    // Re-checked inside the lock: the holder before us may have just built it.
    if let Some(summary) = read_sidecar(&path).await {
        return Ok((path, summary));
    }

    let summary = darkhold_db::catalogue_export::build_sqlite(state.db(), &path).await?;
    if let Ok(json) = serde_json::to_vec(&summary) {
        let sidecar = sidecar_path(&path);
        if let Some(parent) = sidecar.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        let _ = tokio::fs::write(&sidecar, json).await;
    }
    prune_snapshots(&path).await;
    Ok((path, summary))
}

async fn read_sidecar(
    path: &std::path::Path,
) -> Option<darkhold_db::catalogue_export::ExportSummary> {
    if tokio::fs::metadata(path).await.is_err() {
        return None;
    }
    let bytes = tokio::fs::read(sidecar_path(path)).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Keep the newest few snapshots, drop the rest.
async fn prune_snapshots(newest: &std::path::Path) {
    let Some(dir) = newest.parent() else { return };
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    let mut database = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("sqlite") && path != newest {
            database.push(path);
        }
    }
    // Oldest first by mtime, so the tail of the list is what survives.
    let mut stamped = Vec::new();
    for path in database {
        let modified = tokio::fs::metadata(&path)
            .await
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        stamped.push((modified, path));
    }
    stamped.sort();
    let keep = EXPORT_KEEP.saturating_sub(1);
    for (_, path) in stamped.into_iter().rev().skip(keep) {
        let sidecar = sidecar_path(&path);
        let _ = tokio::fs::remove_file(&path).await;
        let _ = tokio::fs::remove_file(&sidecar).await;
    }
}

/// `GET /catalogue/export` — what the file is, before you download tens of MB.
async fn export_manifest(State(state): State<AppState>) -> ApiResult<Json<ExportManifest>> {
    let (path, summary) = snapshot(&state).await?;
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "catalogue.sqlite".to_owned());
    Ok(Json(ExportManifest {
        summary,
        url: "/api/v1/catalogue/export.sqlite".to_owned(),
        filename,
        tables: EXPORT_TABLES.to_vec(),
        privacy: export_privacy(),
        examples: EXPORT_EXAMPLES.to_vec(),
    }))
}

/// `GET /catalogue/export.sqlite` — the snapshot itself.
///
/// Served by `ServeFile`, not read into memory: the file is tens of megabytes on
/// a real catalogue, and a couple of simultaneous downloads should not be able to
/// take the instance down. `ServeFile` also gives conditional requests and Range
/// for free, which is what a resumable download needs.
async fn export_sqlite(State(state): State<AppState>) -> ApiResult<axum::response::Response> {
    let (path, summary) = snapshot(&state).await?;
    let request = axum::http::Request::builder()
        .uri("/catalogue/export.sqlite")
        .body(axum::body::Body::empty())
        .map_err(|e| AppError::Internal(anyhow::anyhow!("export request: {e}")))?;
    let served = tower::ServiceExt::oneshot(tower_http::services::ServeFile::new(&path), request)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serve export: {e}")))?;
    let mut response = served.map(axum::body::Body::new);
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/vnd.sqlite3"),
    );
    headers.insert(
        axum::http::header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_str(&format!(
            "attachment; filename=\"{}\"",
            path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "catalogue.sqlite".to_owned())
        ))
        .map_err(|e| AppError::Internal(anyhow::anyhow!("content-disposition: {e}")))?,
    );
    if let Ok(value) = axum::http::HeaderValue::from_str(&summary.fingerprint) {
        headers.insert(axum::http::header::ETAG, value);
    }
    if let Ok(value) = axum::http::HeaderValue::from_str(&summary.rows.to_string()) {
        headers.insert("x-catalogue-rows", value);
    }
    Ok(response)
}
