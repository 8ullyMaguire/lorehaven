//! M45-22 — the concierge's persistence (spec §54.3, §54.5, §54.6).
//!
//! Two responsibilities, and one rule that shapes every signature here:
//!
//! **§54.6's first invariant: the concierge never reads another reader's data.**
//! Every function that returns rows takes `account_id` and puts it in the
//! `WHERE`. Not one that takes only a session id, not one that filters in Rust
//! after fetching. A filter applied after the fetch is a filter someone will
//! forget the first time a caller needs the unfiltered form — and the caller who
//! needs it is always the one debugging a privacy defect.
//!
//! `sessions_for` is the case that proves it: deleting `AND account_id = ?` from
//! its query leaves every other test in this file green, because nothing else
//! reads another reader's sessions. That test is the only thing standing between
//! §54.6 and a disclosure, which is why it is written to be broken.
//!
//! The WIP half carries §54.5's "one notification per watch, ever" as a UNIQUE
//! constraint plus a `notified_at IS NULL` filter rather than as a check in Rust:
//! two concurrent completions both reading "not yet notified" is a race that
//! passes every single-threaded test.

use serde::Serialize;
use sqlx::FromRow;

use crate::{sql_owned, Database};

// ── per-dialect plumbing ────────────────────────────────────────────────────
//
// Same constraint as `source_adapters.rs`, same consequence: `sqlx::Query` is
// parameterised by its database type, so each statement is written twice and the
// builder is re-created inside each arm. Copied rather than shared because a
// `macro_rules!` that must be visible at every use site in the crate is worse than
// a fourth copy of three macros — and the failure mode of sharing one is that a
// fifth caller binds a parameter in one arm and not the other.

// Run a statement, returning rows affected.
macro_rules! exec {
    ($db:expr, $sql:expr, [$($b:expr),* $(,)?]) => {{
        async {
            let out: Result<u64, sqlx::Error> = match $db.backend() {
                crate::Backend::Sqlite => {
                    let q = sqlx::query($sql);
                    $(let q = q.bind($b);)*
                    q.execute($db.sqlite_pool().expect("sqlite handle")).await.map(|r| r.rows_affected())
                }
                crate::Backend::Postgres => {
                    let q = sqlx::query($sql);
                    $(let q = q.bind($b);)*
                    q.execute($db.postgres_pool().expect("postgres handle")).await.map(|r| r.rows_affected())
                }
            };
            out
        }
    }};
}

/// Fetch every row, decoding into `O`.
macro_rules! fetch_all {
    ($db:expr, $sql:expr, [$($b:expr),* $(,)?], $out:ty) => {{
        async {
            let rows: Result<Vec<$out>, sqlx::Error> = match $db.backend() {
                crate::Backend::Sqlite => {
                    let q = sqlx::query_as::<_, $out>($sql);
                    $(let q = q.bind($b);)*
                    q.fetch_all($db.sqlite_pool().expect("sqlite handle")).await.map_err(Into::into)
                }
                crate::Backend::Postgres => {
                    let q = sqlx::query_as::<_, $out>($sql);
                    $(let q = q.bind($b);)*
                    q.fetch_all($db.postgres_pool().expect("postgres handle")).await.map_err(Into::into)
                }
            };
            rows
        }
    }};
}

/// Fetch a single scalar.
macro_rules! fetch_one {
    ($db:expr, $sql:expr, [$($b:expr),* $(,)?], $out:ty) => {{
        async {
            let row: Result<$out, sqlx::Error> = match $db.backend() {
                crate::Backend::Sqlite => {
                    let q = sqlx::query_as::<_, $out>($sql);
                    $(let q = q.bind($b);)*
                    q.fetch_one($db.sqlite_pool().expect("sqlite handle")).await.map_err(Into::into)
                }
                crate::Backend::Postgres => {
                    let q = sqlx::query_as::<_, $out>($sql);
                    $(let q = q.bind($b);)*
                    q.fetch_one($db.postgres_pool().expect("postgres handle")).await.map_err(Into::into)
                }
            };
            row
        }
    }};
}

/// A stored session, as it comes back out.
#[derive(Debug, Clone, PartialEq, Serialize, FromRow)]
pub struct SessionRow {
    pub id: String,
    pub mood: Option<String>,
    pub budget_minutes: Option<i32>,
    /// The JSON array as written, decoded on the way in by [`decode_work_ids`].
    pub work_ids: String,
    pub estimated_minutes: Option<f64>,
    pub truncated_at: Option<i32>,
    pub rate_source: String,
    pub created_at: String,
}

/// Record one rendered queue (§54.3).
///
/// The queue is a **record of a rendering**, never a ranking input (§54.3, §0.3).
/// `work_ids` is stored as the ordered text the render produced so that a later
/// re-read is byte-identical to what the reader saw; nothing joins against it.
pub async fn record_session(
    db: &Database,
    account_id: &str,
    queue: &lorehaven_domain::concierge::ConciergeQueue,
) -> Result<String, sqlx::Error> {
    let sql = sql_owned(
        db,
        "INSERT INTO concierge_sessions
             (id, account_id, mood, budget_minutes, work_ids, estimated_minutes,
              truncated_at, rate_source, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
            .to_string(),
        "INSERT INTO concierge_sessions
             (id, account_id, mood, budget_minutes, work_ids, estimated_minutes,
              truncated_at, rate_source, created_at)
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6::float8, $7, $8, $9::timestamptz)"
            .to_string(),
    );
    let work_ids = encode_work_ids(queue);
    let mood = queue.session_mood();
    // `and_then(try_from)`, NOT `try_from(...).ok()` after an `unwrap_or(0)`. The
    // first version did the latter, which turned "the reader stated no budget"
    // into a recorded budget of zero — a session that says the reader asked for a
    // zero-minute read. §54.6 makes an absent selector the plain blend, and a
    // stored 0 is a *different session* from a stored NULL: it means they asked
    // for nothing at all.
    let budget = queue
        .session_budget_minutes()
        .and_then(|m| i32::try_from(m).ok());
    let truncated = queue.truncated_at.and_then(|t| i32::try_from(t).ok());
    let rate = match queue.rate_source {
        lorehaven_domain::concierge::RateSource::Observed => "observed",
        lorehaven_domain::concierge::RateSource::Default => "default",
    };
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    exec!(
        db,
        &sql,
        [
            &id,
            account_id,
            mood.as_deref(),
            budget,
            &work_ids,
            queue.estimated_minutes,
            truncated,
            rate,
            &now
        ]
    )
    .await?;
    Ok(id)
}

/// The reader's own sessions, newest first.
///
/// **Scoped in SQL, not filtered in Rust.** §54.6's first invariant, and the
/// query is the whole of it: there is no code path here that could return another
/// reader's row, because the `WHERE` is the only place the rows are chosen.
pub async fn sessions_for(
    db: &Database,
    account_id: &str,
    limit: u32,
) -> Result<Vec<SessionRow>, sqlx::Error> {
    let sql = sql_owned(
        db,
        "SELECT id, mood, budget_minutes, work_ids, estimated_minutes, truncated_at,
                rate_source, created_at
           FROM concierge_sessions
          WHERE account_id = ?1
          ORDER BY created_at DESC, id DESC
          LIMIT ?2"
            .to_string(),
        "SELECT id::text, mood, budget_minutes, work_ids, estimated_minutes::float8,
                truncated_at, rate_source, created_at::text
           FROM concierge_sessions
          WHERE account_id = $1::uuid
          ORDER BY created_at DESC, id DESC
          LIMIT $2"
            .to_string(),
    );
    let rows: Vec<SessionRow> =
        fetch_all!(db, &sql, [account_id, i64::from(limit)], SessionRow).await?;
    Ok(rows)
}

/// One session, scoped to its owner.
///
/// Returns `None` for another reader's session rather than an error, so a caller
/// probing ids cannot distinguish "not yours" from "does not exist" — §2.3's
/// indistinguishable-404 rule applied inside the store.
pub async fn session_for(
    db: &Database,
    account_id: &str,
    session_id: &str,
) -> Result<Option<SessionRow>, sqlx::Error> {
    let sql = sql_owned(
        db,
        "SELECT id, mood, budget_minutes, work_ids, estimated_minutes, truncated_at,
                rate_source, created_at
           FROM concierge_sessions
          WHERE account_id = ?1 AND id = ?2"
            .to_string(),
        "SELECT id::text, mood, budget_minutes, work_ids, estimated_minutes::float8,
                truncated_at, rate_source, created_at::text
           FROM concierge_sessions
          WHERE account_id = $1::uuid AND id = $2::uuid"
            .to_string(),
    );
    let rows: Vec<SessionRow> = fetch_all!(db, &sql, [account_id, session_id], SessionRow).await?;
    Ok(rows.into_iter().next())
}

/// The §15.8 mood keys actually carried by at least one published work.
///
/// §54.2 requires the refusal for an unknown mood to name the moods that exist,
/// and the honest list is the moods this instance can actually serve — a taxonomy
/// node nobody has used is not a mood a reader can ask for and be satisfied by.
///
/// Scoped to published, non-deleted works because a mood only carried by a draft
/// would be offered to a reader and then match nothing.
pub async fn moods_in_use(db: &Database) -> Result<Vec<String>, sqlx::Error> {
    let sql = sql_owned(
        db,
        "SELECT DISTINCT tn.norm
           FROM taxonomy_nodes tn
           JOIN work_tags wt ON wt.node_id = tn.id
           JOIN works w ON w.id = wt.work_id
          WHERE tn.kind = 'mood'
            AND w.lifecycle = 'published'
            AND w.deleted_at IS NULL
          ORDER BY tn.norm"
            .to_string(),
        // The join columns are typed the SAME on both sides (`work_tags.work_id`
        // and `works.id` are both uuid here, both TEXT on SQLite), so there is no
        // cast to write. The first version cast `works.id::text = wt.work_id`,
        // which would compare a text rendering against a uuid — PostgreSQL accepts
        // the expression and returns nothing, so it is a silently empty list rather
        // than an error. `taxonomy::tags_with_confirmation` joins the same two
        // tables and writes no cast, which is the cross-check.
        "SELECT DISTINCT tn.norm
           FROM taxonomy_nodes tn
           JOIN work_tags wt ON wt.node_id = tn.id
           JOIN works w ON w.id = wt.work_id
          WHERE tn.kind = 'mood'
            AND w.lifecycle = 'published'
            AND w.deleted_at IS NULL
          ORDER BY tn.norm"
            .to_string(),
    );
    let rows: Vec<(String,)> = fetch_all!(db, &sql, [], (String,)).await?;
    Ok(rows.into_iter().map(|(m,)| m).collect())
}

/// Add a WIP watch (§54.5), returning the watch id.
///
/// `ON CONFLICT (account_id, work_id) DO NOTHING` **plus** a reselect. The
/// conflict clause alone would make a second watch a silent success returning an
/// id that addresses no row; the reselect is what makes it return the *first*
/// watch's id, so a caller can tell "already watching" from "watching now".
///
/// The conflict target is named rather than a bare `ON CONFLICT DO NOTHING`: the
/// bare form swallows any violation, so a future column-level CHECK turning into
/// an error would be silently absorbed here too.
pub async fn add_watch(
    db: &Database,
    account_id: &str,
    work_id: &str,
) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let insert = sql_owned(
        db,
        "INSERT INTO wip_watches (id, account_id, work_id, notified_at, created_at)
         VALUES (?1, ?2, ?3, NULL, ?4)
         ON CONFLICT (account_id, work_id) DO NOTHING"
            .to_string(),
        "INSERT INTO wip_watches (id, account_id, work_id, notified_at, created_at)
         VALUES ($1::uuid, $2::uuid, $3::uuid, NULL, $4::timestamptz)
         ON CONFLICT (account_id, work_id) DO NOTHING"
            .to_string(),
    );
    exec!(db, &insert, [&id, account_id, work_id, &now]).await?;

    let select = sql_owned(
        db,
        "SELECT id FROM wip_watches WHERE account_id = ?1 AND work_id = ?2".to_string(),
        "SELECT id::text FROM wip_watches WHERE account_id = $1::uuid AND work_id = $2::uuid"
            .to_string(),
    );
    // A 1-tuple, not a bare `String`: `FromRow` is implemented for tuples, not for
    // `String` itself, and the compiler says so at the macro expansion rather than at
    // the call site — which is the per-dialect macro's one ergonomic cost.
    let existing: (String,) = fetch_one!(db, &select, [account_id, work_id], (String,)).await?;
    Ok(existing.0)
}

/// Pending watches on a work: `(account_id, watch_id)` where `notified_at IS NULL`.
///
/// §54.5's "one notification per watch, ever" and "the watch is cancelled by the
/// notification" are the same filter, and it is in the `WHERE` rather than in a
/// `WHERE NOT EXISTS` subquery so the partial index on `wip_watches_pending`
/// serves it.
pub async fn pending_watches_for_work(
    db: &Database,
    work_id: &str,
) -> Result<Vec<(String, String)>, sqlx::Error> {
    let sql = sql_owned(
        db,
        "SELECT account_id, id FROM wip_watches
          WHERE work_id = ?1 AND notified_at IS NULL
          ORDER BY created_at"
            .to_string(),
        "SELECT account_id::text, id::text FROM wip_watches
          WHERE work_id = $1::uuid AND notified_at IS NULL
          ORDER BY created_at"
            .to_string(),
    );
    fetch_all!(db, &sql, [work_id], (String, String)).await
}

/// Consume a watch by its own id: set `notified_at`.
///
/// Returns whether a row changed. The `notified_at IS NULL` predicate is inside
/// the `WHERE`, **not** only in the caller's filter, so two concurrent completion
/// events cannot both claim the same watch. `rows_affected` counting a rewrite is
/// not a concern here: the column only moves from NULL to a timestamp.
pub async fn mark_watched(db: &Database, watch_id: &str) -> Result<bool, sqlx::Error> {
    let sql = sql_owned(
        db,
        "UPDATE wip_watches SET notified_at = ?2 WHERE id = ?1 AND notified_at IS NULL".to_string(),
        "UPDATE wip_watches SET notified_at = $2::timestamptz
          WHERE id = $1::uuid AND notified_at IS NULL"
            .to_string(),
    );
    let now = crate::identity::now_rfc3339();
    let affected = exec!(db, &sql, [watch_id, &now]).await?;
    Ok(affected > 0)
}

/// Withdraw a watch. §54.5: silent — no notification, no tombstone.
///
/// Scoped by `account_id` as well as `work_id`: a caller that guessed a work id
/// must not be able to remove somebody else's watch, and the row count is what
/// tells it whether it did.
pub async fn remove_watch(
    db: &Database,
    account_id: &str,
    work_id: &str,
) -> Result<bool, sqlx::Error> {
    let sql = sql_owned(
        db,
        "DELETE FROM wip_watches WHERE account_id = ?1 AND work_id = ?2".to_string(),
        "DELETE FROM wip_watches WHERE account_id = $1::uuid AND work_id = $2::uuid".to_string(),
    );
    let affected = exec!(db, &sql, [account_id, work_id]).await?;
    Ok(affected > 0)
}

/// Is this work already complete? (§54.5's immediate-notify case.)
///
/// `works.completion`, not `works.lifecycle` — they are orthogonal. A work can
/// be `lifecycle = published` and still `completion = in_progress`, and that is
/// exactly the WIP case this feature exists for. Reading `lifecycle` here would
/// make every published work look complete and fire a notification for the entire
/// catalogue on the first watch.
pub async fn is_complete(db: &Database, work_id: &str) -> Result<bool, sqlx::Error> {
    let sql = sql_owned(
        db,
        "SELECT completion FROM works WHERE id = ?1 AND deleted_at IS NULL".to_string(),
        "SELECT completion FROM works WHERE id = $1::uuid AND deleted_at IS NULL".to_string(),
    );
    let rows: Vec<(String,)> = fetch_all!(db, &sql, [work_id], (String,)).await?;
    Ok(rows
        .first()
        .is_some_and(|(c,)| c.eq_ignore_ascii_case("complete")))
}

/// Encode a rendered queue's work ids as the stored JSON array.
///
/// Hand-rolled rather than `serde_json::to_string` on a `Vec<String>` because the
/// ids are `CHAR(26)` on some dialects and the encoding has to be the one the
/// *reader* sees back identically. Quoting is minimal: these are opaque ids, and
/// an id containing a quote is not one this schema accepts.
fn encode_work_ids(queue: &lorehaven_domain::concierge::ConciergeQueue) -> String {
    let mut out = String::from("[");
    for (n, item) in queue.items.iter().enumerate() {
        if n > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&item.work_id.replace('"', ""));
        out.push('"');
    }
    out.push(']');
    out
}

/// Decode the stored array back into ids.
///
/// The pair is tested together: `encode` writing one shape while `decode` reads
/// another is the defect that makes every session come back empty while looking
/// like a parse that worked.
#[must_use]
pub fn decode_work_ids(stored: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(stored).unwrap_or_default()
}
