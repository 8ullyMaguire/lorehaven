//! Repository for collections, challenges, requests, wishlists, and events.
//!
//! Spec §18. Both dialects. This module persists what the domain rule layer
//! (`crates/domain/src/events.rs`) decides is permitted.

use crate::{Backend, Database};
use anyhow::{Context, Result};
use lorehaven_domain::events::{Constraint, ConstraintResult, WorkFacts};
use serde::Serialize;

// ---------------------------------------------------------------------------
// Cursor envelope helper
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Collections
// ---------------------------------------------------------------------------

/// A collection row.
#[derive(Debug, Clone, Serialize)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub owner: String,
    pub item_policy: String,
    pub is_public: bool,
    pub created_at: String,
}

/// Create a collection. Returns the new id.
pub async fn create_collection(
    db: &Database,
    name: &str,
    description: Option<&str>,
    owner: &str,
    item_policy: &str,
    is_public: bool,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let is_public_int = i64::from(is_public);
    let sql = db.sql(
        "INSERT INTO collections (id, name, description, owner, item_policy, is_public, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO collections (id, name, description, owner, item_policy, is_public, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(name)
                .bind(description)
                .bind(owner)
                .bind(item_policy)
                .bind(is_public_int)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(name)
                .bind(description)
                .bind(owner)
                .bind(item_policy)
                .bind(is_public_int)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(id)
}

/// Fetch a collection by id. Its items come from `list_collection_items`; this
/// returns the collection row alone.
pub async fn get_collection(db: &Database, collection_id: &str) -> Result<Option<Collection>> {
    let sql = db.sql(
        "SELECT id, name, description, owner, item_policy, is_public, created_at FROM collections WHERE id = ?",
        "SELECT id, name, description, owner, item_policy, is_public, created_at FROM collections WHERE id = $1",
    );
    let row: Option<CollectionRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(collection_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(collection_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.map(Collection::from))
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct CollectionRow {
    id: String,
    name: String,
    description: Option<String>,
    owner: String,
    item_policy: String,
    is_public: i64,
    created_at: String,
}

impl From<CollectionRow> for Collection {
    fn from(r: CollectionRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            description: r.description,
            owner: r.owner,
            item_policy: r.item_policy,
            is_public: r.is_public != 0,
            created_at: r.created_at,
        }
    }
}

/// List items in a collection.
pub async fn list_collection_items(
    db: &Database,
    collection_id: &str,
) -> Result<Vec<CollectionItem>> {
    let sql = db.sql(
        "SELECT collection_id, work_id, added_by, added_at, note FROM collection_items WHERE collection_id = ?",
        "SELECT collection_id, work_id, added_by, added_at, note FROM collection_items WHERE collection_id = $1",
    );
    let rows: Vec<CollectionItemRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(collection_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(collection_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows.into_iter().map(CollectionItem::from).collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectionItem {
    pub collection_id: String,
    pub work_id: String,
    pub added_by: String,
    pub added_at: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct CollectionItemRow {
    collection_id: String,
    work_id: String,
    added_by: String,
    added_at: String,
    note: Option<String>,
}

impl From<CollectionItemRow> for CollectionItem {
    fn from(r: CollectionItemRow) -> Self {
        Self {
            collection_id: r.collection_id,
            work_id: r.work_id,
            added_by: r.added_by,
            added_at: r.added_at,
            note: r.note,
        }
    }
}

/// Add a work to a collection (no approval row — approval lives in the route
/// layer when the policy demands it).
pub async fn add_collection_item(
    db: &Database,
    collection_id: &str,
    work_id: &str,
    added_by: &str,
    note: Option<&str>,
) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO collection_items (collection_id, work_id, added_by, added_at, note) VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO collection_items (collection_id, work_id, added_by, added_at, note) VALUES ($1, $2, $3, $4, $5)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(collection_id)
                .bind(work_id)
                .bind(added_by)
                .bind(&now)
                .bind(note)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(collection_id)
                .bind(work_id)
                .bind(added_by)
                .bind(&now)
                .bind(note)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Challenges
// ---------------------------------------------------------------------------

/// A challenge row.
#[derive(Debug, Clone, Serialize)]
pub struct Challenge {
    pub id: String,
    pub name: String,
    pub rules: String,
    pub schedule: String,
    pub created_by: String,
    pub created_at: String,
}

/// Create a challenge. Returns the new id.
pub async fn create_challenge(
    db: &Database,
    name: &str,
    rules: &str,
    schedule: &str,
    created_by: &str,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO challenges (id, name, rules, schedule, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?)",
        "INSERT INTO challenges (id, name, rules, schedule, created_by, created_at) VALUES ($1, $2, $3, $4, $5, $6)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(name)
                .bind(rules)
                .bind(schedule)
                .bind(created_by)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(name)
                .bind(rules)
                .bind(schedule)
                .bind(created_by)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(id)
}

/// Fetch a challenge by id.
pub async fn get_challenge(db: &Database, challenge_id: &str) -> Result<Option<Challenge>> {
    let sql = db.sql(
        "SELECT id, name, rules, schedule, created_by, created_at FROM challenges WHERE id = ?",
        "SELECT id::text, name, rules, schedule, created_by, created_at FROM challenges WHERE id = $1",
    );
    let row: Option<ChallengeRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(challenge_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(challenge_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.map(Challenge::from))
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ChallengeRow {
    id: String,
    name: String,
    rules: String,
    schedule: String,
    created_by: String,
    created_at: String,
}

impl From<ChallengeRow> for Challenge {
    fn from(r: ChallengeRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            rules: r.rules,
            schedule: r.schedule,
            created_by: r.created_by,
            created_at: r.created_at,
        }
    }
}

/// Enter a work into a challenge. Evaluates constraints and records the
/// per-constraint pass/fail at entry time (spec §18.2, §7.10 pitfall).
pub async fn enter_challenge(
    db: &Database,
    challenge_id: &str,
    work_id: &str,
    facts: &WorkFacts,
) -> Result<Vec<ConstraintResult>> {
    // Load the challenge's rules document and parse constraints.
    let challenge = get_challenge(db, challenge_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("challenge not found: {challenge_id}"))?;
    let rules: serde_json::Value =
        serde_json::from_str(&challenge.rules).context("parsing challenge rules JSON")?;
    let constraints: Vec<Constraint> =
        if let Some(arr) = rules.get("constraints").and_then(|a| a.as_array()) {
            arr.iter()
                .filter_map(|v| serde_json::from_value::<Constraint>(v.clone()).ok())
                .collect()
        } else {
            Vec::new()
        };
    let results = lorehaven_domain::events::evaluate_constraints(&constraints, facts);
    let results_json = serde_json::to_string(&results)?;
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO challenge_entries (challenge_id, work_id, entered_at, constraint_check) VALUES (?, ?, ?, ?)",
        "INSERT INTO challenge_entries (challenge_id, work_id, entered_at, constraint_check) VALUES ($1, $2, $3, $4)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(challenge_id)
                .bind(work_id)
                .bind(&now)
                .bind(&results_json)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(challenge_id)
                .bind(work_id)
                .bind(&now)
                .bind(&results_json)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// Requests / Exchanges
// ---------------------------------------------------------------------------

/// A request row.
#[derive(Debug, Clone, Serialize)]
pub struct Request {
    pub id: String,
    pub requester: String,
    pub prompt: String,
    pub anonym_until: Option<String>,
    pub created_at: String,
}

/// SQLx row for requests.
#[derive(Debug, sqlx::FromRow)]
struct RequestRow {
    id: String,
    requester: String,
    prompt: String,
    anonym_until: Option<String>,
    created_at: String,
}

impl From<RequestRow> for Request {
    fn from(r: RequestRow) -> Self {
        Request {
            id: r.id,
            requester: r.requester,
            prompt: r.prompt,
            anonym_until: r.anonym_until,
            created_at: r.created_at,
        }
    }
}

/// Create a request. Returns the new id.
pub async fn create_request(
    db: &Database,
    requester: &str,
    prompt: &str,
    anonym_until: Option<&str>,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO requests (id, requester, prompt, anonym_until, created_at) VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO requests (id, requester, prompt, anonym_until, created_at) VALUES ($1, $2, $3, $4, $5)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(requester)
                .bind(prompt)
                .bind(anonym_until)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(requester)
                .bind(prompt)
                .bind(anonym_until)
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(id)
}

/// Claim a request. Enforces "at most one active claim" in the write (spec
/// §18.3, §7.10 pitfall). Returns the number of rows affected — 1 means the
/// claim succeeded, 0 means there was already an active claim.
pub async fn claim_request(db: &Database, request_id: &str, claimant: &str) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    // The "at most one active claim" guard keys on `status`, not on
    // `fulfilled_at IS NULL` (M18-05). Under the old predicate an *expired*
    // claim still satisfied "there is already an active claim", so a request
    // whose claim had aged out could never be re-claimed — the expiry would
    // have released the work for nobody.
    // The guard blocks on a claim that is still live, which is exactly what the
    // old `fulfilled_at IS NULL` meant, restated as a state (M18-05).
    //
    // Two things that guard must *not* block on, and the reason each is easy to
    // get wrong:
    //
    // - A **fulfilled** claim. The request was answered; a new claimant may
    //   reasonably pick it up. The pre-existing test
    //   `a_fulfilled_request_can_be_claimed_again` pins this, and reading it as
    //   a bug rather than a decision is the mistake this comment exists to stop.
    //   An earlier draft of this fix guarded on `status <> 'expired'` instead,
    //   which reads as the conservative choice and quietly forbids a
    //   re-claim the spec permits.
    //
    // - An **expired** claim. Before the state existed, an expired claim was
    //   indistinguishable from an outstanding one, so the old predicate blocked
    //   it — a request whose claim aged out could never be picked up again, and
    //   the expiry released the work for nobody. `status = 'outstanding'` is the
    //   predicate that fixes that without forbidding the re-claim above.
    // `ON CONFLICT ... DO UPDATE` is load-bearing, not defensive. The key is
    // `(request_id, claimant)`, so a *new* claimant inserting is fine, but the
    // original claimant re-claiming their own expired claim collides with their
    // own earlier row. Under the pre-fix schema that could not arise — the guard
    // refused the re-claim, so the row was never duplicated — and once expiry
    // became a real state the collision is reachable. Refusing it would mean the
    // claimant who held the claim first is the one who can never take it back,
    // which is a worse rule than either of the two it replaces.
    //
    // The conflict arm re-arms the row rather than inserting a second one: a new
    // `claimed_at`, a fresh outstanding status, and the fulfilment columns
    // cleared, which is what "claiming again" means. The `WHERE` on the arm keeps
    // a *fulfilled* row from being reset — `a_fulfilled_request_can_be_claimed_
    // again` is true because a *different* claimant gets a *different* row, and
    // this must not let a fulfilled claim be reopened in place.
    let sql = db.sql(
        "INSERT INTO claims (request_id, claimant, claimed_at, fulfilled_by_work, fulfilled_at, status)
         SELECT ?, ?, ?, NULL, NULL, 'outstanding'
         WHERE NOT EXISTS (
             SELECT 1 FROM claims WHERE request_id = ? AND status = 'outstanding'
         )
         ON CONFLICT (request_id, claimant) DO UPDATE SET
             claimed_at = excluded.claimed_at,
             fulfilled_by_work = NULL,
             fulfilled_at = NULL,
             status = 'outstanding'
         WHERE claims.status = 'expired'",
        "INSERT INTO claims (request_id, claimant, claimed_at, fulfilled_by_work, fulfilled_at, status)
         SELECT $1, $2, $3, NULL, NULL, 'outstanding'
         WHERE NOT EXISTS (
             SELECT 1 FROM claims WHERE request_id = $4 AND status = 'outstanding'
         )
         ON CONFLICT (request_id, claimant) DO UPDATE SET
             claimed_at = excluded.claimed_at,
             fulfilled_by_work = NULL,
             fulfilled_at = NULL,
             status = 'outstanding'
         WHERE claims.status = 'expired'",
    );
    let rows = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(request_id)
            .bind(claimant)
            .bind(&now)
            .bind(request_id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(request_id)
            .bind(claimant)
            .bind(&now)
            .bind(request_id)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(rows > 0)
}

/// Fulfil a claim with a work. Returns true if the claim was found and
/// updated, false if there was no active claim for this claimant.
pub async fn fulfil_claim(
    db: &Database,
    request_id: &str,
    claimant: &str,
    work_id: &str,
) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    // Anti-gaming: one work may fulfil at most one claim per claimant. The
    // check below is on `(fulfilled_by_work, claimant)`, which is what the
    // comment used to claim the opposite of: the same work can fulfil a second
    // *request* from the same claimant only if a different claimant claimed it
    // too, so the rule is per (work, claimant) and not per work.
    //
    // `false` is the single "no" answer here, for all three of: no claim
    // matched, the claim was already fulfilled, or this work already fulfilled
    // another claim from this claimant. Collapsing them is what M18-06 asks
    // for, and it is safe because none of the three is actionable for the
    // caller differently: the claim did not take, and `false` says so.
    // `CAST(1 AS BIGINT)` rather than a bare `1`. `SELECT 1` is INT4 on
    // PostgreSQL and an integer on SQLite, and this row is read into an `i64`,
    // so the PostgreSQL arm fails to decode — invisibly on SQLite, where the
    // value happens to fit. `CAST` is ANSI, so one shared string covers both
    // dialects and `excluded` (below) is the only PostgreSQL-specific token.
    let dup_sql = db.sql(
        "SELECT CAST(1 AS BIGINT) FROM claims WHERE fulfilled_by_work = ? AND claimant = ? AND fulfilled_by_work IS NOT NULL",
        "SELECT CAST(1 AS BIGINT) FROM claims WHERE fulfilled_by_work = $1 AND claimant = $2 AND fulfilled_by_work IS NOT NULL",
    );
    let dup: Option<(i64,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&dup_sql)
                .bind(work_id)
                .bind(claimant)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&dup_sql)
                .bind(work_id)
                .bind(claimant)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    // A duplicate is a soft `false`, matching `join_event` (M18-06). Both
    // functions are `Result<bool>` and both now answer `Ok(true)` for a change
    // and `Ok(false)` for "already there", so a caller needs no per-function
    // knowledge of which duplicates are errors. The alternative — a distinct
    // error per function — is what made this a trap: a caller could follow both
    // signatures correctly and still be wrong on one of them.
    //
    // The anti-gaming rule is unchanged in force; only its reporting moved. A
    // second claim from the same claimant on the same work is still refused, and
    // `false` is the refusal.
    if dup.is_some() {
        return Ok(false);
    }
    let update_sql = db.sql(
        "UPDATE claims SET fulfilled_by_work = ?, fulfilled_at = ?, status = 'fulfilled' WHERE request_id = ? AND claimant = ? AND status = 'outstanding'",
        "UPDATE claims SET fulfilled_by_work = $1, fulfilled_at = $2, status = 'fulfilled' WHERE request_id = $3 AND claimant = $4 AND status = 'outstanding'",
    );
    let rows = match db.backend() {
        Backend::Sqlite => sqlx::query(&update_sql)
            .bind(work_id)
            .bind(&now)
            .bind(request_id)
            .bind(claimant)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&update_sql)
            .bind(work_id)
            .bind(&now)
            .bind(request_id)
            .bind(claimant)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(rows > 0)
}

/// Expire claims that have been outstanding past their request's anonymity
/// window or a fixed deadline. Called by the job queue.
pub async fn expire_claims(db: &Database, now: &str, grace_seconds: i64) -> Result<i64> {
    // Claims whose request has no fulfillment and whose claimed_at is older
    // than the grace period are expired. The statement is written per
    // dialect because the timestamp arithmetic differs; RFC 3339 text
    // compares lexically, so the bound is computed in Rust.
    // now is the request's clock reading; go back `grace_seconds` from it.
    let cutoff = crate::identity::format_rfc3339(
        time::OffsetDateTime::parse(now, &time::format_description::well_known::Rfc3339)
            .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
            .checked_sub(time::Duration::seconds(grace_seconds.max(0)))
            .unwrap_or(time::OffsetDateTime::UNIX_EPOCH),
    );
    // The predicate keys on `status = 'outstanding'`, not on `fulfilled_at IS
    // NULL` (M18-05). That single change is the whole fix: expiring a claim now
    // sets `status = 'expired'`, which the same statement does not match, so a
    // claim is expired exactly once no matter how often the scheduled job runs.
    //
    // The old statement wrote `fulfilled_at = NULL` — the very expression it
    // filtered on — so the row came out of the statement looking exactly like it
    // had going in. The count it returned was every claim past the cutoff rather
    // than the work this run did, and for a request nobody ever fulfilled it grew
    // on every pass instead of falling to zero.
    //
    // `fulfilled_by_work`/`fulfilled_at` are cleared too. They are already NULL
    // on any row with `status = 'outstanding'` (the status is the discriminator,
    // and nothing writes a work without setting the status), so this is
    // belt-and-braces against a half-written row rather than the mechanism.
    let sql = db.sql(
        "UPDATE claims SET status = 'expired', fulfilled_by_work = NULL, fulfilled_at = NULL
         WHERE status = 'outstanding' AND claimed_at < ?",
        "UPDATE claims SET status = 'expired', fulfilled_by_work = NULL, fulfilled_at = NULL
         WHERE status = 'outstanding' AND claimed_at < $1",
    );
    let expired: i64 = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&cutoff)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected() as i64,
        Backend::Postgres => sqlx::query(&sql)
            .bind(&cutoff)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected() as i64,
    };
    Ok(expired)
}

// ---------------------------------------------------------------------------
// Wishlists
// ---------------------------------------------------------------------------

/// Upsert a wishlist row for an account.
pub async fn upsert_wishlist(db: &Database, account: &str, is_public: bool) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    let is_public_int = i64::from(is_public);
    let sql = db.sql(
        "INSERT INTO wishlists (account, is_public) VALUES (?, ?)
         ON CONFLICT(account) DO UPDATE SET is_public = excluded.is_public",
        "INSERT INTO wishlists (account, is_public) VALUES ($1, $2)
         ON CONFLICT(account) DO UPDATE SET is_public = excluded.is_public",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(account)
                .bind(is_public_int)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(account)
                .bind(is_public_int)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    let _ = now;
    Ok(())
}

/// Fetch a wishlist and its items, honoring privacy (spec §18.4).
pub async fn get_wishlist(
    db: &Database,
    account: &str,
    viewer: Option<&str>,
) -> Result<Option<Wishlist>> {
    let wl_sql = db.sql(
        "SELECT account, is_public FROM wishlists WHERE account = ?",
        "SELECT account, is_public FROM wishlists WHERE account = $1",
    );
    let wl_row: Option<WishlistRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&wl_sql)
                .bind(account)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&wl_sql)
                .bind(account)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    let wl = match wl_row {
        Some(w) => w,
        None => return Ok(None),
    };
    // Privacy check: private wishlist visible only to owner.
    let owner_id =
        lorehaven_domain::AccountId::from_uuid(uuid::Uuid::parse_str(account).unwrap_or_default());
    let viewer_id = viewer.map(|v| {
        lorehaven_domain::AccountId::from_uuid(uuid::Uuid::parse_str(v).unwrap_or_default())
    });
    let is_visible = lorehaven_domain::events::wishlist_visible(
        wl.is_public != 0,
        viewer_id.as_ref(),
        &owner_id,
    );
    if !is_visible {
        return Ok(None);
    }
    let items_sql = db.sql(
        "SELECT wishlist, node_id, work_id, note, added_at FROM wishlist_items WHERE wishlist = ?",
        "SELECT wishlist, node_id, work_id, note, added_at FROM wishlist_items WHERE wishlist = $1",
    );
    let item_rows: Vec<WishlistItemRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&items_sql)
                .bind(account)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&items_sql)
                .bind(account)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(Some(Wishlist {
        account: wl.account,
        is_public: wl.is_public != 0,
        items: item_rows.into_iter().map(WishlistItem::from).collect(),
    }))
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct WishlistRow {
    account: String,
    is_public: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Wishlist {
    pub account: String,
    pub is_public: bool,
    pub items: Vec<WishlistItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WishlistItem {
    pub wishlist: String,
    pub node_id: Option<String>,
    pub work_id: Option<String>,
    pub note: Option<String>,
    pub added_at: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct WishlistItemRow {
    wishlist: String,
    node_id: Option<String>,
    work_id: Option<String>,
    note: Option<String>,
    added_at: String,
}

impl From<WishlistItemRow> for WishlistItem {
    fn from(r: WishlistItemRow) -> Self {
        Self {
            wishlist: r.wishlist,
            node_id: r.node_id,
            work_id: r.work_id,
            note: r.note,
            added_at: r.added_at,
        }
    }
}

/// Add an item to a wishlist.
pub async fn add_wishlist_item(
    db: &Database,
    wishlist: &str,
    node_id: Option<&str>,
    work_id: Option<&str>,
    note: Option<&str>,
) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO wishlist_items (wishlist, node_id, work_id, note, added_at) VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO wishlist_items (wishlist, node_id, work_id, note, added_at) VALUES ($1, $2, $3, $4, $5)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(wishlist)
                .bind(node_id)
                .bind(work_id)
                .bind(note)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(wishlist)
                .bind(node_id)
                .bind(work_id)
                .bind(note)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// An event row.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub id: String,
    pub name: String,
    pub document: String,
    pub created_by: String,
    pub created_at: String,
}

/// Create an event. Returns the new id.
pub async fn create_event(
    db: &Database,
    name: &str,
    document: &str,
    created_by: &str,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO events (id, name, document, created_by, created_at) VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO events (id, name, document, created_by, created_at) VALUES ($1, $2, $3, $4, $5)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(name)
                .bind(document)
                .bind(created_by)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(name)
                .bind(document)
                .bind(created_by)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(id)
}

/// Fetch an event by id.
pub async fn get_event(db: &Database, event_id: &str) -> Result<Option<Event>> {
    let sql = db.sql(
        "SELECT id, name, document, created_by, created_at FROM events WHERE id = ?",
        "SELECT id, name, document, created_by, created_at FROM events WHERE id = $1",
    );
    let row: Option<EventRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(event_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(event_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.map(Event::from))
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct EventRow {
    id: String,
    name: String,
    document: String,
    created_by: String,
    created_at: String,
}

impl From<EventRow> for Event {
    fn from(r: EventRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            document: r.document,
            created_by: r.created_by,
            created_at: r.created_at,
        }
    }
}

/// Join an event (insert a participation row). Returns false if already joined.
///
/// A duplicate is reported as `false`, not raised (M18-06). `fulfil_claim` and
/// this function are the two places where "you already did this" is a normal
/// outcome rather than a fault, and they now say so the same way: `Ok(true)` for
/// the change, `Ok(false)` for "already there".
///
/// Previously the duplicate reached the caller as a database error. The
/// signature promised otherwise — `Result<bool>`, with a doc comment saying
/// "returns false if already joined" — and the comment was simply wrong, because
/// the statement was a bare `INSERT` against a composite primary key and the
/// unique violation propagated out of `?`. A caller following the signature
/// would have had to wrap every call in a constraint-violation matcher to get
/// the documented behaviour.
///
/// `ON CONFLICT DO NOTHING` is used rather than catching the error because the
/// duplicate is a *known* case with a *known* answer, and handling it in the
/// statement keeps it out of the error path entirely. It is also the only form
/// that behaves identically on both dialects without inspecting a driver error
/// code — SQLite and PostgreSQL number their unique violations differently.
pub async fn join_event(db: &Database, event_id: &str, account: &str) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO event_participation (event_id, account, joined_at) VALUES (?, ?, ?)
         ON CONFLICT (event_id, account) DO NOTHING",
        "INSERT INTO event_participation (event_id, account, joined_at) VALUES ($1, $2, $3)
         ON CONFLICT (event_id, account) DO NOTHING",
    );
    let rows = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(event_id)
            .bind(account)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(event_id)
            .bind(account)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(rows > 0)
}

/// List an event's participants.
pub async fn list_event_participants(db: &Database, event_id: &str) -> Result<Vec<String>> {
    let sql = db.sql(
        "SELECT account FROM event_participation WHERE event_id = ? ORDER BY joined_at ASC",
        "SELECT account FROM event_participation WHERE event_id = $1 ORDER BY joined_at ASC",
    );
    let rows: Vec<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(event_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(event_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows.into_iter().map(|r| r.0).collect())
}

// ---------------------------------------------------------------------------
// List helpers (used by routes)
// ---------------------------------------------------------------------------

/// List public collections, newest first.
pub async fn list_public_collections(db: &Database, limit: i64) -> Result<Vec<Collection>> {
    let sql = db.sql(
        "SELECT id, name, description, owner, item_policy, is_public, created_at FROM collections WHERE is_public = 1 ORDER BY created_at DESC LIMIT ?",
        "SELECT id, name, description, owner, item_policy, is_public, created_at FROM collections WHERE is_public = 1 ORDER BY created_at DESC LIMIT $1",
    );
    let rows: Vec<CollectionRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows.into_iter().map(Collection::from).collect())
}

/// Update a collection's metadata (owner-check in the WHERE clause).
pub async fn update_collection(
    db: &Database,
    collection_id: &str,
    owner: &str,
    name: &str,
    description: Option<&str>,
    item_policy: &str,
    is_public: bool,
) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let is_public_int = i64::from(is_public);
    let _ = now;
    let sql = db.sql(
        "UPDATE collections SET name = ?, description = ?, item_policy = ?, is_public = ? WHERE id = ? AND owner = ?",
        "UPDATE collections SET name = $1, description = $2, item_policy = $3, is_public = $4 WHERE id = $5 AND owner = $6",
    );
    let rows = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(name)
            .bind(description)
            .bind(item_policy)
            .bind(is_public_int)
            .bind(collection_id)
            .bind(owner)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(name)
            .bind(description)
            .bind(item_policy)
            .bind(is_public_int)
            .bind(collection_id)
            .bind(owner)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(rows > 0)
}

/// List challenges, newest first.
pub async fn list_challenges(db: &Database, limit: i64) -> Result<Vec<Challenge>> {
    let sql = db.sql(
        "SELECT id, name, rules, schedule, created_by, created_at FROM challenges ORDER BY created_at DESC LIMIT ?",
        "SELECT id, name, rules, schedule, created_by, created_at FROM challenges ORDER BY created_at DESC LIMIT $1",
    );
    let rows: Vec<ChallengeRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows.into_iter().map(Challenge::from).collect())
}

/// List requests, newest first.
pub async fn list_requests(db: &Database, limit: i64) -> Result<Vec<Request>> {
    let sql = db.sql(
        "SELECT id, requester, prompt, anonym_until, created_at FROM requests ORDER BY created_at DESC LIMIT ?",
        "SELECT id, requester, prompt, anonym_until, created_at FROM requests ORDER BY created_at DESC LIMIT $1",
    );
    let rows: Vec<RequestRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows.into_iter().map(Request::from).collect())
}

/// List events, newest first.
pub async fn list_events(db: &Database, limit: i64) -> Result<Vec<Event>> {
    let sql = db.sql(
        "SELECT id, name, document, created_by, created_at FROM events ORDER BY created_at DESC LIMIT ?",
        "SELECT id, name, document, created_by, created_at FROM events ORDER BY created_at DESC LIMIT $1",
    );
    let rows: Vec<EventRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows.into_iter().map(Event::from).collect())
}

/// Total word count for a work (the sum of current chapter revisions).
pub async fn work_word_count(db: &Database, work_id: &str) -> Result<i64> {
    let sql = db.sql(
        "SELECT COALESCE(CAST(SUM(r.word_count) AS BIGINT), 0) FROM chapters c JOIN chapter_revisions r ON r.id = c.current_revision_id WHERE c.work_id = ? AND c.deleted_at IS NULL",
        "SELECT COALESCE(SUM(r.word_count), 0)::bigint FROM chapters c JOIN chapter_revisions r ON r.id = c.current_revision_id WHERE c.work_id = $1::uuid AND c.deleted_at IS NULL",
    );
    let n: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(n)
}
