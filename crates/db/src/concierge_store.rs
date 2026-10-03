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

/// Bind a list of work ids as `Uuid` on PostgreSQL and as text on SQLite.
///
/// Exists because the two engines disagree about the TYPE of every id column —
/// `TEXT` on SQLite, `uuid` on PostgreSQL — and sqlx binds one Rust type per
/// parameter. Binding the string on PostgreSQL reaches the server as text and it
/// answers `operator does not exist: uuid = text`, which surfaces as a 500 on one
/// engine only.
///
/// Two ways this failed before it was factored out, and both are worth not
/// repeating:
///
/// - `duration_estimates` bound ids as text and returned 500 on PostgreSQL only,
///   and only when the list was non-empty — an empty `IN ()` compares nothing, so a
///   queue with no items passed on both engines and a queue with items did not.
/// - `filter_by_mond` had the identical line, one function away.
///
/// So: parse, and SKIP an id that will not parse rather than binding it as text.
/// It cannot match a uuid column either way, and skipping keeps one malformed id
/// from failing a reader's whole queue over a work the blend already returned.
///
/// `DB` is the sqlx database parameter rather than a concrete one: naming
/// `Postgres` here would tie the helper to one arm, and the SQLite arm needs the
/// text binds it already has. Only the PostgreSQL call sites use it.
/// Bind a list of work ids as `Uuid` on PostgreSQL and as text on SQLite.
///
/// A macro, not a function, and for a mechanical reason: `sqlx::query::QueryAs`
/// carries its argument type as a third generic parameter, and that type CHANGES
/// with every `bind`. A function cannot name its own return type after mutating it,
/// so any signature either loses the argument type (a compile error) or names it
/// concretely, which pins the helper to one query's row shape. The `exec!` and
/// `fetch_all!` macros below this one avoid the same wall by being macros.
///
/// The behaviour it exists for: every id column is `TEXT` on SQLite and `uuid` on
/// PostgreSQL, and sqlx binds one Rust type per parameter. Binding the string on
/// PostgreSQL reaches the server as text and it answers
/// `operator does not exist: uuid = text` — a 500 on one engine only.
///
/// Two occurrences of exactly this line, and both are worth not repeating:
///
/// - `duration_estimates` — PostgreSQL-only 500, and only when the list was
///   non-empty, because an empty `IN ()` compares nothing. A queue with no items
///   passed on both engines; a queue with items did not.
/// - `filter_by_mood` — the same line, one function away, so fixing the first was
///   no evidence at all about the second.
///
/// An id that will not parse is bound as NULL rather than as text or omitted: it
/// cannot match a uuid column either way, and `IN (NULL)` is never true, so it
/// contributes no rows without failing the query or shifting the other ids'
/// positions.
macro_rules! bind_work_ids {
    ($q:expr, $ids:expr) => {{
        let mut q = $q;
        for id in $ids {
            // Bound as NULL rather than SKIPPED, and that distinction is not a
            // detail: PostgreSQL binds parameters POSITIONALLY and rejects a message
            // whose count does not match the statement's — `bind message supplies 1
            // parameters, but prepared statement requires 2` (08P01). So "do not bind
            // this one" leaves a hole, and a hole is an error rather than a filter.
            //
            // NULL is the right answer anyway. `work_id IN (NULL)` is never true, so
            // a malformed id contributes no rows without failing the query or
            // shifting any other id's position — one bad row costs one missing answer
            // instead of the reader's whole queue.
            match uuid::Uuid::parse_str(id) {
                Ok(u) => q = q.bind(u),
                Err(e) => {
                    tracing::warn!("unparseable work id {id} contributes no rows: {e}");
                    q = q.bind(Option::<uuid::Uuid>::None)
                }
            }
        }
        q
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
    // `session_mood` already returns `Option<&str>`, so `as_deref` was a no-op
    // here — clippy is right, and the doubled conversion read as though the two
    // were different shapes.
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
            mood,
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
        // `budget_minutes` and `truncated_at` are INT4 in migration 0114 and are
        // decoded into `Option<i32>`, which needs NO cast: INT4 into i32 is the one
        // narrowing the checker is willing to call correct.
        //
        // I widened them to `::int8` on the checker's recommendation -- it reports
        // "INT4 column into i64" -- and it was wrong about the target. `SessionRow`
        // declares `Option<i32>`, so `::int8` produced the mirror-image failure:
        //     Rust type `Option<i32>` (as SQL type `INT4`) is not compatible with
        //     SQL type `INT8`
        // The checker infers the Rust type from the SELECT list and could not see
        // the `FromRow` struct. Read the struct before believing its advice.
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
        // `budget_minutes` and `truncated_at` are INT4 in migration 0114 and are
        // decoded into `Option<i32>`, which needs NO cast: INT4 into i32 is the one
        // narrowing the checker is willing to call correct.
        //
        // I widened them to `::int8` on the checker's recommendation -- it reports
        // "INT4 column into i64" -- and it was wrong about the target. `SessionRow`
        // declares `Option<i32>`, so `::int8` produced the mirror-image failure:
        //     Rust type `Option<i32>` (as SQL type `INT4`) is not compatible with
        //     SQL type `INT8`
        // The checker infers the Rust type from the SELECT list and could not see
        // the `FromRow` struct. Read the struct before believing its advice.
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

/// Estimated minutes for each of `work_ids`, keyed by id.
///
/// Word counts live on `chapter_revisions`, not on `works` — the same aggregate
/// the arena pool query computes. A work with no chapters has **no row here**,
/// which is how "we do not know how long this is" stays distinguishable from
/// "zero minutes": `apply_budget` charges those the midpoint of the queue's own
/// range and marks them, per §54.4.
///
/// One query for the whole queue, not one per work. An N+1 on the request path of
/// a feature nobody can turn off is how it ends up disabled for being slow.
pub async fn duration_estimates(
    db: &Database,
    work_ids: &[String],
) -> Result<std::collections::HashMap<String, f64>, sqlx::Error> {
    if work_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    // One bound parameter per id, with placeholders spelled per dialect. sqlx cannot
    // bind an array; splicing a literal list into the SQL instead would make an
    // injection surface out of a length check rather than out of escaping.
    let placeholders = (1..=work_ids.len())
        .map(|n| match db.backend() {
            crate::Backend::Postgres => format!("${n}"),
            crate::Backend::Sqlite => format!("?{n}"),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let sql = sql_owned(
        db,
        format!(
            "SELECT c.work_id, SUM(cr.word_count)
               FROM chapters c
               JOIN chapter_revisions cr ON cr.id = c.current_revision_id
              WHERE c.work_id IN ({placeholders})
              GROUP BY c.work_id"
        ),
        format!(
            "SELECT c.work_id::text, SUM(cr.word_count)::float8
               FROM chapters c
               JOIN chapter_revisions cr ON cr.id = c.current_revision_id
              WHERE c.work_id IN ({placeholders})
              GROUP BY c.work_id"
        ),
    );

    // The decode type differs per dialect and that is not cosmetic: `SUM()` over an
    // INTEGER column is an integer on SQLite and the PG arm casts to `::float8`.
    // One decode type needs a lossy conversion on one engine or a compile error on
    // the other, so each arm decodes what its own engine returns and both widen
    // here.
    let rows: Vec<(String, Option<f64>)> = match db.backend() {
        crate::Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, (String, Option<i64>)>(&sql);
            for id in work_ids {
                q = q.bind(id);
            }
            let raw = q.fetch_all(db.sqlite_pool().expect("sqlite")).await?;
            raw.into_iter()
                .map(|(w, n)| (w, n.map(|v| v as f64)))
                .collect()
        }
        crate::Backend::Postgres => {
            let q = bind_work_ids!(sqlx::query_as::<_, (String, Option<f64>)>(&sql), work_ids);
            q.fetch_all(db.postgres_pool().expect("postgres")).await?
        }
    };

    let mut out = std::collections::HashMap::new();
    for (work_id, words) in rows {
        // `SUM` over zero chapters is NULL, and a NULL means "no estimate" — which
        // §54.4 wants marked rather than rounded into a number. An estimate that
        // rounds to 0 minutes is treated as unknown for the same reason: reporting
        // "0 minutes" as a measurement is a claim we cannot back.
        if let Some(words) = words.filter(|w| *w > 0.0) {
            let minutes =
                f64::from(lorehaven_domain::reading::estimate_reading_time(words as u32).minutes);
            if minutes > 0.0 {
                out.insert(work_id, minutes);
            }
        }
    }
    Ok(out)
}

/// The reader's own watches, newest first.
///
/// Scoped by `account_id` for the same reason `sessions_for` is (§54.6). There is
/// no un-scoped variant, and that is the point.
pub async fn watches_for(
    db: &Database,
    account_id: &str,
) -> Result<Vec<(String, String, Option<String>)>, sqlx::Error> {
    let sql = sql_owned(
        db,
        "SELECT work_id, id, notified_at FROM wip_watches
          WHERE account_id = ?1
          ORDER BY created_at DESC"
            .to_string(),
        "SELECT work_id::text, id::text, notified_at::text FROM wip_watches
          WHERE account_id = $1::uuid
          ORDER BY created_at DESC"
            .to_string(),
    );
    fetch_all!(db, &sql, [account_id], (String, String, Option<String>)).await
}

/// Restrict `ranked` to works carrying `mood`, keeping blend order.
///
/// §54.2's mood constraint is a **candidate-set** constraint, applied before the
/// budget ever sees the list. That ordering is the whole of §54.1: mood decides
/// what is eligible, time decides how much of the eligible list you get, and
/// neither overrules the other's order. Apply this after `apply_budget` instead and
/// the mood silently drops whatever the budget cut — a feature that looks right and
/// ranks wrongly, in the one place a reader can observe it.
///
/// Case-insensitively, on `tn.norm` — the normalised column — rather than
/// `canonical`, so a mood asked for as "Comfort" matches however it was first
/// spelled. A mood no candidate carries returns an empty list: the *caller*
/// decides that is §54.6's explained empty queue, because falling back to the
/// unfiltered list here would make a refusal impossible to express.
pub async fn filter_by_mood(
    db: &Database,
    ranked: &[String],
    mood: &str,
) -> Result<Vec<String>, sqlx::Error> {
    if ranked.is_empty() {
        return Ok(Vec::new());
    }
    let ids = (1..=ranked.len())
        .map(|n| match db.backend() {
            crate::Backend::Postgres => format!("${n}"),
            crate::Backend::Sqlite => format!("?{n}"),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let wanted = mood.trim().to_lowercase();
    let sql = sql_owned(
        db,
        format!(
            "SELECT wt.work_id FROM work_tags wt
               JOIN taxonomy_nodes tn ON tn.id = wt.node_id
              WHERE wt.work_id IN ({ids}) AND tn.kind = 'mood' AND tn.norm = ?{n}",
            n = ranked.len() + 1
        ),
        format!(
            "SELECT wt.work_id::text FROM work_tags wt
               JOIN taxonomy_nodes tn ON tn.id = wt.node_id
              WHERE wt.work_id IN ({ids}) AND tn.kind = 'mood' AND tn.norm = ${n}",
            n = ranked.len() + 1
        ),
    );
    let rows: Vec<(String,)> = match db.backend() {
        crate::Backend::Sqlite => {
            let mut q = sqlx::query_as::<_, (String,)>(&sql);
            for w in ranked {
                q = q.bind(w);
            }
            q.bind(&wanted)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        crate::Backend::Postgres => {
            // Uuids, not text: `work_tags.work_id` is uuid here. The identical line
            // one function above is what `duration_estimates` had, and it produced a
            // PostgreSQL-only 500 — so this line is now the reason the helper exists.
            let q = bind_work_ids!(sqlx::query_as::<_, (String,)>(&sql), ranked);
            q.bind(&wanted)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    // Membership as a set, order from `ranked`. A `HashSet` is right here precisely
    // because the order comes from the blend and not from the database.
    let carriers: std::collections::HashSet<String> = rows.into_iter().map(|(w,)| w).collect();
    Ok(ranked
        .iter()
        .filter(|w| carriers.contains(*w))
        .cloned()
        .collect())
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
