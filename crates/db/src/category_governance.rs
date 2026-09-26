//! Directory Category Governance repository (spec §45). Both dialects.
//!
//! SQL is written SQLite-style (`?`) and rewritten for PostgreSQL by
//! [`Database::sql`], per crate convention.

use anyhow::Result;
use serde::Serialize;
use sqlx::{FromRow, Row};

use crate::{Backend, Database};
use lorehaven_domain::category_governance::*;

/// A directory category row (§45.1).
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Category {
    pub id: String,
    pub slug: String,
    pub label: String,
    pub state: String,
    pub merged_into: Option<String>,
    pub source: String,
    pub created_by: String,
    pub created_at: String,
}

/// A category proposal row (§45.2).
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct CategoryProposal {
    pub id: String,
    pub category_slug: String,
    pub action: String,
    pub payload: String,
    pub status: String,
    pub yes_votes: i64,
    pub no_votes: i64,
    pub quorum_needed: i64,
    pub closes_at: String,
    pub created_by: String,
    pub decided_by: Option<String>,
    pub decision_reason: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
}

/// A changelog entry (§45.5).
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct CategoryChangelog {
    pub id: String,
    pub category_slug: String,
    pub event: String,
    pub actor: String,
    pub document: String,
    pub created_at: String,
}

/// An entry moderation proposal row (§45.6).
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct EntryModProposal {
    pub id: String,
    pub entry_id: String,
    pub action: String,
    pub target_category: Option<String>,
    pub status: String,
    pub yes_votes: i64,
    pub no_votes: i64,
    pub quorum_needed: i64,
    pub closes_at: String,
    pub created_by: String,
    pub decided_by: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
}

// --- Categories --------------------------------------------------------------

/// Upsert a category (§45.1). `ON CONFLICT DO NOTHING` means a config upsert
/// adds new slugs but never resurrects a community-removed category.
pub async fn upsert_category(
    db: &Database,
    slug: &str,
    label: &str,
    source: &str,
    actor: &str,
    now: &str,
) -> Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        "INSERT INTO categories (id, slug, label, state, source, created_by, created_at)
         VALUES (?, ?, ?, 'active', ?, ?, ?)
         ON CONFLICT(slug) DO NOTHING",
        "INSERT INTO categories (id, slug, label, state, source, created_by, created_at)
         VALUES (?, ?, ?, 'active', ?, ?, ?)
         ON CONFLICT(slug) DO NOTHING",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(slug)
                .bind(label)
                .bind(source)
                .bind(actor)
                .bind(now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(slug)
                .bind(label)
                .bind(source)
                .bind(actor)
                .bind(now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Seed the built-in categories from §39.2.
pub async fn seed_categories(db: &Database, now: &str) -> Result<()> {
    let seeds: [(&str, &str); 8] = [
        ("fanfiction_archive", "Fanfiction Archives"),
        ("discord_server", "Discord Servers"),
        ("author_platform", "Author Platforms"),
        ("writing_tool", "Writing Tools"),
        ("community", "Communities"),
        ("podcast_newsletter", "Podcasts & Newsletters"),
        ("lorehaven_instance", "Lorehaven Instances"),
        ("other", "Other"),
    ];
    for (slug, label) in seeds {
        upsert_category(db, slug, label, "seed", "system", now).await?;
    }
    Ok(())
}

/// List all categories ordered by label.
pub async fn list_categories(db: &Database) -> Result<Vec<Category>> {
    let sql = db.sql(
        "SELECT * FROM categories ORDER BY label",
        "SELECT * FROM categories ORDER BY label",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, Category>(&sql)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, Category>(&sql)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

/// Get one category by slug.
pub async fn get_category(db: &Database, slug: &str) -> Result<Option<Category>> {
    let sql = db.sql(
        "SELECT * FROM categories WHERE slug = ?",
        "SELECT * FROM categories WHERE slug = ?",
    );
    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, Category>(&sql)
                .bind(slug)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, Category>(&sql)
                .bind(slug)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row)
}

/// Count active categories (§45.4 ceiling check).
pub async fn count_active_categories(db: &Database) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM categories WHERE state = 'active'",
        "SELECT COUNT(*) FROM categories WHERE state = 'active'",
    );
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(row.get::<i64, _>(0))
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(row.get::<i64, _>(0))
        }
    }
}

/// Apply a category rename (label only, slug stable — §45.1).
pub async fn rename_category(db: &Database, slug: &str, new_label: &str) -> Result<bool> {
    let sql = db.sql(
        "UPDATE categories SET label = ? WHERE slug = ?",
        "UPDATE categories SET label = ? WHERE slug = ?",
    );
    match db.backend() {
        Backend::Sqlite => {
            let result = sqlx::query(&sql)
                .bind(new_label)
                .bind(slug)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(result.rows_affected() > 0)
        }
        Backend::Postgres => {
            let result = sqlx::query(&sql)
                .bind(new_label)
                .bind(slug)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(result.rows_affected() > 0)
        }
    }
}

/// Apply a category merge: mark source merged, re-home entries (§45.1).
/// Entry scores and vote rows are untouched — no cascade.
pub async fn merge_categories(db: &Database, source_slug: &str, target_slug: &str) -> Result<bool> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let mut tx = pool.begin().await?;
            sqlx::query(
                "UPDATE categories SET state = 'merged',
                 merged_into = (SELECT id FROM categories WHERE slug = ?)
                 WHERE slug = ?",
            )
            .bind(target_slug)
            .bind(source_slug)
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE directory_entries SET category = ? WHERE category = ?")
                .bind(target_slug)
                .bind(source_slug)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let mut tx = pool.begin().await?;
            sqlx::query(
                "UPDATE categories SET state = 'merged',
                 merged_into = (SELECT id FROM categories WHERE slug = $1)
                 WHERE slug = $2",
            )
            .bind(target_slug)
            .bind(source_slug)
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE directory_entries SET category = $3 WHERE category = $4")
                .bind(target_slug)
                .bind(source_slug)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
    }
    Ok(true)
}

/// Apply a category deprecation (§45.1).
pub async fn deprecate_category(db: &Database, slug: &str) -> Result<bool> {
    let sql = db.sql(
        "UPDATE categories SET state = 'deprecated' WHERE slug = ?",
        "UPDATE categories SET state = 'deprecated' WHERE slug = ?",
    );
    match db.backend() {
        Backend::Sqlite => {
            let result = sqlx::query(&sql)
                .bind(slug)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(result.rows_affected() > 0)
        }
        Backend::Postgres => {
            let result = sqlx::query(&sql)
                .bind(slug)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(result.rows_affected() > 0)
        }
    }
}

/// Hard delete a community-created, empty category. Seed/config categories
/// and categories with entries are refused (§45.1: hard delete refused).
pub async fn hard_delete_category(db: &Database, slug: &str) -> Result<bool> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let mut tx = pool.begin().await?;
            let has_entries: i64 = sqlx::query(
                "SELECT COUNT(*) FROM directory_entries WHERE category = ? AND removed_at IS NULL",
            )
            .bind(slug)
            .fetch_one(&mut *tx)
            .await?
            .get(0);
            if has_entries > 0 {
                return Ok(false);
            }
            let result =
                sqlx::query("DELETE FROM categories WHERE slug = ? AND source = 'community'")
                    .bind(slug)
                    .execute(&mut *tx)
                    .await?;
            tx.commit().await?;
            Ok(result.rows_affected() > 0)
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let mut tx = pool.begin().await?;
            let has_entries: i64 = sqlx::query(
                "SELECT COUNT(*) FROM directory_entries WHERE category = $1 AND removed_at IS NULL",
            )
            .bind(slug)
            .fetch_one(&mut *tx)
            .await?
            .get(0);
            if has_entries > 0 {
                return Ok(false);
            }
            let result =
                sqlx::query("DELETE FROM categories WHERE slug = $2 AND source = 'community'")
                    .bind(slug)
                    .execute(&mut *tx)
                    .await?;
            tx.commit().await?;
            Ok(result.rows_affected() > 0)
        }
    }
}

// --- Proposals ---------------------------------------------------------------

/// Create a new category proposal (§45.2).
// DB functions take their parameters explicitly rather than a builder:
// a builder here would only move the same fields one call deeper.
#[allow(clippy::too_many_arguments)]
pub async fn create_proposal(
    db: &Database,
    category_slug: &str,
    action: ProposalAction,
    payload: &str,
    quorum_needed: u32,
    closes_at: &str,
    created_by: &str,
    now: &str,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        "INSERT INTO category_proposals
         (id, category_slug, action, payload, quorum_needed, closes_at, created_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO category_proposals
         (id, category_slug, action, payload, quorum_needed, closes_at, created_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(category_slug)
                .bind(action.as_str())
                .bind(payload)
                .bind(quorum_needed as i64)
                .bind(closes_at)
                .bind(created_by)
                .bind(now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(category_slug)
                .bind(action.as_str())
                .bind(payload)
                .bind(quorum_needed as i64)
                .bind(closes_at)
                .bind(created_by)
                .bind(now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(id)
}

/// Get a proposal by id.
pub async fn get_proposal(db: &Database, id: &str) -> Result<Option<CategoryProposal>> {
    let sql = db.sql(
        "SELECT * FROM category_proposals WHERE id = ?",
        "SELECT * FROM category_proposals WHERE id = ?",
    );
    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, CategoryProposal>(&sql)
                .bind(id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, CategoryProposal>(&sql)
                .bind(id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row)
}

/// List open proposals for a category.
pub async fn list_open_proposals(
    db: &Database,
    category_slug: &str,
) -> Result<Vec<CategoryProposal>> {
    let sql = db.sql(
        "SELECT * FROM category_proposals WHERE category_slug = ? AND status = 'open' ORDER BY created_at DESC",
        "SELECT * FROM category_proposals WHERE category_slug = ? AND status = 'open' ORDER BY created_at DESC",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, CategoryProposal>(&sql)
                .bind(category_slug)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, CategoryProposal>(&sql)
                .bind(category_slug)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

/// Count open proposals targeting a category (§45.4 anti-churn).
pub async fn count_open_proposals(db: &Database, category_slug: &str) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM category_proposals WHERE category_slug = ? AND status = 'open'",
        "SELECT COUNT(*) FROM category_proposals WHERE category_slug = ? AND status = 'open'",
    );
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(category_slug)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(row.get::<i64, _>(0))
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(category_slug)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(row.get::<i64, _>(0))
        }
    }
}

/// Most recent proposal of an action on a category, for the 72-hour cooldown (§45.4).
pub async fn last_proposal_time(
    db: &Database,
    category_slug: &str,
    action: &str,
) -> Result<Option<String>> {
    let sql = db.sql(
        "SELECT created_at FROM category_proposals WHERE category_slug = ? AND action = ? ORDER BY created_at DESC LIMIT 1",
        "SELECT created_at FROM category_proposals WHERE category_slug = ? AND action = ? ORDER BY created_at DESC LIMIT 1",
    );
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(category_slug)
                .bind(action)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(row.map(|r| r.get::<String, _>(0)))
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(category_slug)
                .bind(action)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(row.map(|r| r.get::<String, _>(0)))
        }
    }
}

// --- Votes -------------------------------------------------------------------

/// Cast a vote on a category proposal. Vote insert, count update and status
/// change share one transaction (§45.2 auto-execution). Returns
/// `Some(true)` passed, `Some(false)` failed, `None` still open.
pub async fn vote_on_proposal(
    db: &Database,
    proposal_id: &str,
    account_id: &str,
    value: VoteValue,
    now: &str,
) -> Result<Option<bool>> {
    let vote_id = uuid::Uuid::new_v4().to_string();
    let decided = match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let mut tx = pool.begin().await?;
            sqlx::query(
                "INSERT INTO category_votes (id, proposal_id, account_id, value, created_at)
                 VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT(proposal_id, account_id) DO NOTHING",
            )
            .bind(&vote_id)
            .bind(proposal_id)
            .bind(account_id)
            .bind(value.as_str())
            .bind(now)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE category_proposals
                 SET yes_votes = (SELECT COUNT(*) FROM category_votes WHERE proposal_id = ? AND value = 'yes'),
                     no_votes = (SELECT COUNT(*) FROM category_votes WHERE proposal_id = ? AND value = 'no')
                 WHERE id = ?",
            )
            .bind(proposal_id)
            .bind(proposal_id)
            .bind(proposal_id)
            .execute(&mut *tx)
            .await?;
            let proposal: CategoryProposal =
                sqlx::query_as("SELECT * FROM category_proposals WHERE id = ?")
                    .bind(proposal_id)
                    .fetch_one(&mut *tx)
                    .await?;
            let decided = proposal_decided(
                proposal.yes_votes as u32,
                proposal.no_votes as u32,
                proposal.quorum_needed as u32,
            );
            if let Some(passed) = decided {
                let new_status = if passed { "passed" } else { "failed" };
                sqlx::query(
                    "UPDATE category_proposals SET status = ?, decided_by = ?, decided_at = ? WHERE id = ?",
                )
                .bind(new_status)
                .bind(account_id)
                .bind(now)
                .bind(proposal_id)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
            decided
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let mut tx = pool.begin().await?;
            sqlx::query(
                "INSERT INTO category_votes (id, proposal_id, account_id, value, created_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT(proposal_id, account_id) DO NOTHING",
            )
            .bind(&vote_id)
            .bind(proposal_id)
            .bind(account_id)
            .bind(value.as_str())
            .bind(now)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE category_proposals
                 SET yes_votes = (SELECT COUNT(*) FROM category_votes WHERE proposal_id = $6 AND value = 'yes'),
                     no_votes = (SELECT COUNT(*) FROM category_votes WHERE proposal_id = $7 AND value = 'no')
                 WHERE id = $8",
            )
            .bind(proposal_id)
            .bind(proposal_id)
            .bind(proposal_id)
            .execute(&mut *tx)
            .await?;
            let proposal: CategoryProposal =
                sqlx::query_as("SELECT * FROM category_proposals WHERE id = $9")
                    .bind(proposal_id)
                    .fetch_one(&mut *tx)
                    .await?;
            let decided = proposal_decided(
                proposal.yes_votes as u32,
                proposal.no_votes as u32,
                proposal.quorum_needed as u32,
            );
            if let Some(passed) = decided {
                let new_status = if passed { "passed" } else { "failed" };
                sqlx::query(
                    "UPDATE category_proposals SET status = $10, decided_by = $11, decided_at = $12 WHERE id = $13",
                )
                .bind(new_status)
                .bind(account_id)
                .bind(now)
                .bind(proposal_id)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
            decided
        }
    };
    Ok(decided)
}

/// Veto a proposal (operator only, §45.3).
pub async fn veto_proposal(
    db: &Database,
    proposal_id: &str,
    operator_id: &str,
    reason: &str,
    now: &str,
) -> Result<bool> {
    let sql = db.sql(
        "UPDATE category_proposals SET status = 'vetoed', decided_by = ?, decision_reason = ?, decided_at = ? WHERE id = ? AND status = 'open'",
        "UPDATE category_proposals SET status = 'vetoed', decided_by = ?, decision_reason = ?, decided_at = ? WHERE id = ? AND status = 'open'",
    );
    match db.backend() {
        Backend::Sqlite => {
            let result = sqlx::query(&sql)
                .bind(operator_id)
                .bind(reason)
                .bind(now)
                .bind(proposal_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(result.rows_affected() > 0)
        }
        Backend::Postgres => {
            let result = sqlx::query(&sql)
                .bind(operator_id)
                .bind(reason)
                .bind(now)
                .bind(proposal_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(result.rows_affected() > 0)
        }
    }
}

/// Expire a proposal past its TTL without quorum (§45.2).
pub async fn expire_proposal(db: &Database, proposal_id: &str, now: &str) -> Result<bool> {
    let sql = db.sql(
        "UPDATE category_proposals SET status = 'expired', decided_at = ? WHERE id = ? AND status = 'open' AND closes_at < ?",
        "UPDATE category_proposals SET status = 'expired', decided_at = ? WHERE id = ? AND status = 'open' AND closes_at < ?",
    );
    match db.backend() {
        Backend::Sqlite => {
            let result = sqlx::query(&sql)
                .bind(now)
                .bind(proposal_id)
                .bind(now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(result.rows_affected() > 0)
        }
        Backend::Postgres => {
            let result = sqlx::query(&sql)
                .bind(now)
                .bind(proposal_id)
                .bind(now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(result.rows_affected() > 0)
        }
    }
}

// --- Changelog ---------------------------------------------------------------

/// Append a changelog entry (§45.5).
pub async fn append_changelog(
    db: &Database,
    category_slug: &str,
    event: &str,
    actor: &str,
    document: &str,
    now: &str,
) -> Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        "INSERT INTO category_changelog (id, category_slug, event, actor, document, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
        "INSERT INTO category_changelog (id, category_slug, event, actor, document, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(category_slug)
                .bind(event)
                .bind(actor)
                .bind(document)
                .bind(now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(category_slug)
                .bind(event)
                .bind(actor)
                .bind(document)
                .bind(now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// List changelog entries for a category, newest first.
pub async fn list_changelog(
    db: &Database,
    category_slug: &str,
    limit: i64,
) -> Result<Vec<CategoryChangelog>> {
    let sql = db.sql(
        "SELECT * FROM category_changelog WHERE category_slug = ? ORDER BY created_at DESC LIMIT ?",
        "SELECT * FROM category_changelog WHERE category_slug = ? ORDER BY created_at DESC LIMIT ?",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, CategoryChangelog>(&sql)
                .bind(category_slug)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, CategoryChangelog>(&sql)
                .bind(category_slug)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

// --- Entry moderation (§45.6) ------------------------------------------------

/// Create an entry moderation proposal.
pub async fn create_entry_mod_proposal(
    db: &Database,
    entry_id: &str,
    action: EntryModAction,
    target_category: Option<&str>,
    closes_at: &str,
    created_by: &str,
    now: &str,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let sql = db.sql(
        "INSERT INTO entry_moderation_proposals
         (id, entry_id, action, target_category, quorum_needed, closes_at, created_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO entry_moderation_proposals
         (id, entry_id, action, target_category, quorum_needed, closes_at, created_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(entry_id)
                .bind(action.as_str())
                .bind(target_category)
                .bind(ENTRY_MOD_QUORUM as i64)
                .bind(closes_at)
                .bind(created_by)
                .bind(now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(entry_id)
                .bind(action.as_str())
                .bind(target_category)
                .bind(ENTRY_MOD_QUORUM as i64)
                .bind(closes_at)
                .bind(created_by)
                .bind(now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(id)
}

/// Get an entry moderation proposal by id.
pub async fn get_entry_mod_proposal(db: &Database, id: &str) -> Result<Option<EntryModProposal>> {
    let sql = db.sql(
        "SELECT * FROM entry_moderation_proposals WHERE id = ?",
        "SELECT * FROM entry_moderation_proposals WHERE id = ?",
    );
    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, EntryModProposal>(&sql)
                .bind(id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, EntryModProposal>(&sql)
                .bind(id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row)
}

/// List open moderation proposals for an entry.
pub async fn list_entry_mod_proposals(
    db: &Database,
    entry_id: &str,
) -> Result<Vec<EntryModProposal>> {
    let sql = db.sql(
        "SELECT * FROM entry_moderation_proposals WHERE entry_id = ? AND status = 'open' ORDER BY created_at DESC",
        "SELECT * FROM entry_moderation_proposals WHERE entry_id = ? AND status = 'open' ORDER BY created_at DESC",
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, EntryModProposal>(&sql)
                .bind(entry_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, EntryModProposal>(&sql)
                .bind(entry_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows)
}

/// Cast a vote on an entry moderation proposal. One transaction.
pub async fn vote_on_entry_mod(
    db: &Database,
    proposal_id: &str,
    account_id: &str,
    value: VoteValue,
    now: &str,
) -> Result<Option<bool>> {
    let vote_id = uuid::Uuid::new_v4().to_string();
    let decided = match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let mut tx = pool.begin().await?;
            sqlx::query(
                "INSERT INTO entry_moderation_votes (id, proposal_id, account_id, value, created_at)
                 VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT(proposal_id, account_id) DO NOTHING",
            )
            .bind(&vote_id)
            .bind(proposal_id)
            .bind(account_id)
            .bind(value.as_str())
            .bind(now)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE entry_moderation_proposals
                 SET yes_votes = (SELECT COUNT(*) FROM entry_moderation_votes WHERE proposal_id = ? AND value = 'yes'),
                     no_votes = (SELECT COUNT(*) FROM entry_moderation_votes WHERE proposal_id = ? AND value = 'no')
                 WHERE id = ?",
            )
            .bind(proposal_id)
            .bind(proposal_id)
            .bind(proposal_id)
            .execute(&mut *tx)
            .await?;
            let proposal: EntryModProposal =
                sqlx::query_as("SELECT * FROM entry_moderation_proposals WHERE id = ?")
                    .bind(proposal_id)
                    .fetch_one(&mut *tx)
                    .await?;
            let decided = proposal_decided(
                proposal.yes_votes as u32,
                proposal.no_votes as u32,
                proposal.quorum_needed as u32,
            );
            if let Some(passed) = decided {
                let new_status = if passed { "passed" } else { "failed" };
                sqlx::query(
                    "UPDATE entry_moderation_proposals SET status = ?, decided_by = ?, decided_at = ? WHERE id = ?",
                )
                .bind(new_status)
                .bind(account_id)
                .bind(now)
                .bind(proposal_id)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
            decided
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let mut tx = pool.begin().await?;
            sqlx::query(
                "INSERT INTO entry_moderation_votes (id, proposal_id, account_id, value, created_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT(proposal_id, account_id) DO NOTHING",
            )
            .bind(&vote_id)
            .bind(proposal_id)
                .bind(account_id)
            .bind(value.as_str())
            .bind(now)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE entry_moderation_proposals
                 SET yes_votes = (SELECT COUNT(*) FROM entry_moderation_votes WHERE proposal_id = $6 AND value = 'yes'),
                     no_votes = (SELECT COUNT(*) FROM entry_moderation_votes WHERE proposal_id = $7 AND value = 'no')
                 WHERE id = $8",
            )
            .bind(proposal_id)
            .bind(proposal_id)
            .bind(proposal_id)
            .execute(&mut *tx)
            .await?;
            let proposal: EntryModProposal =
                sqlx::query_as("SELECT * FROM entry_moderation_proposals WHERE id = $9")
                    .bind(proposal_id)
                    .fetch_one(&mut *tx)
                    .await?;
            let decided = proposal_decided(
                proposal.yes_votes as u32,
                proposal.no_votes as u32,
                proposal.quorum_needed as u32,
            );
            if let Some(passed) = decided {
                let new_status = if passed { "passed" } else { "failed" };
                sqlx::query(
                    "UPDATE entry_moderation_proposals SET status = $10, decided_by = $11, decided_at = $12 WHERE id = $13",
                )
                .bind(new_status)
                .bind(account_id)
                .bind(now)
                .bind(proposal_id)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
            decided
        }
    };
    Ok(decided)
}

/// Apply a passed entry moderation action (move or remove, §45.6).
/// Move re-homes the entry; remove soft-deletes it (removed_at set, votes kept).
pub async fn apply_entry_mod_action(db: &Database, proposal_id: &str, now: &str) -> Result<bool> {
    let proposal = get_entry_mod_proposal(db, proposal_id).await?;
    let Some(p) = proposal else {
        return Ok(false);
    };
    if p.status != "passed" {
        return Ok(false);
    }

    match p.action.as_str() {
        "move" => {
            let Some(target) = &p.target_category else {
                return Ok(false);
            };
            let sql = db.sql(
                "UPDATE directory_entries SET category = ? WHERE id = ?",
                "UPDATE directory_entries SET category = ? WHERE id = ?",
            );
            match db.backend() {
                Backend::Sqlite => {
                    let result = sqlx::query(&sql)
                        .bind(target)
                        .bind(&p.entry_id)
                        .execute(db.sqlite_pool().expect("sqlite"))
                        .await?;
                    Ok(result.rows_affected() > 0)
                }
                Backend::Postgres => {
                    let result = sqlx::query(&sql)
                        .bind(target)
                        .bind(&p.entry_id)
                        .execute(db.postgres_pool().expect("postgres"))
                        .await?;
                    Ok(result.rows_affected() > 0)
                }
            }
        }
        "remove" => {
            let sql = db.sql(
                "UPDATE directory_entries SET removed_at = $1 WHERE id = $2 AND removed_at IS NULL",
                "UPDATE directory_entries SET removed_at = $3 WHERE id = $4 AND removed_at IS NULL",
            );
            match db.backend() {
                Backend::Sqlite => {
                    let result = sqlx::query(&sql)
                        .bind(now)
                        .bind(&p.entry_id)
                        .execute(db.sqlite_pool().expect("sqlite"))
                        .await?;
                    Ok(result.rows_affected() > 0)
                }
                Backend::Postgres => {
                    let result = sqlx::query(&sql)
                        .bind(now)
                        .bind(&p.entry_id)
                        .execute(db.postgres_pool().expect("postgres"))
                        .await?;
                    Ok(result.rows_affected() > 0)
                }
            }
        }
        _ => Ok(false),
    }
}
