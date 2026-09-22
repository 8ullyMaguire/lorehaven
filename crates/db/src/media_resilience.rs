//! Media resilience repository (spec §32.7.1-4).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use lorehaven_domain::media_resilience::{
    CuratorAction, LinkProvider, LinkStatus, MediaContextKind, MediaKind,
};

use crate::{sql_owned, Backend, Database};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
pub struct MediaReference {
    pub id: String,
    pub perceptual_hash: Option<String>,
    pub content_hash: String,
    pub media_kind: String,
    pub first_seen_at: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_seconds: Option<i64>,
    pub format: Option<String>,
    pub file_size_bytes: Option<i64>,
    pub content_notes: String,
    pub curator_verified: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
pub struct AvailabilityLinkRow {
    pub id: String,
    pub media_reference_id: String,
    pub url: String,
    pub provider: String,
    pub status: String,
    pub last_checked_at: String,
    pub last_healthy_at: Option<String>,
    pub consecutive_failures: i64,
    pub added_by: Option<String>,
    pub verified_by: String,
    pub reported_broken_by: String,
    pub priority: i64,
    pub failure_details: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
pub struct WorkMediaReference {
    pub id: String,
    pub work_id: String,
    pub chapter_id: Option<String>,
    pub media_reference_id: String,
    pub context: String,
    pub display_url: String,
    pub author_note: Option<String>,
    pub inserted_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
pub struct CuratorReward {
    pub id: String,
    pub account_id: String,
    pub action: String,
    pub media_reference_id: Option<String>,
    pub availability_link_id: Option<String>,
    pub amount: i64,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Media references
// ---------------------------------------------------------------------------

pub async fn insert_media_reference(
    db: &Database,
    id: &str,
    content_hash: &str,
    media_kind: MediaKind,
) -> Result<()> {
    let sql = sql_owned(
        db,
        "INSERT INTO media_references (id, content_hash, media_kind, first_seen_at)
         VALUES (?, ?, ?, datetime('now'))".to_string(),
        "INSERT INTO media_references (id, content_hash, media_kind, first_seen_at)
         VALUES ($1, $2, $3, NOW())".to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql).bind(id).bind(content_hash).bind(media_kind.as_str())
                .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql).bind(id).bind(content_hash).bind(media_kind.as_str())
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

pub async fn find_media_reference_by_content_hash(
    db: &Database,
    content_hash: &str,
) -> Result<Option<MediaReference>> {
    let sql = sql_owned(
        db,
        "SELECT id, perceptual_hash, content_hash, media_kind, first_seen_at,
                width, height, duration_seconds, format, file_size_bytes,
                content_notes,
                CASE WHEN curator_verified THEN 1 ELSE 0 END AS curator_verified,
                created_at, updated_at
         FROM media_references WHERE content_hash = ?".to_string(),
        "SELECT id, perceptual_hash, content_hash, media_kind, first_seen_at,
                width, height, duration_seconds, format, file_size_bytes,
                content_notes, curator_verified,
                created_at, updated_at
         FROM media_references WHERE content_hash = $1".to_string(),
    );
    Ok(match db.backend() {
        Backend::Sqlite => sqlx::query_as::<_, MediaReference>(&sql).bind(content_hash)
            .fetch_optional(db.sqlite_pool().expect("sqlite")).await?,
        Backend::Postgres => sqlx::query_as::<_, MediaReference>(&sql).bind(content_hash)
            .fetch_optional(db.postgres_pool().expect("postgres")).await?,
    })
}

pub async fn find_media_reference_by_id(
    db: &Database,
    id: &str,
) -> Result<Option<MediaReference>> {
    let sql = sql_owned(
        db,
        "SELECT id, perceptual_hash, content_hash, media_kind, first_seen_at,
                width, height, duration_seconds, format, file_size_bytes,
                content_notes,
                CASE WHEN curator_verified THEN 1 ELSE 0 END AS curator_verified,
                created_at, updated_at
         FROM media_references WHERE id = ?".to_string(),
        "SELECT id, perceptual_hash, content_hash, media_kind, first_seen_at,
                width, height, duration_seconds, format, file_size_bytes,
                content_notes, curator_verified,
                created_at, updated_at
         FROM media_references WHERE id = $1".to_string(),
    );
    Ok(match db.backend() {
        Backend::Sqlite => sqlx::query_as::<_, MediaReference>(&sql).bind(id)
            .fetch_optional(db.sqlite_pool().expect("sqlite")).await?,
        Backend::Postgres => sqlx::query_as::<_, MediaReference>(&sql).bind(id)
            .fetch_optional(db.postgres_pool().expect("postgres")).await?,
    })
}

// ---------------------------------------------------------------------------
// Availability links
// ---------------------------------------------------------------------------

pub async fn insert_availability_link(
    db: &Database,
    id: &str,
    media_reference_id: &str,
    url: &str,
    provider: LinkProvider,
    added_by: Option<&str>,
    priority: i64,
) -> Result<()> {
    let sql = sql_owned(
        db,
        "INSERT INTO availability_links
            (id, media_reference_id, url, provider, added_by, priority)
         VALUES (?, ?, ?, ?, ?, ?)".to_string(),
        "INSERT INTO availability_links
            (id, media_reference_id, url, provider, added_by, priority)
         VALUES ($1, $2, $3, $4, $5, $6)".to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql).bind(id).bind(media_reference_id).bind(url)
                .bind(provider.as_str()).bind(added_by).bind(priority)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql).bind(id).bind(media_reference_id).bind(url)
                .bind(provider.as_str()).bind(added_by).bind(priority)
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

pub async fn find_availability_links_for_reference(
    db: &Database,
    media_reference_id: &str,
) -> Result<Vec<AvailabilityLinkRow>> {
    let sql = sql_owned(
        db,
        "SELECT id, media_reference_id, url, provider, status, last_checked_at,
                last_healthy_at, consecutive_failures, added_by, verified_by,
                reported_broken_by, priority, failure_details, created_at, updated_at
         FROM availability_links
         WHERE media_reference_id = ?
         ORDER BY priority DESC".to_string(),
        "SELECT id, media_reference_id, url, provider, status, last_checked_at,
                last_healthy_at, consecutive_failures, added_by, verified_by,
                reported_broken_by, priority, failure_details, created_at, updated_at
         FROM availability_links
         WHERE media_reference_id = $1
         ORDER BY priority DESC".to_string(),
    );
    Ok(match db.backend() {
        Backend::Sqlite => sqlx::query_as::<_, AvailabilityLinkRow>(&sql).bind(media_reference_id)
            .fetch_all(db.sqlite_pool().expect("sqlite")).await?,
        Backend::Postgres => sqlx::query_as::<_, AvailabilityLinkRow>(&sql).bind(media_reference_id)
            .fetch_all(db.postgres_pool().expect("postgres")).await?,
    })
}

pub async fn update_link_status(
    db: &Database,
    link_id: &str,
    status: LinkStatus,
    consecutive_failures: i64,
) -> Result<()> {
    let sql = sql_owned(
        db,
        "UPDATE availability_links
         SET status = ?, consecutive_failures = ?,
             last_checked_at = datetime('now'),
             updated_at = datetime('now')
         WHERE id = ?".to_string(),
        "UPDATE availability_links
         SET status = $1, consecutive_failures = $2,
             last_checked_at = NOW(),
             updated_at = NOW()
         WHERE id = $3".to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql).bind(status.as_str()).bind(consecutive_failures).bind(link_id)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql).bind(status.as_str()).bind(consecutive_failures).bind(link_id)
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Work-media references
// ---------------------------------------------------------------------------

pub async fn insert_work_media_reference(
    db: &Database,
    id: &str,
    work_id: &str,
    chapter_id: Option<&str>,
    media_reference_id: &str,
    context: MediaContextKind,
    display_url: &str,
    author_note: Option<&str>,
) -> Result<()> {
    let sql = sql_owned(
        db,
        "INSERT INTO work_media_references
            (id, work_id, chapter_id, media_reference_id, context, display_url, author_note)
         VALUES (?, ?, ?, ?, ?, ?, ?)".to_string(),
        "INSERT INTO work_media_references
            (id, work_id, chapter_id, media_reference_id, context, display_url, author_note)
         VALUES ($1, $2, $3, $4, $5, $6, $7)".to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql).bind(id).bind(work_id).bind(chapter_id)
                .bind(media_reference_id).bind(context.as_str()).bind(display_url).bind(author_note)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql).bind(id).bind(work_id).bind(chapter_id)
                .bind(media_reference_id).bind(context.as_str()).bind(display_url).bind(author_note)
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

pub async fn find_work_media_references(
    db: &Database,
    work_id: &str,
) -> Result<Vec<WorkMediaReference>> {
    let sql = sql_owned(
        db,
        "SELECT id, work_id, chapter_id, media_reference_id, context, display_url,
                author_note, inserted_at, deleted_at
         FROM work_media_references
         WHERE work_id = ? AND deleted_at IS NULL".to_string(),
        "SELECT id, work_id, chapter_id, media_reference_id, context, display_url,
                author_note, inserted_at, deleted_at
         FROM work_media_references
         WHERE work_id = $1 AND deleted_at IS NULL".to_string(),
    );
    Ok(match db.backend() {
        Backend::Sqlite => sqlx::query_as::<_, WorkMediaReference>(&sql).bind(work_id)
            .fetch_all(db.sqlite_pool().expect("sqlite")).await?,
        Backend::Postgres => sqlx::query_as::<_, WorkMediaReference>(&sql).bind(work_id)
            .fetch_all(db.postgres_pool().expect("postgres")).await?,
    })
}

// ---------------------------------------------------------------------------
// Curator rewards
// ---------------------------------------------------------------------------

pub async fn insert_curator_reward(
    db: &Database,
    account_id: &str,
    action: CuratorAction,
    media_reference_id: Option<&str>,
    availability_link_id: Option<&str>,
    amount: i64,
) -> Result<()> {
    let sql = sql_owned(
        db,
        "INSERT INTO curator_rewards
            (id, account_id, action, media_reference_id, availability_link_id, amount)
         VALUES (?, ?, ?, ?, ?, ?)".to_string(),
        "INSERT INTO curator_rewards
            (id, account_id, action, media_reference_id, availability_link_id, amount)
         VALUES ($1, $2, $3, $4, $5, $6)".to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql).bind(Uuid::new_v4().to_string()).bind(account_id)
                .bind(action.as_str()).bind(media_reference_id).bind(availability_link_id).bind(amount)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql).bind(Uuid::new_v4().to_string()).bind(account_id)
                .bind(action.as_str()).bind(media_reference_id).bind(availability_link_id).bind(amount)
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

pub async fn sum_curator_rewards_today(
    db: &Database,
    account_id: &str,
) -> Result<i64> {
    let sql = sql_owned(
        db,
        "SELECT COALESCE(SUM(amount), 0) AS total
         FROM curator_rewards
         WHERE account_id = ?
           AND created_at >= datetime('now', 'start of day')".to_string(),
        "SELECT COALESCE(SUM(amount), 0) AS total
         FROM curator_rewards
         WHERE account_id = $1
           AND created_at >= date_trunc('day', NOW())".to_string(),
    );
    let row: (i64,) = match db.backend() {
        Backend::Sqlite => sqlx::query_as(&sql).bind(account_id)
            .fetch_one(db.sqlite_pool().expect("sqlite")).await?,
        Backend::Postgres => sqlx::query_as(&sql).bind(account_id)
            .fetch_one(db.postgres_pool().expect("postgres")).await?,
    };
    Ok(row.0)
}

// ---------------------------------------------------------------------------
// Query helpers for monitoring
// ---------------------------------------------------------------------------

pub async fn count_healthy_links(
    db: &Database,
    media_reference_id: &str,
) -> Result<i64> {
    let sql = sql_owned(
        db,
        "SELECT COUNT(*) AS cnt FROM availability_links
         WHERE media_reference_id = ? AND status = 'healthy'".to_string(),
        "SELECT COUNT(*) AS cnt FROM availability_links
         WHERE media_reference_id = $1 AND status = 'healthy'".to_string(),
    );
    let row: (i64,) = match db.backend() {
        Backend::Sqlite => sqlx::query_as(&sql).bind(media_reference_id)
            .fetch_one(db.sqlite_pool().expect("sqlite")).await?,
        Backend::Postgres => sqlx::query_as(&sql).bind(media_reference_id)
            .fetch_one(db.postgres_pool().expect("postgres")).await?,
    };
    Ok(row.0)
}

pub async fn find_links_needing_check(
    db: &Database,
    limit: i64,
) -> Result<Vec<AvailabilityLinkRow>> {
    let sql = sql_owned(
        db,
        "SELECT id, media_reference_id, url, provider, status, last_checked_at,
                last_healthy_at, consecutive_failures, added_by, verified_by,
                reported_broken_by, priority, failure_details, created_at, updated_at
         FROM availability_links
         WHERE status IN ('healthy', 'degraded', 'pending_verification')
         ORDER BY last_checked_at ASC
         LIMIT ?".to_string(),
        "SELECT id, media_reference_id, url, provider, status, last_checked_at,
                last_healthy_at, consecutive_failures, added_by, verified_by,
                reported_broken_by, priority, failure_details, created_at, updated_at
         FROM availability_links
         WHERE status IN ('healthy', 'degraded', 'pending_verification')
         ORDER BY last_checked_at ASC
         LIMIT $1".to_string(),
    );
    Ok(match db.backend() {
        Backend::Sqlite => sqlx::query_as::<_, AvailabilityLinkRow>(&sql).bind(limit)
            .fetch_all(db.sqlite_pool().expect("sqlite")).await?,
        Backend::Postgres => sqlx::query_as::<_, AvailabilityLinkRow>(&sql).bind(limit)
            .fetch_all(db.postgres_pool().expect("postgres")).await?,
    })
}

pub async fn find_references_below_threshold(
    db: &Database,
    threshold: i64,
    limit: i64,
) -> Result<Vec<MediaReference>> {
    let sql = sql_owned(
        db,
        "SELECT m.id, m.perceptual_hash, m.content_hash, m.media_kind, m.first_seen_at,
                m.width, m.height, m.duration_seconds, m.format, m.file_size_bytes,
                m.content_notes,
                CASE WHEN m.curator_verified THEN 1 ELSE 0 END AS curator_verified,
                m.created_at, m.updated_at
         FROM media_references m
         LEFT JOIN (
             SELECT media_reference_id, COUNT(*) AS healthy_count
             FROM availability_links
             WHERE status = 'healthy'
             GROUP BY media_reference_id
         ) h ON m.id = h.media_reference_id
         WHERE COALESCE(h.healthy_count, 0) < ?
         ORDER BY m.created_at ASC
         LIMIT ?".to_string(),
        "SELECT m.id, m.perceptual_hash, m.content_hash, m.media_kind, m.first_seen_at,
                m.width, m.height, m.duration_seconds, m.format, m.file_size_bytes,
                m.content_notes, m.curator_verified,
                m.created_at, m.updated_at
         FROM media_references m
         LEFT JOIN (
             SELECT media_reference_id, COUNT(*) AS healthy_count
             FROM availability_links
             WHERE status = 'healthy'
             GROUP BY media_reference_id
         ) h ON m.id = h.media_reference_id
         WHERE COALESCE(h.healthy_count, 0) < $1
         ORDER BY m.created_at ASC
         LIMIT $2".to_string(),
    );
    Ok(match db.backend() {
        Backend::Sqlite => sqlx::query_as::<_, MediaReference>(&sql).bind(threshold).bind(limit)
            .fetch_all(db.sqlite_pool().expect("sqlite")).await?,
        Backend::Postgres => sqlx::query_as::<_, MediaReference>(&sql).bind(threshold).bind(limit)
            .fetch_all(db.postgres_pool().expect("postgres")).await?,
    })
}
