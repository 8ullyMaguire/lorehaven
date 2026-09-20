//! Typed-vote storage (spec §35.2, repo M32): the taxonomy, casts, meta-mod
//! verdicts, vote budgets and karma.
//!
//! Both dialects are written out, per house rule §2.1; every bind is `String`
//! or `i64` (ADR 0004) — weights and karma travel as basis points, timestamps
//! as RFC 3339 text, and booleans as 0/1 integers.
//!
//! Two things this module deliberately does *not* do:
//!
//! * it never enforces a rule (budget, weight, transparency) — those are
//!   [`lorehaven_domain::typed_votes`] functions, so a route can report a
//!   refusal as a validation failure rather than an opaque database error;
//! * nothing outside this module and its display route reads `forum_karma`
//!   (spec §35.2: karma never gates trust, moderation, ranking or credits).
//!   `milestone_32.rs` greps the workspace to keep that true.

use anyhow::Result;
use lorehaven_domain::typed_votes::{
    decay_karma_bp, effective_taxonomy, months_inactive, vote_weight_bp, VoteType,
};
use sqlx::FromRow;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::identity::format_rfc3339;
use crate::{Backend, Database};

/// Every taxonomy row, instance default and category-scoped alike.
pub async fn all_vote_types(db: &Database) -> Result<Vec<VoteType>> {
    let sql = db.sql(
        "SELECT id, label, category_scope, position, weight_bp, cost, is_negative FROM forum_vote_types ORDER BY position, id",
        "SELECT id, label, category_scope, position, weight_bp, cost, is_negative FROM forum_vote_types ORDER BY position, id",
    );
    let rows: Vec<VoteTypeRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, VoteTypeRow>(&sql)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, VoteTypeRow>(&sql)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows.into_iter().map(VoteType::from).collect())
}

/// The taxonomy a category's surface offers: its own rows, else the default
/// set (spec §35.2).
pub async fn vote_types_for_category(db: &Database, category_id: &str) -> Result<Vec<VoteType>> {
    let all = all_vote_types(db).await?;
    Ok(effective_taxonomy(&all, category_id))
}

/// One taxonomy row by its id (type ids are globally unique).
pub async fn vote_type(db: &Database, id: &str) -> Result<Option<VoteType>> {
    let sql = db.sql(
        "SELECT id, label, category_scope, position, weight_bp, cost, is_negative FROM forum_vote_types WHERE id = ?",
        "SELECT id, label, category_scope, position, weight_bp, cost, is_negative FROM forum_vote_types WHERE id = $1",
    );
    let row: Option<VoteTypeRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, VoteTypeRow>(&sql)
                .bind(id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, VoteTypeRow>(&sql)
                .bind(id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.map(VoteType::from))
}

#[derive(Debug, Clone, FromRow)]
struct VoteTypeRow {
    id: String,
    label: String,
    category_scope: Option<String>,
    position: i64,
    weight_bp: i64,
    cost: i64,
    is_negative: i64,
}

impl From<VoteTypeRow> for VoteType {
    fn from(row: VoteTypeRow) -> Self {
        Self {
            id: row.id,
            label: row.label,
            category_scope: row.category_scope,
            position: row.position,
            weight_bp: row.weight_bp,
            cost: row.cost,
            is_negative: row.is_negative != 0,
        }
    }
}

/// What a post's vote surface needs to know about the post itself.
#[derive(Debug, Clone, FromRow)]
pub struct PostContext {
    /// The post being voted on.
    pub post_id: String,
    /// Its topic.
    pub topic_id: String,
    /// The pseud whose karma the votes accrue to.
    pub author_pseud: String,
    /// The topic's category, which selects the taxonomy.
    pub category_id: String,
    /// Whether the author has opted in to revealing who voted.
    pub votes_visible: i64,
}

/// The post a vote attaches to, with the category its taxonomy comes from.
pub async fn post_context(db: &Database, post_id: &str) -> Result<Option<PostContext>> {
    let sql = db.sql(
        "SELECT p.id AS post_id, p.topic_id, p.author_pseud, t.category_id, p.votes_visible \
         FROM forum_posts p JOIN forum_topics t ON t.id = p.topic_id \
         WHERE p.id = ? AND p.deleted_at IS NULL",
        "SELECT p.id AS post_id, p.topic_id, p.author_pseud, t.category_id, p.votes_visible \
         FROM forum_posts p JOIN forum_topics t ON t.id = p.topic_id \
         WHERE p.id = $1 AND p.deleted_at IS NULL",
    );
    let row: Option<PostContext> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, PostContext>(&sql)
                .bind(post_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, PostContext>(&sql)
                .bind(post_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row)
}

/// A stored vote.
#[derive(Debug, Clone, FromRow)]
pub struct StoredVote {
    /// Surrogate id — the handle `POST /forum/votes/{id}/meta` names.
    pub id: String,
    /// The post voted on.
    pub post_id: String,
    /// The pseud that cast it.
    pub pseud: String,
    /// The vote type key.
    pub vote_type: String,
    /// The caster's weight when the vote was cast, in basis points. Frozen:
    /// later meta-moderation moves *future* weight, never history.
    pub weight_at_cast_bp: i64,
    /// When the vote was cast (or last changed).
    pub created_at: String,
}

/// One pseud's vote on one post.
pub async fn vote_for(db: &Database, post_id: &str, pseud: &str) -> Result<Option<StoredVote>> {
    let sql = db.sql(
        "SELECT id, post_id, pseud, vote_type, weight_at_cast_bp, created_at \
         FROM forum_votes WHERE post_id = ? AND pseud = ?",
        "SELECT id, post_id, pseud, vote_type, weight_at_cast_bp, created_at \
         FROM forum_votes WHERE post_id = $1 AND pseud = $2",
    );
    let row: Option<StoredVote> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, StoredVote>(&sql)
                .bind(post_id)
                .bind(pseud)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, StoredVote>(&sql)
                .bind(post_id)
                .bind(pseud)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row)
}

/// A vote by its surrogate id.
pub async fn vote_by_id(db: &Database, vote_id: &str) -> Result<Option<StoredVote>> {
    let sql = db.sql(
        "SELECT id, post_id, pseud, vote_type, weight_at_cast_bp, created_at \
         FROM forum_votes WHERE id = ?",
        "SELECT id, post_id, pseud, vote_type, weight_at_cast_bp, created_at \
         FROM forum_votes WHERE id = $1",
    );
    let row: Option<StoredVote> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, StoredVote>(&sql)
                .bind(vote_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, StoredVote>(&sql)
                .bind(vote_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row)
}

/// Record (or change) one pseud's vote on one post.
///
/// A change is an upsert: one row per (post, pseud) is the invariant the unique
/// index enforces. `created_at` moves with the change, because the rolling
/// budget window charges when the vote was last *cast*.
pub async fn upsert_vote(
    db: &Database,
    post_id: &str,
    pseud: &str,
    vote_type: &str,
    weight_at_cast_bp: i64,
    now: &str,
) -> Result<()> {
    let sql = db.sql(
        "INSERT INTO forum_votes (id, post_id, pseud, vote_type, weight_at_cast_bp, created_at) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(post_id, pseud) DO UPDATE SET vote_type = excluded.vote_type, \
             weight_at_cast_bp = excluded.weight_at_cast_bp, created_at = excluded.created_at",
        "INSERT INTO forum_votes (id, post_id, pseud, vote_type, weight_at_cast_bp, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (post_id, pseud) DO UPDATE SET vote_type = excluded.vote_type, \
             weight_at_cast_bp = excluded.weight_at_cast_bp, created_at = excluded.created_at",
    );
    let id = uuid::Uuid::new_v4().to_string();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(post_id)
                .bind(pseud)
                .bind(vote_type)
                .bind(weight_at_cast_bp)
                .bind(now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(post_id)
                .bind(pseud)
                .bind(vote_type)
                .bind(weight_at_cast_bp)
                .bind(now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Remove a vote, returning the row that was removed (the caller needs its
/// weight to reverse the karma it contributed).
pub async fn delete_vote(db: &Database, post_id: &str, pseud: &str) -> Result<Option<StoredVote>> {
    let removed = vote_for(db, post_id, pseud).await?;
    if removed.is_none() {
        return Ok(None);
    }
    let sql = db.sql(
        "DELETE FROM forum_votes WHERE post_id = ? AND pseud = ?",
        "DELETE FROM forum_votes WHERE post_id = $1 AND pseud = $2",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(post_id)
                .bind(pseud)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(post_id)
                .bind(pseud)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(removed)
}

/// Aggregate counts per vote type on a post, ordered by type key.
pub async fn vote_counts(db: &Database, post_id: &str) -> Result<Vec<(String, i64)>> {
    let sql = db.sql(
        "SELECT vote_type, COUNT(*) FROM forum_votes WHERE post_id = ? GROUP BY vote_type ORDER BY vote_type",
        "SELECT vote_type, COUNT(*) FROM forum_votes WHERE post_id = $1 GROUP BY vote_type ORDER BY vote_type",
    );
    let rows: Vec<(String, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(post_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(post_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

/// One individual vote, for the transparency tiers that may see them.
#[derive(Debug, Clone, FromRow)]
pub struct IndividualVote {
    /// The vote's id — what `POST /forum/votes/{id}/meta` names, and why a
    /// moderator's view has to carry it.
    pub id: String,
    /// Who voted.
    pub pseud: String,
    /// What they chose.
    pub vote_type: String,
    /// When.
    pub created_at: String,
}

/// The votes on a post with their casters. Only call this once the
/// transparency tier says the viewer may see them.
pub async fn votes_on_post(db: &Database, post_id: &str) -> Result<Vec<IndividualVote>> {
    let sql = db.sql(
        "SELECT id, pseud, vote_type, created_at FROM forum_votes WHERE post_id = ? ORDER BY created_at",
        "SELECT id, pseud, vote_type, created_at FROM forum_votes WHERE post_id = $1 ORDER BY created_at",
    );
    let rows: Vec<IndividualVote> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, IndividualVote>(&sql)
                .bind(post_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, IndividualVote>(&sql)
                .bind(post_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

/// The weighted total of a post's votes: every vote at the weight its caster
/// held when they cast it.
pub async fn weighted_total_bp(db: &Database, post_id: &str) -> Result<i64> {
    let sql = db.sql(
        "SELECT COALESCE(SUM(weight_at_cast_bp), 0) FROM forum_votes WHERE post_id = ?",
        "SELECT COALESCE(SUM(weight_at_cast_bp), 0) FROM forum_votes WHERE post_id = $1",
    );
    let total: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(post_id)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(post_id)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(total)
}

/// Set the post author's opt-in to revealing who voted.
pub async fn set_votes_visible(db: &Database, post_id: &str, visible: bool) -> Result<bool> {
    let sql = db.sql(
        "UPDATE forum_posts SET votes_visible = ? WHERE id = ? AND deleted_at IS NULL",
        "UPDATE forum_posts SET votes_visible = $1 WHERE id = $2 AND deleted_at IS NULL",
    );
    let flag = if visible { 1_i64 } else { 0_i64 };
    let affected = match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(flag)
                .bind(post_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?
                .rows_affected()
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(flag)
                .bind(post_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?
                .rows_affected()
        }
    };
    Ok(affected > 0)
}

// ---------------------------------------------------------------------------
// Budget
// ---------------------------------------------------------------------------

/// Budget charged by the votes an account's pseuds cast inside the window.
///
/// The budget is per **account**, not per pseud: a second pseud must not buy a
/// second allowance, or the vote budget is a suggestion. Trust is an account
/// property already (§19), so the two line up.
pub async fn budget_spent(db: &Database, account_id: &str, window_start: &str) -> Result<i64> {
    let sql = db.sql(
        "SELECT COALESCE(SUM(t.cost), 0) FROM forum_votes v \
         JOIN pseuds p ON p.id = v.pseud \
         JOIN forum_vote_types t ON t.id = v.vote_type \
         WHERE p.account_id = ? AND v.created_at > ?",
        "SELECT COALESCE(SUM(t.cost), 0) FROM forum_votes v \
         JOIN pseuds p ON p.id = v.pseud \
         JOIN forum_vote_types t ON t.id = v.vote_type \
         WHERE p.account_id = $1 AND v.created_at > $2",
    );
    let total: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(account_id)
                .bind(window_start)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(account_id)
                .bind(window_start)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(total)
}

/// The oldest charge still inside the window — when votes start coming back.
/// `None` when the window holds no votes.
pub async fn oldest_charge(
    db: &Database,
    account_id: &str,
    window_start: &str,
) -> Result<Option<String>> {
    let sql = db.sql(
        "SELECT MIN(v.created_at) FROM forum_votes v \
         JOIN pseuds p ON p.id = v.pseud WHERE p.account_id = ? AND v.created_at > ?",
        "SELECT MIN(v.created_at) FROM forum_votes v \
         JOIN pseuds p ON p.id = v.pseud WHERE p.account_id = $1 AND v.created_at > $2",
    );
    let oldest: Option<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(account_id)
                .bind(window_start)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(account_id)
                .bind(window_start)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(oldest)
}

/// Meta-mod points an account spent inside the window.
pub async fn meta_mods_cast(db: &Database, account_id: &str, window_start: &str) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM forum_meta_votes m \
         JOIN pseuds p ON p.id = m.pseud WHERE p.account_id = ? AND m.created_at > ?",
        "SELECT COUNT(*) FROM forum_meta_votes m \
         JOIN pseuds p ON p.id = m.pseud WHERE p.account_id = $1 AND m.created_at > $2",
    );
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(account_id)
                .bind(window_start)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(account_id)
                .bind(window_start)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(count)
}

// ---------------------------------------------------------------------------
// Meta-moderation
// ---------------------------------------------------------------------------

/// A verdict a steward left on someone's vote.
#[derive(Debug, Clone, FromRow)]
pub struct MetaAction {
    /// The steward.
    pub pseud: String,
    /// `true` for fair, `false` for unfair.
    pub fair: i64,
    /// When.
    pub created_at: String,
}

/// A caster's meta-moderation record: `(fair, unfair)` verdict counts.
pub async fn meta_verdicts(db: &Database, caster_pseud: &str) -> Result<(i64, i64)> {
    let sql = db.sql(
        "SELECT COALESCE(SUM(CASE WHEN m.fair = 1 THEN 1 ELSE 0 END), 0), \
                COALESCE(SUM(CASE WHEN m.fair = 0 THEN 1 ELSE 0 END), 0) \
         FROM forum_meta_votes m JOIN forum_votes v ON v.id = m.vote_id WHERE v.pseud = ?",
        "SELECT COALESCE(SUM(CASE WHEN m.fair = 1 THEN 1 ELSE 0 END), 0), \
                COALESCE(SUM(CASE WHEN m.fair = 0 THEN 1 ELSE 0 END), 0) \
         FROM forum_meta_votes m JOIN forum_votes v ON v.id = m.vote_id WHERE v.pseud = $1",
    );
    let row: (i64, i64) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(caster_pseud)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(caster_pseud)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row)
}

/// The caster's current vote weight, in basis points (spec §35.2).
///
/// The weight a *future* vote is stamped with. Meta-moderation never touches a
/// stored vote: history keeps the weight it was cast at, so a caster's past
/// contribution to other people's karma is not rewritten by a later flag.
pub async fn caster_weight_bp(
    db: &Database,
    caster_pseud: &str,
    min_weight_bp: i64,
    min_verdicts: i64,
) -> Result<i64> {
    let (fair, unfair) = meta_verdicts(db, caster_pseud).await?;
    Ok(vote_weight_bp(fair, unfair, min_weight_bp, min_verdicts))
}

/// A steward's existing verdict on a vote, if any.
pub async fn meta_vote_for(db: &Database, vote_id: &str, pseud: &str) -> Result<Option<bool>> {
    let sql = db.sql(
        "SELECT fair FROM forum_meta_votes WHERE vote_id = ? AND pseud = ?",
        "SELECT fair FROM forum_meta_votes WHERE vote_id = $1 AND pseud = $2",
    );
    let fair: Option<i64> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(vote_id)
                .bind(pseud)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(vote_id)
                .bind(pseud)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(fair.map(|value| value != 0))
}

/// Record a steward's verdict on a vote (one per steward per vote).
pub async fn upsert_meta_vote(
    db: &Database,
    vote_id: &str,
    pseud: &str,
    fair: bool,
    now: &str,
) -> Result<()> {
    let sql = db.sql(
        "INSERT INTO forum_meta_votes (vote_id, pseud, fair, created_at) VALUES (?, ?, ?, ?) \
         ON CONFLICT(vote_id, pseud) DO UPDATE SET fair = excluded.fair, created_at = excluded.created_at",
        "INSERT INTO forum_meta_votes (vote_id, pseud, fair, created_at) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (vote_id, pseud) DO UPDATE SET fair = excluded.fair, created_at = excluded.created_at",
    );
    let flag = if fair { 1_i64 } else { 0_i64 };
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(vote_id)
                .bind(pseud)
                .bind(flag)
                .bind(now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(vote_id)
                .bind(pseud)
                .bind(flag)
                .bind(now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Every verdict left on a vote — what a meta-mod is allowed to see.
pub async fn meta_actions_on_vote(db: &Database, vote_id: &str) -> Result<Vec<MetaAction>> {
    let sql = db.sql(
        "SELECT pseud, fair, created_at FROM forum_meta_votes WHERE vote_id = ? ORDER BY created_at",
        "SELECT pseud, fair, created_at FROM forum_meta_votes WHERE vote_id = $1 ORDER BY created_at",
    );
    let rows: Vec<MetaAction> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, MetaAction>(&sql)
                .bind(vote_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, MetaAction>(&sql)
                .bind(vote_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}


// ---------------------------------------------------------------------------
// Karma (display only — spec §35.2, and §0.3 before it)
// ---------------------------------------------------------------------------

/// A pseud's karma and what produced it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct KarmaSummary {
    /// Whose karma this is.
    pub pseud: String,
    /// The stored value, in basis points, after the last decay was applied.
    pub karma_bp: i64,
    /// When karma last changed (a vote arrived, or decay was applied).
    pub updated_at: String,
    /// How many votes this pseud's posts have received.
    pub votes_received: i64,
    /// What those votes totalled at cast weight, in basis points.
    pub weighted_received_bp: i64,
}

/// Read the stored karma row.
async fn read_karma(db: &Database, pseud: &str) -> Result<Option<(i64, String)>> {
    let sql = db.sql(
        "SELECT karma_bp, updated_at FROM forum_karma WHERE pseud = ?",
        "SELECT karma_bp, updated_at FROM forum_karma WHERE pseud = $1",
    );
    let row: Option<(i64, String)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(pseud)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(pseud)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row)
}

/// Write the karma row.
async fn write_karma(db: &Database, pseud: &str, karma_bp: i64, now: OffsetDateTime) -> Result<()> {
    let sql = db.sql(
        "INSERT INTO forum_karma (pseud, karma_bp, updated_at) VALUES (?, ?, ?) \
         ON CONFLICT(pseud) DO UPDATE SET karma_bp = excluded.karma_bp, updated_at = excluded.updated_at",
        "INSERT INTO forum_karma (pseud, karma_bp, updated_at) VALUES ($1, $2, $3) \
         ON CONFLICT (pseud) DO UPDATE SET karma_bp = excluded.karma_bp, updated_at = excluded.updated_at",
    );
    let at = format_rfc3339(now);
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(pseud)
                .bind(karma_bp)
                .bind(&at)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(pseud)
                .bind(karma_bp)
                .bind(&at)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// A pseud's karma with the votes behind it.
///
/// The stored number is already decayed: accrual and the read-time decay pass
/// both write through [`write_karma`], so a dormant pseud's figure is never a
/// stale high-water mark.
pub async fn karma_summary(db: &Database, pseud: &str) -> Result<KarmaSummary> {
    let stored = read_karma(db, pseud).await?;
    let (karma_bp, updated_at) = stored.unwrap_or_else(|| (0, String::new()));

    let sql = db.sql(
        "SELECT COUNT(*), COALESCE(SUM(v.weight_at_cast_bp), 0) FROM forum_votes v \
         JOIN forum_posts p ON p.id = v.post_id \
         WHERE p.author_pseud = ? AND p.deleted_at IS NULL",
        "SELECT COUNT(*), COALESCE(SUM(v.weight_at_cast_bp), 0) FROM forum_votes v \
         JOIN forum_posts p ON p.id = v.post_id \
         WHERE p.author_pseud = $1 AND p.deleted_at IS NULL",
    );
    let (votes_received, weighted_received_bp): (i64, i64) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(pseud)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(pseud)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };

    Ok(KarmaSummary {
        pseud: pseud.to_owned(),
        karma_bp,
        updated_at,
        votes_received,
        weighted_received_bp,
    })
}

/// Add (or remove) a vote's weight from the receiver's karma.
///
/// Decay for the months since the last accrual is applied *before* the
/// addition, so receiving a vote never resurrects decayed karma.
pub async fn accrue_karma(
    db: &Database,
    pseud: &str,
    delta_bp: i64,
    now: OffsetDateTime,
    decay_percent: i64,
) -> Result<i64> {
    let stored = read_karma(db, pseud).await?;
    let current = stored.as_ref().map_or(0, |(karma, _)| *karma);
    let decayed = stored.as_ref().map_or(current, |(_, updated_at)| {
        OffsetDateTime::parse(updated_at, &Rfc3339)
            .map_or(current, |at| {
                decay_karma_bp(current, months_inactive(at, now), decay_percent)
            })
    });
    let next = (decayed + delta_bp).max(0);
    write_karma(db, pseud, next, now).await?;
    Ok(next)
}

/// Apply the monthly inactivity decay to a pseud's karma, and persist it.
///
/// Idempotent: it writes only when a whole 30-day month has passed since the
/// last karma change, and moves the anchor to `now` when it does. The karma
/// display route calls this so an inactive pseud's number is not a stale
/// high-water mark; a worker may sweep it on the same terms.
pub async fn decay_karma(
    db: &Database,
    pseud: &str,
    now: OffsetDateTime,
    decay_percent: i64,
) -> Result<i64> {
    let Some((current, updated_at)) = read_karma(db, pseud).await? else {
        return Ok(0);
    };
    let months =
        OffsetDateTime::parse(&updated_at, &Rfc3339).map_or(0, |at| months_inactive(at, now));
    if months <= 0 {
        return Ok(current);
    }
    let decayed = decay_karma_bp(current, months, decay_percent);
    write_karma(db, pseud, decayed, now).await?;
    Ok(decayed)
}
