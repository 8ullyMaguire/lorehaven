//! Discovery repository: taste profiles, recommendations, recipes, dashboards.
//!
//! Spec §16.1–16.8. Both dialects.

use crate::search::content_filter_sql;
use crate::{sql_owned, Backend, Database};
use anyhow::Result;
use lorehaven_domain::ids::WorkId;
use serde::Serialize;
use sqlx::Row;

/// A taste profile row.
#[derive(Debug, Clone, Serialize)]
pub struct TasteProfile {
    pub account: String,
    pub signals: serde_json::Value,
    pub computed_at: String,
}

/// A recommendation candidate.
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub work_id: WorkId,
    pub score: i64,
    pub reason: String,
}

/// Blind Date: one work per reader per day, chosen without looking at their profile.
///
/// Closes gap B on the ideas list (#14). The audit's finding was that this is
/// *implementation of an existing spec surface*, not missing design — `spec.md:2858`
/// lists Blind Date among the discovery surfaces beside Recent and trending, and
/// `spec.md:7462` gives the bot a `/blind-date` command. Nothing existed under any
/// spelling, so the surface had never been built.
///
/// **The selection is deterministic in (account, day), which is the whole design.** A
/// reader who reloads gets the same work; a reader who tells a friend gets a different
/// one; and tomorrow's pick differs from today's. Three consequences follow, and each
/// is why this is not `ORDER BY random()`:
///
/// 1. **Reload-stability.** A random pick changes on every refresh, which makes the
///    work impossible to bookmark, rate, or discuss with anyone — the reader has no
///    stable referent. Deriving the pick from a hash of (account, date) means the page
///    can be reloaded, shared as a link, and closed and reopened tomorrow morning.
///
/// 2. **A reader cannot reroll.** The obvious way to implement this is "pick a random
///    work, and if the reader has seen it, pick another" — but that leaks the pool.
///    Refreshing until the work changes is a legitimate reader behaviour and the
///    surface has to survive it, so the pick cannot depend on how many times the reader
///    has asked.
///
/// 3. **Different readers see different works**, because the account id is in the hash.
///    A single daily "work of the day" for everyone would be a trending slot, which
///    §16.1a already has; Blind Date exists to get *outside* the profile.
///
/// Eligibility is deliberately narrow, and each clause is a decision:
///
/// * **published, public, not deleted.** A work nobody can read cannot be the answer.
/// * **not bookmarked by this reader.** Showing someone a thing they saved is the same
///   mistake every other strategy avoids.
/// * **not by a pseud the reader has already read.** Blind Date is meant to surface an
///   unknown *author* as often as an unknown work, and an author the reader follows is
///   neither blind nor unknown.
/// * **`visibility = 'public'`** explicitly, not via lifecycle alone: `lifecycle` says
///   whether a work is published, `visibility` says whether it is listable, and a work
///   can be published-but-unlisted (a direct-link-only work). Blind Date is a discovery
///   surface, so unlisted stays out.
///
/// **Ordering is by a hash-derived key, not by a computed score.** There is no score
/// here on purpose: a score would reintroduce exactly the popularity weighting that makes
/// the other strategies blind, and "random but weighted" is just trending with extra
/// steps. Within the eligible set the ordering is arbitrary by design, and the
/// eligibility clauses are the only thing that shapes it.
///
/// **Backdated dates are treated as published on their schedule.** A work scheduled for
/// last week has been eligible since last week; treating `published_at > today` as
/// future would hide it for as long as the schedule was wrong.
/// Read a taste profile (owner only).
pub async fn blind_date_work(db: &Database, account: &str, today: &str) -> Result<Option<String>> {
    // One seed per (account, day). Hashing rather than randomising is what makes the
    // pick stable across reloads -- see the doc comment.
    // `works.id` is uuid on PostgreSQL and TEXT on SQLite, and `subject_id` /
    // `owner_pseud_id` are TEXT on both -- so the comparisons need `::text` on the uuid
    // side and the returned column needs it too for sqlx to decode a `String`. Both are
    // PostgreSQL-only syntax, so they come from one fragment rather than being written
    // into the literal. This is the fourth appearance of this split in the codebase
    // (see `rec_strategy.rs`, `payout_store.rs`, `series_recs.rs`).
    let seed = blind_date_seed(account, today);
    let id_cast = match db.backend() {
        crate::Backend::Postgres => "::text",
        crate::Backend::Sqlite => "",
    };
    let sql = format!(
        r#"
        SELECT w.id{id_cast} AS id
        FROM works w
        WHERE w.lifecycle = 'published'
          AND w.visibility IS NOT NULL
          AND w.deleted_at IS NULL
          -- A scheduled work is eligible from the day it was scheduled to appear, so
          -- COALESCE treats a missing `published_at` as the creation date rather than
          -- excluding the work outright.
          AND date(COALESCE(w.published_at, w.created_at)) <= date('{today}')
          AND w.id NOT IN (
              SELECT subject_id FROM bookmarks
              WHERE account_id = '{account}' AND subject_type = 'work'
          )
          AND w.owner_pseud_id NOT IN (
              -- Any pseud this reader has already read something by. Read, not
              -- bookmarked: a reader who finished one book by an author has met them.
              SELECT rp.id
              FROM pseuds rp
              JOIN works rw ON rw.owner_pseud_id = rp.id
              JOIN reading_status rs ON rs.subject_id = rw.id
              WHERE rs.account_id = '{account}'
                AND rs.subject_type = 'work'
                AND rs.status = 'finished'
                
          )
        "#,
    );
    let ids: Vec<String> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    // Ordered in Rust, not SQL, because neither engine has a portable hash function:
    // PostgreSQL has `md5`, SQLite has none of `md5`/`sha*` without an extension, and
    // `random()` differs per engine and per call. Fetching every eligible id and sorting
    // on an FNV-1a of (id, seed) is the portable version of the same idea, and it keeps
    // the ordering logic next to `blind_date_seed`, which is where a reader would look
    // for it.
    //
    // The cost is bounded by the eligible set. On a catalogue of a few hundred thousand
    // works this would need the ordering pushed back into SQL with an indexed hash
    // column -- at which point this becomes a `blind_date_assignments` table, and the
    // pick is stored per (account, day) rather than derived. Not yet; the derivation is
    // what makes the surface stateless today.
    Ok(ids
        .into_iter()
        .min_by_key(|id| blind_date_order_key(id, &seed))
        .map(|id| id.to_string()))
}

/// FNV-1a over (work id, seed), as the ordering key.
///
/// Separate from `blind_date_seed` on purpose: the seed is per (account, day) and is the
/// same for every candidate, so it cannot order anything on its own. This mixes the
/// candidate's id back in, which is what makes the order differ between candidates.
fn blind_date_order_key(work_id: &str, seed: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in work_id.as_bytes().iter().chain(b"|").chain(seed.as_bytes()) {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The per-(account, day) ordering seed.
///
/// FNV-1a over the two inputs rather than a cryptographic hash: this needs to spread
/// work ids evenly across a sort order, not resist an attacker. `DefaultHasher` is not
/// usable because its output is not guaranteed stable across Rust releases, and a seed
/// that changes between compiler versions would silently reshuffle every reader's
/// Blind Date on the next toolchain bump.
fn blind_date_seed(account: &str, today: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in account
        .as_bytes()
        .iter()
        .chain(b"|")
        .chain(today.as_bytes())
    {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

pub async fn taste_profile_for(db: &Database, account: &str) -> Result<Option<TasteProfile>> {
    let row: Option<(String, String, String)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(
                "SELECT account, signals, computed_at FROM taste_profiles WHERE account = ?",
            )
            .bind(account)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(
                "SELECT account, signals, computed_at FROM taste_profiles WHERE account = $1",
            )
            .bind(account)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    Ok(row.map(|(account, signals, computed_at)| TasteProfile {
        account,
        signals: serde_json::from_str(&signals).unwrap_or_default(),
        computed_at,
    }))
}

/// Save a taste profile.
pub async fn save_taste_profile(
    db: &Database,
    account: &str,
    signals: &serde_json::Value,
    computed_at: &str,
) -> Result<()> {
    let signals_json = serde_json::to_string(signals)?;
    match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(
                "INSERT OR REPLACE INTO taste_profiles (account, signals, computed_at) VALUES (?, ?, ?)",
            )
            .bind(account)
            .bind(&signals_json)
            .bind(computed_at)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        crate::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO taste_profiles (account, signals, computed_at) VALUES ($1, $2, $3) ON CONFLICT (account) DO UPDATE SET signals = EXCLUDED.signals, computed_at = EXCLUDED.computed_at",
            )
            .bind(account)
            .bind(&signals_json)
            .bind(computed_at)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

/// Clear a taste profile.
pub async fn clear_taste_profile(db: &Database, account: &str) -> Result<()> {
    match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query("DELETE FROM taste_profiles WHERE account = ?")
                .bind(account)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        crate::Backend::Postgres => {
            sqlx::query("DELETE FROM taste_profiles WHERE account = $1")
                .bind(account)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Recompute the taste profile for an account from reading history.
pub async fn recompute_taste_profile(db: &Database, account: &str) -> Result<()> {
    // Aggregate from reading history, ratings, notes, bookmarks
    let signals = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as::<_, (String,)>(
                "SELECT COALESCE(json_group_array(DISTINCT wt.node_id), '[]')
                 FROM reading_history_entry rh
                 JOIN work_tags wt ON wt.work_id = rh.subject_id
                 WHERE rh.account_id = ?
                 AND rh.subject_type = 'work'
                 LIMIT 100",
            )
            .bind(account)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as::<_, (String,)>(
                "SELECT COALESCE(json_agg(DISTINCT wt.node_id)::text, '[]')
                 FROM reading_history_entry rh
                 JOIN work_tags wt ON wt.work_id = rh.subject_id::uuid
                 WHERE rh.account_id = $1::uuid
                 AND rh.subject_type = 'work'
                 LIMIT 100",
            )
            .bind(account)
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    let signals_json: serde_json::Value = serde_json::from_str(&signals.0).unwrap_or_default();
    let now = crate::identity::now_rfc3339();
    save_taste_profile(db, account, &signals_json, &now).await
}

/// Get public recommendations (popular recent works).
///
/// `account` is the viewer's account, used only to exclude the works they have
/// content-filtered (spec §46.4, §46.7.1). `None` -- an anonymous visitor --
/// filters nothing, which is correct rather than a convenience.
pub async fn public_recommendations(
    db: &Database,
    viewer_pseud: Option<uuid::Uuid>,
    limit: i64,
) -> Result<Vec<WorkId>> {
    let rules = content_filter_sql::for_pseud(db, viewer_pseud).await?;
    let filter = content_filter_sql::exclusion_for(&rules, "w.id");
    // The exclusion carries `?` of its own and is renumbered once by
    // `sql_owned`, so it has to be bound in the position it appears in the text:
    // here inside the WHERE, ahead of `LIMIT ?`. Binding order is positional and
    // unchecked, so a swapped pair is a wrong answer, not an error.
    let sqlite = format!(
        "SELECT w.id FROM works w
         WHERE w.lifecycle = 'published' AND w.visibility = 'public'
           {filter_clause}
         ORDER BY w.updated_at DESC
         LIMIT ?",
        filter_clause = filter.clause()
    );
    let postgres = format!(
        "SELECT w.id::text FROM works w
         WHERE w.lifecycle = 'published' AND w.visibility = 'public'
           {filter_clause}
         ORDER BY w.updated_at DESC
         LIMIT ?",
        filter_clause = filter.clause()
    );
    // The bind sequence is repeated per arm on purpose. `sqlx::query_as`
    // monomorphises on the backend, so one built query cannot be handed both a
    // `SqlitePool` and a `PgPool`; a generic helper over the executor needs a
    // bound the compiler rejects as a cycle. Only the renumbered statement
    // differs, and it comes from one `sql_owned` call, so the two arms cannot
    // disagree about the SQL itself.
    let rows: Vec<(String,)> = match db.backend() {
        crate::Backend::Sqlite => {
            let mut query = sqlx::query_as::<_, (String,)>(&sqlite);
            for bind in &filter.binds {
                query = query.bind(bind);
            }
            query = query.bind(limit);
            query.fetch_all(db.sqlite_pool().expect("sqlite")).await?
        }
        crate::Backend::Postgres => {
            let sql = sql_owned(db, sqlite, postgres);
            let mut query = sqlx::query_as::<_, (String,)>(&sql);
            for bind in &filter.binds {
                query = query.bind(bind);
            }
            query = query.bind(limit);
            query
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|(id,)| id.parse().unwrap_or_default())
        .collect())
}

/// Get personalized recommendations based on taste profile.
pub async fn personalized_recommendations(
    db: &Database,
    account: &str,
    viewer_pseud: Option<uuid::Uuid>,
    limit: i64,
) -> Result<Vec<WorkId>> {
    // First try to get the taste profile
    let profile = taste_profile_for(db, account).await?;
    match profile {
        Some(_) => {
            // Use taste profile to find similar works, minus the viewer's
            // content-filtered ones (spec §46.4, §46.7.1). The account's own
            // rules are the ones that can be read here: this engine is keyed by
            // account, and a pseud-scoped rule is not reachable from it.
            let rules = content_filter_sql::for_pseud(db, viewer_pseud).await?;
            let filter = content_filter_sql::exclusion_for(&rules, "w.id");
            let sqlite = format!(
                "SELECT DISTINCT w.id FROM works w
                 JOIN work_tags wt ON wt.work_id = w.id
                 WHERE w.lifecycle = 'published' AND w.visibility = 'public'
                 AND wt.node_id IN (
                     SELECT json_each.value FROM taste_profiles tp,
                     json_each(tp.signals)
                     WHERE tp.account = ?
                 )
                 AND w.owner_pseud_id NOT IN (
                     SELECT id FROM pseuds WHERE account_id = ?
                 )
                 {filter_clause}
                 ORDER BY w.updated_at DESC
                 LIMIT ?",
                filter_clause = filter.clause()
            );
            let postgres = format!(
                "SELECT DISTINCT w.id::text FROM works w
                 JOIN work_tags wt ON wt.work_id = w.id
                 WHERE w.lifecycle = 'published' AND w.visibility = 'public'
                 AND wt.node_id IN (
                     SELECT json_array_elements_text(tp.signals::json)
                     FROM taste_profiles tp
                     WHERE tp.account = ?
                 )
                 AND w.owner_pseud_id NOT IN (
                     SELECT id FROM pseuds WHERE account_id = ?::uuid
                 )
                 {filter_clause}
                 ORDER BY w.updated_at DESC
                 LIMIT ?",
                filter_clause = filter.clause()
            );
            // See the note in `public_recommendations`: the bind sequence is
            // repeated per arm because `query_as` cannot cross backends.
            let rows: Vec<(String,)> = match db.backend() {
                crate::Backend::Sqlite => {
                    // The exclusion's `?` appear in the WHERE clause, ahead of
                    // `LIMIT ?` in the text, so they are bound ahead of it here.
                    let mut query = sqlx::query_as::<_, (String,)>(&sqlite)
                        .bind(account)
                        .bind(account);
                    for bind in &filter.binds {
                        query = query.bind(bind);
                    }
                    query = query.bind(limit);
                    query.fetch_all(db.sqlite_pool().expect("sqlite")).await?
                }
                crate::Backend::Postgres => {
                    let sql = sql_owned(db, sqlite, postgres);
                    let mut query = sqlx::query_as::<_, (String,)>(&sql)
                        .bind(account)
                        .bind(account);
                    for bind in &filter.binds {
                        query = query.bind(bind);
                    }
                    query = query.bind(limit);
                    query
                        .fetch_all(db.postgres_pool().expect("postgres"))
                        .await?
                }
            };
            Ok(rows
                .into_iter()
                .map(|(id,)| id.parse().unwrap_or_default())
                .collect())
        }
        None => public_recommendations(db, viewer_pseud, limit).await,
    }
}

/// Media-reference collaborative recommendations (spec §32.7.3, §9.10).
///
/// Finds works that share media references with the user's bookmarked works.
/// The user's library works are the ones they have bookmarked; we look up
/// which media references those works use, then find other works using the
/// same references. Works with more shared references rank higher.
pub async fn media_reference_collaborative_recommendations(
    db: &Database,
    account_id: &str,
    viewer_pseud: Option<uuid::Uuid>,
    limit: i64,
) -> Result<Vec<WorkId>> {
    // Step 1: works sharing media references with the reader's bookmarked works.
    // Step 2: exclude the reader's content-filtered works (spec §46.4, §46.7.1).
    // Step 3: rank by count of shared references, descending.
    //
    // The work id lives on `work_media_references`, not on `works`, so the
    // exclusion correlates on `wmr.work_id` -- the reason the predicate takes an
    // expression rather than a table name.
    let rules = content_filter_sql::for_pseud(db, viewer_pseud).await?;
    let filter = content_filter_sql::exclusion_for(&rules, "wmr.work_id");
    // Both arms are written with `?` in bind order and renumbered once. The
    // account appears three times in the original text, so it is bound three
    // times here too rather than relying on `$1` reuse, which keeps the two
    // dialects' bind counts identical.
    let sqlite = format!(
        "SELECT wmr.work_id
         FROM work_media_references wmr
         WHERE wmr.media_reference_id IN (
             SELECT DISTINCT wmr2.media_reference_id
             FROM work_media_references wmr2
             JOIN bookmarks b ON b.subject_id = wmr2.work_id AND b.subject_type = 'work'
             WHERE b.account_id = ?
         )
         AND wmr.work_id NOT IN (
             SELECT subject_id FROM bookmarks WHERE account_id = ? AND subject_type = 'work'
         )
         AND wmr.work_id NOT IN (
             SELECT w.id FROM works w
             JOIN pseuds p ON p.id = w.owner_pseud_id
             WHERE p.account_id = ?
         )
         AND wmr.deleted_at IS NULL
         {filter_clause}
         GROUP BY wmr.work_id
         ORDER BY COUNT(DISTINCT wmr.media_reference_id) DESC, wmr.work_id
         LIMIT ?",
        filter_clause = filter.clause()
    );
    let postgres = format!(
        "SELECT wmr.work_id::text
         FROM work_media_references wmr
         WHERE wmr.media_reference_id IN (
             SELECT DISTINCT wmr2.media_reference_id
             FROM work_media_references wmr2
             JOIN bookmarks b ON b.subject_id = wmr2.work_id AND b.subject_type = 'work'
             WHERE b.account_id = ?::uuid
         )
         AND wmr.work_id NOT IN (
             SELECT subject_id FROM bookmarks WHERE account_id = ?::uuid AND subject_type = 'work'
         )
         AND wmr.work_id NOT IN (
             SELECT w.id FROM works w
             JOIN pseuds p ON p.id = w.owner_pseud_id
             WHERE p.account_id = ?::uuid
         )
         AND wmr.deleted_at IS NULL
         {filter_clause}
         GROUP BY wmr.work_id
         ORDER BY COUNT(DISTINCT wmr.media_reference_id) DESC, wmr.work_id
         LIMIT ?",
        filter_clause = filter.clause()
    );
    // The exclusion's `?` sit in the WHERE, ahead of `LIMIT ?`, so they bind
    // ahead of it. See `public_recommendations` for why this is per-arm.
    let rows: Vec<(String,)> = match db.backend() {
        Backend::Sqlite => {
            let mut query = sqlx::query_as::<_, (String,)>(&sqlite)
                .bind(account_id)
                .bind(account_id)
                .bind(account_id);
            for bind in &filter.binds {
                query = query.bind(bind);
            }
            query = query.bind(limit);
            query.fetch_all(db.sqlite_pool().expect("sqlite")).await?
        }
        Backend::Postgres => {
            let sql = sql_owned(db, sqlite, postgres);
            let mut query = sqlx::query_as::<_, (String,)>(&sql)
                .bind(account_id)
                .bind(account_id)
                .bind(account_id);
            for bind in &filter.binds {
                query = query.bind(bind);
            }
            query = query.bind(limit);
            query
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };

    Ok(rows
        .into_iter()
        .map(|(id,)| id.parse().unwrap_or_default())
        .collect())
}

/// An operator-set work affinity (private, never rendered publicly).
#[derive(Debug, Clone, Serialize)]
pub struct OperatorAffinity {
    pub work_id: String,
    pub affinity_bp: i64,
    pub operator: String,
    pub rationale: String,
    pub set_at: String,
}

/// Set or replace the operator affinity for a work. Audit-logged.
///
/// affinity_bp is clamped to -5000..=10000 (base-point range).
pub async fn set_operator_affinity(
    db: &Database,
    work_id: &str,
    affinity_bp: i64,
    operator: &str,
    rationale: &str,
) -> Result<()> {
    let clamped = affinity_bp.clamp(-5000, 10000);
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(
                "INSERT OR REPLACE INTO operator_affinities (work_id, affinity_bp, operator, rationale, set_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(work_id)
            .bind(clamped)
            .bind(operator)
            .bind(rationale)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        crate::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO operator_affinities (work_id, affinity_bp, operator, rationale, set_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (work_id) DO UPDATE SET
                    affinity_bp = EXCLUDED.affinity_bp,
                    operator = EXCLUDED.operator,
                    rationale = EXCLUDED.rationale,
                    set_at = EXCLUDED.set_at",
            )
            .bind(work_id)
            .bind(clamped)
            .bind(operator)
            .bind(rationale)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }

    // Audit trail: operator action recorded server-side.
    let audit_doc = serde_json::json!({
        "work_id": work_id,
        "affinity_bp": clamped,
        "rationale": rationale,
    });
    crate::governance::audit_append(
        db,
        operator,
        "operator.set_affinity",
        "work",
        work_id,
        &audit_doc.to_string(),
    )
    .await?;
    Ok(())
}

/// List all operator affinities. Used internally to apply ranking multipliers.
pub async fn list_operator_affinities(db: &Database) -> Result<Vec<OperatorAffinity>> {
    let rows: Vec<(String, i64, String, String, String)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(
                "SELECT work_id, affinity_bp, operator, rationale, set_at
                 FROM operator_affinities ORDER BY set_at DESC",
            )
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(
                "SELECT work_id, affinity_bp::bigint, operator, rationale, set_at
                 FROM operator_affinities ORDER BY set_at DESC",
            )
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(
            |(work_id, affinity_bp, operator, rationale, set_at)| OperatorAffinity {
                work_id,
                affinity_bp,
                operator,
                rationale,
                set_at,
            },
        )
        .collect())
}

/// Save a recipe (create or update).
pub async fn save_recipe(
    db: &Database,
    id: &str,
    owner: &str,
    name: &str,
    document: &serde_json::Value,
    is_public: bool,
    created_at: &str,
) -> Result<()> {
    let now = created_at.to_string();
    match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(
                "INSERT OR REPLACE INTO recipes (id, owner, name, document, is_public, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(owner)
            .bind(name)
            .bind(document.to_string())
            .bind(is_public as i64)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        crate::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO recipes (id, owner, name, document, is_public, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (id) DO UPDATE SET
                    owner = EXCLUDED.owner,
                    name = EXCLUDED.name,
                    document = EXCLUDED.document,
                    is_public = EXCLUDED.is_public,
                    created_at = EXCLUDED.created_at",
            )
            .bind(id)
            .bind(owner)
            .bind(name)
            .bind(document.to_string())
            .bind(is_public as i64)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

/// Read a recipe by ID, enforcing visibility: a non-public recipe
/// is only returned to its owner.
pub async fn get_recipe(db: &Database, id: &str, viewer: &str) -> Result<Option<RecipeRow>> {
    let row: Option<(String, String, String, String, i64, String)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(
                "SELECT id, owner, name, document, is_public, created_at FROM recipes WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        crate::Backend::Postgres => sqlx::query_as(
            "SELECT id, owner, name, document, is_public, created_at FROM recipes WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(db.postgres_pool().expect("postgres"))
        .await?,
    };
    match row {
        Some((id, owner, name, document, is_public, created_at)) => {
            if is_public == 0 && owner != viewer {
                return Ok(None);
            }
            Ok(Some(RecipeRow {
                id,
                owner,
                name,
                document: serde_json::from_str(&document).unwrap_or_default(),
                is_public: is_public != 0,
                created_at,
            }))
        }
        None => Ok(None),
    }
}

/// List recipes visible to the viewer (all public ones + the viewer's own private ones).
pub async fn list_recipes(db: &Database, viewer: &str) -> Result<Vec<RecipeRow>> {
    let rows: Vec<(String, String, String, String, i64, String)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(
                "SELECT id, owner, name, document, is_public, created_at FROM recipes WHERE is_public = 1 OR owner = ? ORDER BY created_at DESC",
            )
            .bind(viewer)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(
                "SELECT id, owner, name, document, is_public, created_at FROM recipes WHERE is_public = 1 OR owner = $1 ORDER BY created_at DESC",
            )
            .bind(viewer)
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(
            |(id, owner, name, document, is_public, created_at)| RecipeRow {
                id,
                owner,
                name,
                document: serde_json::from_str(&document).unwrap_or_default(),
                is_public: is_public != 0,
                created_at,
            },
        )
        .collect())
}

/// Update a recipe's name or document (owner-only).
pub async fn update_recipe(
    db: &Database,
    id: &str,
    owner: &str,
    name: &str,
    document: &serde_json::Value,
) -> Result<bool> {
    let updated = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query("UPDATE recipes SET name = ?, document = ? WHERE id = ? AND owner = ?")
                .bind(name)
                .bind(document.to_string())
                .bind(id)
                .bind(owner)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?
                .rows_affected()
        }
        crate::Backend::Postgres => {
            sqlx::query("UPDATE recipes SET name = $1, document = $2 WHERE id = $3 AND owner = $4")
                .bind(name)
                .bind(document.to_string())
                .bind(id)
                .bind(owner)
                .execute(db.postgres_pool().expect("postgres"))
                .await?
                .rows_affected()
        }
    };
    Ok(updated > 0)
}

/// Delete a recipe (owner-only).
pub async fn delete_recipe(db: &Database, id: &str, owner: &str) -> Result<bool> {
    let deleted = match db.backend() {
        crate::Backend::Sqlite => sqlx::query("DELETE FROM recipes WHERE id = ? AND owner = ?")
            .bind(id)
            .bind(owner)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        crate::Backend::Postgres => sqlx::query("DELETE FROM recipes WHERE id = $1 AND owner = $2")
            .bind(id)
            .bind(owner)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(deleted > 0)
}

/// A recipe row read from the database.
#[derive(Debug, Clone, Serialize)]
pub struct RecipeRow {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub document: serde_json::Value,
    pub is_public: bool,
    pub created_at: String,
}

/// A dashboard layout row read from the database.
#[derive(Debug, Clone, Serialize)]
pub struct DashboardLayout {
    pub account: String,
    pub slots: serde_json::Value,
    pub updated_at: String,
}

/// Read a dashboard layout for an account.
pub async fn get_dashboard_layout(db: &Database, account: &str) -> Result<Option<DashboardLayout>> {
    let row: Option<(String, String, String)> = match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query_as(
                "SELECT account, slots, updated_at FROM dashboard_layouts WHERE account = ?",
            )
            .bind(account)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
        }
        crate::Backend::Postgres => {
            sqlx::query_as(
                "SELECT account, slots, updated_at FROM dashboard_layouts WHERE account = $1",
            )
            .bind(account)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?
        }
    };
    Ok(row.map(|(account, slots, updated_at)| DashboardLayout {
        account,
        slots: serde_json::from_str(&slots).unwrap_or_default(),
        updated_at,
    }))
}

/// Save a dashboard layout for an account.
pub async fn save_dashboard_layout(
    db: &Database,
    account: &str,
    slots: &serde_json::Value,
) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    let slots_str = slots.to_string();
    match db.backend() {
        crate::Backend::Sqlite => {
            sqlx::query(
                "INSERT OR REPLACE INTO dashboard_layouts (account, slots, updated_at)
                 VALUES (?, ?, ?)",
            )
            .bind(account)
            .bind(&slots_str)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        crate::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO dashboard_layouts (account, slots, updated_at)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (account) DO UPDATE SET
                    slots = EXCLUDED.slots,
                    updated_at = EXCLUDED.updated_at",
            )
            .bind(account)
            .bind(&slots_str)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

/// Titles and author handles for a batch of work ids, for feeds that show
/// what a work is instead of the uuid that identifies it. Works that went
/// missing between candidacy and rendering are simply absent from the map.
pub async fn work_details_for(
    db: &Database,
    ids: &[String],
) -> Result<std::collections::HashMap<String, (String, String)>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let mut map = std::collections::HashMap::new();
    match db.backend() {
        Backend::Sqlite => {
            let placeholders = std::iter::repeat_n("?", ids.len())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT w.id AS wid, w.title, COALESCE(p.handle, '') AS handle \
                 FROM works w LEFT JOIN pseuds p ON p.id = w.owner_pseud_id \
                 WHERE w.id IN ({placeholders}) AND w.deleted_at IS NULL"
            );
            let mut q = sqlx::query(&sql);
            for id in ids {
                q = q.bind(id);
            }
            let rows = q.fetch_all(db.sqlite_pool().expect("sqlite")).await?;
            for r in rows {
                map.insert(
                    r.get::<String, _>("wid"),
                    (r.get::<String, _>("title"), r.get::<String, _>("handle")),
                );
            }
        }
        Backend::Postgres => {
            let rows = sqlx::query(
                "SELECT w.id::text AS wid, w.title, COALESCE(p.handle, '') AS handle \
                 FROM works w LEFT JOIN pseuds p ON p.id = w.owner_pseud_id \
                 WHERE w.id = ANY($1::uuid[]) AND w.deleted_at IS NULL",
            )
            .bind(ids)
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await?;
            for r in rows {
                map.insert(
                    r.get::<String, _>("wid"),
                    (r.get::<String, _>("title"), r.get::<String, _>("handle")),
                );
            }
        }
    }
    Ok(map)
}
