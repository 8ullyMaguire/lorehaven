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
    pub yes_votes: i32,
    pub no_votes: i32,
    pub quorum_needed: i32,
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
    pub yes_votes: i32,
    pub no_votes: i32,
    pub quorum_needed: i32,
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
    // Both sides have to exist, and the source has to be a real category rather
    // than a redirect. Without these checks the subselect below resolves to NULL
    // and the source is left in state 'merged' pointing at nothing: a redirect
    // no reader can follow, with its entries still labelled under the dead slug.
    let Some(source) = get_category(db, source_slug).await? else {
        return Ok(false);
    };
    if source.state != "active" {
        return Ok(false);
    }
    let Some(target) = get_category(db, target_slug).await? else {
        return Ok(false);
    };
    if target.state != "active" || source_slug == target_slug {
        return Ok(false);
    }

    // The target is resolved now, so bind its id instead of re-selecting it.
    let merged_into = target.id;
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let mut tx = pool.begin().await?;
            sqlx::query(
                "UPDATE categories SET state = 'merged', merged_into = ?
                 WHERE slug = ? AND state = 'active'",
            )
            .bind(&merged_into)
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
            // Each statement numbers its own placeholders from $1: this one
            // binds two values, so it starts at $1. Continuing the previous
            // statement's numbering is not a thing sqlx does, and PostgreSQL
            // rejects $3 in a statement with two binds as "there is no parameter
            // $3" -- so a category merge failed on PostgreSQL and worked on
            // SQLite, which does not check.
            sqlx::query(
                "UPDATE categories SET state = 'merged', merged_into = $1
                 WHERE slug = $2 AND state = 'active'",
            )
            .bind(&merged_into)
            .bind(source_slug)
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE directory_entries SET category = $1 WHERE category = $2")
                .bind(target_slug)
                .bind(source_slug)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
    }
    Ok(true)
}

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
