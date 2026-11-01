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
//! # Why `sort=quality` is `likes`
//!
//! Because that is what the operator means by quality: the community's own
//! endorsement, normalised across sources by the crawler. `sort=aggregate` adds
//! the same story's likes from every other source plus the kudos it earned on
//! darkhold, which is what "the ones from everywhere plus the ones from here"
//! asks for. The response carries all three numbers so the UI never has to
//! pretend they are the same thing.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use darkhold_db::catalogue::{self, CatalogueQuery, CatalogueRow, CatalogueSort, SourceCount};
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
}

/// Mount the catalogue's read routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/catalogue", get(search))
        .route("/catalogue/sources", get(sources))
        .route("/catalogue/export", get(export_manifest))
        .route("/catalogue/export.sqlite", get(export_sqlite))
}

async fn search(
    State(state): State<AppState>,
    Query(params): Query<CatalogueParams>,
) -> ApiResult<Json<CataloguePage>> {
    let sort = match params.sort.as_deref() {
        None => CatalogueSort::Likes,
        Some(raw) => CatalogueSort::parse(raw).ok_or_else(|| invalid(format!("unknown sort: {raw}")))?,
    };
    let limit = params.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return Err(invalid("limit must be between 1 and 100"));
    }
    // One parser for all three clocks, shared with the CLI (M63-01): a window
    // typed at the terminal must mean exactly what it means in a URL.
    let now = time::OffsetDateTime::now_utc();
    let window = |raw: &Option<String>, field: &str| match raw {
        None => Ok(None),
        Some(text) => darkhold_domain::window::parse(text, now)
            .map(Some)
            .map_err(|e| invalid(format!("{field}: {e}"))),
    };
    let query = CatalogueQuery {
        source: params.source.clone(),
        min_likes: params.min_likes,
        q: params.q.clone(),
        sort: Some(sort),
        limit: Some(limit),
        offset: Some(params.offset.unwrap_or(0)),
        updated: window(&params.updated, "updated")?,
        refreshed: window(&params.refreshed, "refreshed")?,
        added: window(&params.added, "added")?,
    };
    let items = catalogue::search(state.db(), &query).await?;
    let mut sources: Vec<String> = items.iter().map(|row| row.source_key.clone()).collect();
    sources.sort();
    sources.dedup();
    Ok(Json(CataloguePage {
        items,
        sort: sort.as_str().to_owned(),
        limit,
        offset: query.offset.unwrap_or(0),
        sources,
    }))
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
