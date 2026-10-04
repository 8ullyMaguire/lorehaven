//! Reader-surface discovery queries: items 14, 27 and 33 of the 100-idea audit.
//!
//! Spec: `docs/spec-reader-surface-t1.md`. Read-only; nothing here writes.
//!
//! # Why this file exists separately
//!
//! Three read-only discovery queries would be perfectly at home in `discovery.rs`. They are
//! here because of one rule that is easy to lose in a file that has forty other queries:
//! **no query in this module may read a private bookmark row.** `bookmarks.is_public`
//! defaults to 0, so most bookmarks are private, and "most bookmarked" computed over all
//! of them publishes in aggregate what readers chose to keep private.
//!
//! `most_bookmarked_this_week` is the test for it: ten private bookmarks on one work and
//! one public bookmark on another must rank the *second* work first. That test was written
//! before the query, specifically so removing the predicate turns it red.
//!
//! # Dialect notes
//!
//! The date window is a **bound parameter**, not inline arithmetic, because the two engines
//! spell "seven days ago" differently (`longevity.rs` uses `NOW() - INTERVAL`,
//! `payout_store.rs` uses `CAST(strftime('%s', …) AS INTEGER)`). One code path, and a test
//! can pass a fixed clock.
//!
//! sqlx does **not** rewrite `?1` into `$1` for PostgreSQL, so the Postgres arms below are
//! written with `$1` explicitly. `db.sql(sqlite, postgres)` picks the arm.

use anyhow::Result;
use serde::Serialize;
use std::collections::HashMap;

use crate::{Backend, Database};

/// Below this weighted-Jaccard score, a work is not "similar" — it is a guess.
///
/// The floor is the honesty rule from spec §5. Two works sharing one generic freeform tag
/// score around 0.14, and showing that as "Similar works" is a lie the reader can check.
pub const FILTER_FLOOR: f64 = 0.15;

/// Fewer tags than this and there is nothing to compare. Emitted rather than silently
/// skipped, so a caller can tell "too untagged" from "nothing similar".
pub const MIN_TAGS: usize = 2;

/// Default cap on "New in your fandoms" (spec §3).
pub const NEW_IN_FANDOMS_LIMIT: i64 = 12;
/// Default cap on the weekly leaderboard (spec §4).
pub const MOST_BOOKMARKED_LIMIT: i64 = 12;
/// Default cap on the similar-works rail (spec §5).
pub const SIMILAR_WORKS_LIMIT: i64 = 5;

/// One work as the reader-surface sections need it.
///
/// `recent_bookmarks` and `similarity` are `Some` only for the section that computes them,
/// so a client cannot mistake a `None` similarity for a score of zero.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SurfaceWork {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub completion: String,
    pub published_at: Option<String>,
    /// Item 4 of the 100-idea audit: total words across the work's CURRENT chapter
    /// revisions. A plain `i64`, not an `Option`, because the aggregate is COALESCEd to 0
    /// in SQL on both engines — "this work has no chapters yet" is 0 words, which is a
    /// true answer, not a missing field. An `Option` here would put `undefined` in front of
    /// the client for every work with no prose, and a card that renders nothing for a
    /// missing count renders nothing for a zero count too.
    pub word_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_bookmarks: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f64>,
}

/// One row of a reader-surface query, in SELECT order.
///
/// A `type` alias, not a struct: `sqlx::query_as` decodes positionally into a tuple, and
/// deriving `FromRow` would pull a trait from the driver for three queries. Clippy asked
/// for this (`type_complexity`) after `word_count` made the leaderboard tuple seven wide,
/// and the complaint is fair — four of those elements are bare `String`s whose order is
/// only recorded here.
///
/// Two variants, because the two queries genuinely select different things and pretending
/// otherwise would mean optional columns a reader has to null-check for no reason.
type SurfaceRow = (String, String, String, String, Option<String>, i64);
/// `SurfaceRow` plus `recent_bookmarks`, for the leaderboard only.
type LeaderboardRow = (String, String, String, String, Option<String>, i64, i64);

impl SurfaceWork {
    fn plain(
        id: String,
        title: String,
        summary: String,
        completion: String,
        published_at: Option<String>,
        word_count: i64,
    ) -> Self {
        Self {
            id,
            title,
            summary,
            completion,
            published_at,
            word_count,
            recent_bookmarks: None,
            similarity: None,
        }
    }
}

// ---------------------------------------------------------------------------------------
// Item 27 — "Most bookmarked this week"
// ---------------------------------------------------------------------------------------

/// The works with the most **public** bookmarkers since `window_start`, newest count first.
///
/// `window_start` is RFC3339 and supplied by the caller, which is what makes this testable
/// with a fixed clock rather than a sleep.
///
/// Ties break on title ascending. Without that, two works with equal counts come back in
/// whatever order the planner chose — the test is flaky and the page reshuffles between
/// renders.
pub async fn most_bookmarked_this_week(
    db: &Database,
    window_start: &str,
    limit: i64,
) -> Result<Vec<SurfaceWork>> {
    // The Postgres arm casts the COLUMN, not the bound parameter:
    // `bookmarks.created_at` is declared TEXT in `migrations/postgres/0009_library.sql`,
    // so casting `$1` alone leaves the comparison between text and timestamptz, which
    // PostgreSQL rejects. String comparison would also work -- the stored format is
    // fixed-width ISO -- but the cast states the intent. (This note lives out here
    // because a SQL literal is not a place Rust can lex English: `not` is a keyword and
    // the comparison operators are lexed as operators even after two slashes.)
    let sql = db.sql(
        "SELECT w.id, w.title, w.summary, w.completion, w.published_at,
                COUNT(DISTINCT b.account_id) AS recent_bookmarks,
                COALESCE((
                  SELECT SUM(cr.word_count)
                    FROM chapters c
                    JOIN chapter_revisions cr ON cr.id = c.current_revision_id
                   WHERE c.work_id = w.id AND c.deleted_at IS NULL
                ), 0) AS word_count
         FROM bookmarks b
         JOIN works w ON w.id = b.subject_id
         WHERE b.subject_type = 'work'
           AND b.is_public = 1
           AND w.lifecycle = 'published'
           AND w.deleted_at IS NULL
           AND b.created_at >= ?1
         GROUP BY w.id, w.title, w.summary, w.completion, w.published_at
         ORDER BY recent_bookmarks DESC, w.title ASC
         LIMIT ?2",
        "SELECT w.id::text, w.title, w.summary, w.completion, w.published_at,
                COUNT(DISTINCT b.account_id) AS recent_bookmarks,
                COALESCE((
                  SELECT CAST(SUM(cr.word_count) AS BIGINT)
                    FROM chapters c
                    JOIN chapter_revisions cr ON cr.id = c.current_revision_id
                   WHERE c.work_id = w.id AND c.deleted_at IS NULL
                ), 0) AS word_count
         FROM bookmarks b
         JOIN works w ON w.id = b.subject_id
         WHERE b.subject_type = 'work'
           AND b.is_public = TRUE
           AND w.lifecycle = 'published'
           AND w.deleted_at IS NULL
           AND b.created_at::timestamptz >= $1::timestamptz
         GROUP BY w.id, w.title, w.summary, w.completion, w.published_at
         ORDER BY recent_bookmarks DESC, w.title ASC
         LIMIT $2",
    );

    // id, title, summary, completion, published_at, recent_bookmarks, word_count
    let rows: Vec<LeaderboardRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(window_start)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(window_start)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows
        .into_iter()
        .map(
            |(id, title, summary, completion, published_at, count, word_count)| {
                let mut w =
                    SurfaceWork::plain(id, title, summary, completion, published_at, word_count);
                w.recent_bookmarks = Some(count);
                w
            },
        )
        .collect())
}

// ---------------------------------------------------------------------------------------
// Item 14 — "New in your fandoms"
// ---------------------------------------------------------------------------------------

/// The newest published works in the fandoms this reader has **publicly** bookmarked.
///
/// Returns an empty `Vec` when the reader has no public bookmarks, and deliberately does
/// not fall back to all recent works: a section that changes subject when it has no data is
/// a section nobody can learn to read (spec §3).
pub async fn new_in_your_fandoms(
    db: &Database,
    account_id: &str,
    limit: i64,
) -> Result<Vec<SurfaceWork>> {
    let node_ids = reader_fandom_nodes(db, account_id).await?;
    if node_ids.is_empty() {
        return Ok(Vec::new());
    }

    // Fetched once and rendered twice.
    let bookmarked = reader_bookmarked_work_ids(db, account_id).await?;

    // Every id is BOUND, never interpolated, and the reason is the same one documented on
    // `similar_works`: `works.id` is uuid on PostgreSQL and TEXT on SQLite, so a
    // hand-built `IN ('a','b')` list compares uuid against text on one engine and works on
    // the other -- a defect only the two-engine run can find. Binding removes the cast, the
    // per-element `::uuid` arm, and the quote-escaping that would otherwise be
    // load-bearing. `work_tags.node_id` is text on BOTH engines, so the node ids need no
    // cast at all.
    let nodes_sqlite = vec!["?"; node_ids.len()].join(",");
    let nodes_pg = (1..=node_ids.len())
        .map(|n| format!("${n}"))
        .collect::<Vec<_>>()
        .join(",");

    // The exclusion list is OPTIONAL, and an empty `IN ()` is a syntax error on both
    // engines rather than a match-nothing, so the clause and the placeholders are built
    // together and the bind order follows the clause order.
    let mut pg_n = node_ids.len();
    let (excl_sqlite, excl_pg) = if bookmarked.is_empty() {
        (String::new(), String::new())
    } else {
        pg_n += bookmarked.len();
        let s = vec!["?"; bookmarked.len()].join(",");
        let p = (node_ids.len() + 1..=pg_n)
            .map(|n| format!("${n}::uuid"))
            .collect::<Vec<_>>()
            .join(",");
        (
            format!("AND w.id NOT IN ({s})"),
            format!("AND w.id NOT IN ({p})"),
        )
    };

    // `db.sql` borrows both arms, so they must outlive the call -- an inline
    // `&format!(...)` leaves a Cow pointing at a freed temporary.
    let sqlite_arm = format!(
        "SELECT DISTINCT w.id, w.title, w.summary, w.completion, w.published_at,
                COALESCE((
                  SELECT SUM(cr.word_count)
                    FROM chapters c
                    JOIN chapter_revisions cr ON cr.id = c.current_revision_id
                   WHERE c.work_id = w.id AND c.deleted_at IS NULL
                ), 0) AS word_count
         FROM works w
         JOIN work_tags wt ON wt.work_id = w.id
         WHERE wt.node_id IN ({nodes_sqlite})
           AND w.lifecycle = 'published'
           AND w.visibility = 'public'
           AND w.deleted_at IS NULL
           {excl_sqlite}
         ORDER BY w.published_at DESC, w.title ASC
         LIMIT {limit}"
    );
    let postgres_arm = format!(
        "SELECT DISTINCT w.id::text, w.title, w.summary, w.completion, w.published_at,
                COALESCE((
                  SELECT CAST(SUM(cr.word_count) AS BIGINT)
                    FROM chapters c
                    JOIN chapter_revisions cr ON cr.id = c.current_revision_id
                   WHERE c.work_id = w.id AND c.deleted_at IS NULL
                ), 0) AS word_count
         FROM works w
         JOIN work_tags wt ON wt.work_id = w.id
         WHERE wt.node_id IN ({nodes_pg})
           AND w.lifecycle = 'published'
           AND w.visibility = 'public'
           AND w.deleted_at IS NULL
           {excl_pg}
         ORDER BY w.published_at DESC, w.title ASC
         LIMIT {limit}"
    );
    let sql = db.sql(&sqlite_arm, &postgres_arm);

    // The query is BUILT INSIDE each arm, not hoisted above the `match`. A single `let q`
    // before the match fixes the pool type to whichever arm the compiler resolves first,
    // and the other arm then fails with `type mismatch ... expected Sqlite, found
    // Postgres` -- a pool-type error that reads like a database-layer problem and is
    // really a lexical one.
    let rows: Vec<SurfaceRow> = match db.backend() {
        Backend::Sqlite => {
            let mut q =
                sqlx::query_as::<_, (String, String, String, String, Option<String>, i64)>(&sql);
            for node in &node_ids {
                q = q.bind(node);
            }
            for id in &bookmarked {
                q = q.bind(id);
            }
            q.fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            let mut q =
                sqlx::query_as::<_, (String, String, String, String, Option<String>, i64)>(&sql);
            for node in &node_ids {
                q = q.bind(node);
            }
            for id in &bookmarked {
                q = match uuid::Uuid::parse_str(id) {
                    Ok(u) => q.bind(u),
                    Err(_) => q.bind(id),
                };
            }
            q.fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows
        .into_iter()
        .map(
            |(id, title, summary, completion, published_at, word_count)| {
                SurfaceWork::plain(id, title, summary, completion, published_at, word_count)
            },
        )
        .collect())
}

/// The fandom node ids this reader has reached through a public bookmark.
async fn reader_fandom_nodes(db: &Database, account_id: &str) -> Result<Vec<String>> {
    let sql = db.sql(
        "SELECT DISTINCT tn.id
         FROM bookmarks b
         JOIN work_tags wt ON wt.work_id = b.subject_id
         JOIN taxonomy_nodes tn ON tn.id = wt.node_id
         WHERE b.account_id = ?1
           AND b.subject_type = 'work'
           AND b.is_public = 1
           AND tn.kind = 'fandom'",
        "SELECT DISTINCT tn.id::text
         FROM bookmarks b
         JOIN work_tags wt ON wt.work_id = b.subject_id
         JOIN taxonomy_nodes tn ON tn.id = wt.node_id
         WHERE b.account_id = $1::uuid
           AND b.subject_type = 'work'
           AND b.is_public = TRUE
           AND tn.kind = 'fandom'",
    );

    let rows: Vec<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(account_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(account_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}

/// Every work this reader has bookmarked, public or private.
///
/// This one **is** allowed to read private rows, because it is scoped to `account_id` and
/// exists to keep a reader's own library out of their "new" list. It returns only ids, for
/// the same reader, and is never used to rank anything other works.
async fn reader_bookmarked_work_ids(db: &Database, account_id: &str) -> Result<Vec<String>> {
    let sql = db.sql(
        "SELECT DISTINCT subject_id FROM bookmarks
         WHERE account_id = ?1 AND subject_type = 'work'",
        "SELECT DISTINCT subject_id::text FROM bookmarks
         WHERE account_id = $1::uuid AND subject_type = 'work'",
    );

    let rows: Vec<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(account_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(account_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}

// ---------------------------------------------------------------------------------------
// Item 33 — "Similar works"
// ---------------------------------------------------------------------------------------

/// Evidence weight for a taxonomy node kind (spec §5).
///
/// Fandom and ship are the strongest requests a reader can make; a generic tag is weak.
/// Resolved here rather than in the caller so the weighting lives in one place and the
/// unit test below can pin it.
pub fn kind_weight(kind: &str) -> i64 {
    match kind {
        "fandom" | "ship" => 3,
        "character" => 2,
        _ => 1,
    }
}

/// Weighted Jaccard over `(node_id, weight)` pairs, in `[0,1]`.
///
/// Plain Jaccard would treat a fandom and a stray freeform tag as equal evidence. This
/// takes `min` over shared and `max` over union, so the result stays in `[0,1]` and remains
/// comparable across works — a score is a fraction, not an unbounded sum.
///
/// Empty on either side scores 0.0 rather than panicking or returning 1.0.
pub fn weighted_jaccard(a: &[(String, i64)], b: &[(String, i64)]) -> f64 {
    let ma: HashMap<&str, i64> = a.iter().map(|(n, w)| (n.as_str(), *w)).collect();
    let mb: HashMap<&str, i64> = b.iter().map(|(n, w)| (n.as_str(), *w)).collect();
    if ma.is_empty() || mb.is_empty() {
        return 0.0;
    }

    let mut shared = 0i64;
    let mut union = 0i64;
    for (node, wa) in &ma {
        match mb.get(node) {
            Some(wb) => {
                shared += (*wa).min(*wb);
                union += (*wa).max(*wb);
            }
            None => union += *wa,
        }
    }
    for (node, wb) in &mb {
        if !ma.contains_key(node) {
            union += wb;
        }
    }

    if union == 0 {
        0.0
    } else {
        shared as f64 / union as f64
    }
}

/// Works most similar to `work_id`, best first.
///
/// Scored in Rust, not SQL. Expressing weighted Jaccard in SQL means a CASE expression per
/// tag kind per engine; the Rust version is unit-testable with no database at all, which is
/// where `weights_a_fandom_above_a_freeform_tag` lives.
///
/// Returns an empty `Vec` when the subject has fewer than `MIN_TAGS`, or when nothing
/// clears `FILTER_FLOOR`. Callers render nothing in either case.
pub async fn similar_works(db: &Database, work_id: &str, limit: i64) -> Result<Vec<SurfaceWork>> {
    let tags = load_tag_sets(db, work_id).await?;
    let subject = match tags.get(work_id) {
        Some(t) if t.len() >= MIN_TAGS => t.clone(),
        // Too untagged to have an opinion about anything.
        _ => return Ok(Vec::new()),
    };

    // Candidate set: works sharing at least one node, which the `(work_id, node_id)` index
    // answers. Scored and truncated here, so this stays a single indexed read plus a sort.
    let mut scored: Vec<(String, f64)> = tags
        .iter()
        .filter(|(id, _)| id.as_str() != work_id)
        .filter_map(|(id, t)| {
            if t.len() < MIN_TAGS {
                return None;
            }
            let score = weighted_jaccard(&subject, t);
            (score > FILTER_FLOOR).then(|| (id.clone(), score))
        })
        .collect();

    // Score desc, then id asc. The id tie-break is for the same reason the leaderboard has
    // one: deterministic order, so the test is not flaky and the page does not reshuffle.
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .expect("scores are finite")
            .then_with(|| a.0.cmp(&b.0))
    });
    scored.truncate(limit.max(0) as usize);

    if scored.is_empty() {
        return Ok(Vec::new());
    }

    let ids: Vec<String> = scored.iter().map(|(id, _)| id.clone()).collect();

    // The ids are BOUND, not interpolated, and the reason is not tidiness.
    //
    // The previous version built `IN ('...')` with `format!` plus single-quote escaping and
    // then had to add `'{}'::uuid` per element for PostgreSQL, because `works.id` is uuid
    // there and TEXT on SQLite -- a bare `id IN ('...')` compares uuid to text and
    // PostgreSQL answers "operator does not exist: text = uuid" (42703). SQLite accepts the
    // uncast form, so the defect could only ever appear in the two-engine run.
    //
    // Binding removes both problems at once: `?` / `$n::uuid` is the engine's own cast, and
    // the escaping is no longer load-bearing. It is the same fix as `work_tag_sets` below,
    // and leaving one of the two interpolated would mean the next reader has to work out
    // which style this function uses.
    let sqlite_arm = format!(
        "SELECT id, title, summary, completion, published_at,
                COALESCE((
                  SELECT SUM(cr.word_count)
                    FROM chapters c
                    JOIN chapter_revisions cr ON cr.id = c.current_revision_id
                   WHERE c.work_id = works.id AND c.deleted_at IS NULL
                ), 0) AS word_count
         FROM works
         WHERE id IN ({}) AND lifecycle = 'published' AND deleted_at IS NULL",
        vec!["?"; ids.len()].join(",")
    );
    let postgres_arm = format!(
        "SELECT id::text, title, summary, completion, published_at,
                COALESCE((
                  SELECT CAST(SUM(cr.word_count) AS BIGINT)
                    FROM chapters c
                    JOIN chapter_revisions cr ON cr.id = c.current_revision_id
                   WHERE c.work_id = works.id AND c.deleted_at IS NULL
                ), 0) AS word_count
         FROM works
         WHERE id IN ({}) AND lifecycle = 'published' AND deleted_at IS NULL",
        (1..=ids.len())
            .map(|n| format!("${n}::uuid"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let sql = db.sql(&sqlite_arm, &postgres_arm);

    let rows: Vec<SurfaceRow> = match db.backend() {
        Backend::Sqlite => {
            let mut q =
                sqlx::query_as::<_, (String, String, String, String, Option<String>, i64)>(&sql);
            for id in &ids {
                q = q.bind(id);
            }
            q.fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            let mut q =
                sqlx::query_as::<_, (String, String, String, String, Option<String>, i64)>(&sql);
            for id in &ids {
                q = match uuid::Uuid::parse_str(id) {
                    Ok(u) => q.bind(u),
                    Err(_) => q.bind(id),
                };
            }
            q.fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    let scores: HashMap<String, f64> = scored.into_iter().collect();
    let mut out: Vec<SurfaceWork> = rows
        .into_iter()
        .filter_map(
            |(id, title, summary, completion, published_at, word_count)| {
                let similarity = scores.get(&id).copied()?;
                let mut w =
                    SurfaceWork::plain(id, title, summary, completion, published_at, word_count);
                w.similarity = Some(similarity);
                Some(w)
            },
        )
        .collect();
    out.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .expect("finite")
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(out)
}

/// `work_id -> (node_id, evidence weight)` for the subject and everything sharing a node.
async fn load_tag_sets(
    db: &Database,
    work_id: &str,
) -> Result<HashMap<String, Vec<(String, i64)>>> {
    // Two statements, deliberately.
    //
    // The one-statement spelling looks equivalent and is not:
    //
    //   SELECT ... WHERE wt.work_id = ?1
    //        OR wt.node_id IN (SELECT node_id FROM work_tags WHERE work_id = ?1)
    //
    // returns only the rows that *match* the predicate -- so a candidate work that shares
    // one tag with the subject comes back with that ONE tag, not with its full set. Scoring
    // then compares the subject's complete tags against a one-tag stub, the union is
    // wildly inflated, every score collapses, and `MIN_TAGS` filters out results that were
    // good matches all along. It fails silently: no error, just an empty rail.
    //
    // Found by a diagnostic that compared the single-statement row count (4) against the
    // raw `work_tags` count (6) for a three-work fixture.
    let subject_nodes_sql = db.sql(
        "SELECT node_id FROM work_tags WHERE work_id = ?1",
        "SELECT node_id::text FROM work_tags WHERE work_id = $1::uuid",
    );
    let subject_nodes: Vec<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&subject_nodes_sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&subject_nodes_sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    let mut out: HashMap<String, Vec<(String, i64)>> = HashMap::new();
    if subject_nodes.is_empty() {
        return Ok(out);
    }
    // Seed the subject's own rows so `similar_works` can score against them.
    {
        let sql = db.sql(
            "SELECT wt.work_id, wt.node_id, tn.kind
             FROM work_tags wt
             JOIN taxonomy_nodes tn ON tn.id = wt.node_id
             WHERE wt.work_id = ?1",
            "SELECT wt.work_id::text, wt.node_id::text, tn.kind
             FROM work_tags wt
             JOIN taxonomy_nodes tn ON tn.id = wt.node_id
             WHERE wt.work_id = $1::uuid",
        );
        let rows: Vec<(String, String, String)> = match db.backend() {
            Backend::Sqlite => {
                sqlx::query_as(&sql)
                    .bind(work_id)
                    .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                    .await?
            }
            Backend::Postgres => {
                sqlx::query_as(&sql)
                    .bind(work_id)
                    .fetch_all(db.postgres_pool().expect("postgres handle"))
                    .await?
            }
        };
        for (wid, node, kind) in rows {
            out.entry(wid).or_default().push((node, kind_weight(&kind)));
        }
    }

    // Candidates are works sharing at least one node. Finding them is one indexed read.
    // `work_tags.node_id` is TEXT on PostgreSQL, so the bound node id must NOT be cast to
    // uuid -- `$1::uuid` fails with "operator does not exist: text = uuid". The work id it
    // returns IS uuid, hence the `::text` on the output side.
    let candidates_sql = db.sql(
        "SELECT DISTINCT work_id FROM work_tags WHERE node_id = ?1",
        "SELECT DISTINCT work_id::text FROM work_tags WHERE node_id = $1",
    );
    let mut candidates: Vec<String> = Vec::new();
    for node in &subject_nodes {
        let ids: Vec<String> = match db.backend() {
            Backend::Sqlite => {
                sqlx::query_scalar(&candidates_sql)
                    .bind(node)
                    .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                    .await?
            }
            Backend::Postgres => {
                sqlx::query_scalar(&candidates_sql)
                    .bind(node)
                    .fetch_all(db.postgres_pool().expect("postgres handle"))
                    .await?
            }
        };
        for id in ids {
            // The subject shares its own nodes, so it is always in this set. Seeding it
            // again here would duplicate its rows: a one-tag subject would read back with
            // two and slip past the MIN_TAGS guard in `similar_works`.
            if id != work_id && !candidates.contains(&id) {
                candidates.push(id);
            }
        }
    }

    // Then each candidate's COMPLETE tag set, in one statement over the whole set.
    //
    // Fetching per shared node instead is the same bug one level down: it still returns only
    // the rows that matched, so a candidate sharing one tag with the subject gets a one-tag
    // stub, its union is inflated, and every score collapses below the floor. Weighted
    // Jaccard's denominator is the union of both sides, so a partial right-hand side makes
    // the number wrong even when the tag it did return was the right one.
    if candidates.is_empty() {
        return Ok(out);
    }
    // The ids are BOUND, not interpolated. An earlier version built the IN-list with
    // `format!("'{}'", id)` plus single-quote escaping -- a hand-rolled SQL literal for
    // values that have a parameter syntax. The candidates come from taxonomy_nodes, so
    // they are not attacker-controlled today, but the escaping was load-bearing anyway,
    // and the day one of them is derived from anything a reader typed, "not today" is
    // the whole security argument.
    let placeholders = vec!["?"; candidates.len()].join(",");
    let full_sqlite = format!(
        "SELECT wt.work_id, wt.node_id, tn.kind
         FROM work_tags wt
         JOIN taxonomy_nodes tn ON tn.id = wt.node_id
         WHERE wt.work_id IN ({placeholders})"
    );
    let pg_placeholders = (1..=candidates.len())
        .map(|n| format!("${n}::uuid"))
        .collect::<Vec<_>>()
        .join(",");
    let full_postgres = format!(
        "SELECT wt.work_id::text, wt.node_id::text, tn.kind
         FROM work_tags wt
         JOIN taxonomy_nodes tn ON tn.id = wt.node_id
         WHERE wt.work_id IN ({pg_placeholders})"
    );
    // Both arms are bound to locals, not inline `&format!(...)`: `db.sql` borrows them, so
    // a Cow pointing at a freed temporary is a borrow error (E0716) that reads like a
    // lifetime puzzle rather than "the value has nowhere to live".
    let full_sql = db.sql(&full_sqlite, &full_postgres);
    let rows: Vec<(String, String, String)> = match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, (String, String, String)>(&full_sql);
            for id in &candidates {
                q = q.bind(id);
            }
            q.fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            let mut q = sqlx::query_as::<_, (String, String, String)>(&full_sql);
            for id in &candidates {
                q = match uuid::Uuid::parse_str(id) {
                    Ok(u) => q.bind(u),
                    Err(_) => q.bind(id),
                };
            }
            q.fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    for (wid, node_id, kind) in rows {
        out.entry(wid)
            .or_default()
            .push((node_id, kind_weight(&kind)));
    }
    Ok(out)
}
