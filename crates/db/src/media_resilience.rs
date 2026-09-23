//! Media resilience repository (spec §32.7.1-4).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Row};
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

/// A media reference as the reader sees it: link counts + best URL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkMediaReferenceView {
    pub id: String,
    pub work_id: String,
    pub chapter_id: Option<String>,
    pub context: String,
    pub display_url: String,
    pub author_note: Option<String>,
    pub inserted_at: String,
    pub healthy_links: i64,
    pub total_links: i64,
    pub best_url: Option<String>,
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
// §32.7.9 Import integration — media rescue during imports
// ---------------------------------------------------------------------------

/// The result of rescuing media for one import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportMediaSummary {
    /// Number of image URLs found across all chapters.
    pub total_urls: usize,
    /// Number of those URLs already held as media references.
    pub already_held: usize,
    /// Number of new media references created.
    pub new_references: usize,
    /// Number of URLs that could not be parsed or had a non-http(s) scheme.
    pub unparseable: usize,
}

/// Find the media reference for a URL, or create one with its availability link.
///
/// Deduplication is by URL string (normalised to the form `http(s)://host/path`).
/// A URL already seen by any work returns the existing reference; a URL seen
/// only within this import creates one. A duplicate URL within the import does
/// not create a second reference.
pub async fn upsert_media_reference_for_import(
    db: &Database,
    work_id: &str,
    chapter_id: Option<&str>,
    url: &str,
) -> Result<(bool, String)> {
    // Check for an existing availability link with this exact URL. The join
    // collapses a URL that any work has already rescued to the reference it
    // belongs to, so an image posted once and imported twice is one row.
    let existing = sql_owned(
        db,
        "SELECT mr.id
         FROM availability_links al
         JOIN media_references mr ON al.media_reference_id = mr.id
         WHERE al.url = ? LIMIT 1".to_string(),
        "SELECT mr.id
         FROM availability_links al
         JOIN media_references mr ON al.media_reference_id = mr.id
         WHERE al.url = $1 LIMIT 1".to_string(),
    );
    let existing_id = match db.backend() {
        Backend::Sqlite => sqlx::query(&existing).bind(url)
            .fetch_optional(db.sqlite_pool().expect("sqlite")).await?
            .map(|row| row.get::<String, _>("id")),
        Backend::Postgres => sqlx::query(&existing).bind(url)
            .fetch_optional(db.postgres_pool().expect("postgres")).await?
            .map(|row| row.get::<String, _>("id")),
    };

    // A URL already held needs only the work association; a new URL creates
    // the reference, its source link, and the association.
    let (created, reference_id) = match existing_id {
        Some(id) => (false, id),
        None => {
            let id = Uuid::new_v4().to_string();
            // The content hash is not known until the bytes are fetched; a
            // placeholder keeps the NOT NULL constraint satisfied and marks
            // the reference as awaiting its first fetch by the health
            // monitor.
            insert_media_reference(db, &id, "pending", MediaKind::Image).await?;
            insert_availability_link(
                db,
                &Uuid::new_v4().to_string(),
                &id,
                url,
                LinkProvider::Other,
                None,
                1,
            )
            .await?;
            (true, id)
        }
    };

    insert_work_media_reference(
        db,
        &Uuid::new_v4().to_string(),
        work_id,
        chapter_id,
        &reference_id,
        MediaContextKind::InlineEmbed,
        url,
        None,
    )
    .await?;

    Ok((created, reference_id))
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

// ---------------------------------------------------------------------------
// Phase 2 (§32.7.5): Curator role management
// ---------------------------------------------------------------------------

use lorehaven_domain::media_resilience::VerificationType;

/// Opt an account into the curator role.
pub async fn opt_in_curator(
    db: &Database,
    account_id: &str,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query(
                "INSERT INTO curator_roles (account_id, opted_in_at, opted_in_by)
                 VALUES (?, ?, 'self')
                 ON CONFLICT(account_id)
                 DO UPDATE SET opted_in_at = ?, opted_out_at = NULL",
            )
            .bind(account_id)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query(
                "INSERT INTO curator_roles (account_id, opted_in_at, opted_in_by)
                 VALUES ($1, $2, 'self')
                 ON CONFLICT(account_id)
                 DO UPDATE SET opted_in_at = $2, opted_out_at = NULL",
            )
            .bind(account_id)
            .bind(&now)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// Opt an account out of the curator role.
pub async fn opt_out_curator(
    db: &Database,
    account_id: &str,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query("UPDATE curator_roles SET opted_out_at = ? WHERE account_id = ?")
                .bind(&now)
                .bind(account_id)
                .execute(pool)
                .await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query("UPDATE curator_roles SET opted_out_at = $1 WHERE account_id = $2")
                .bind(&now)
                .bind(account_id)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

/// Check whether a given account+pseud can edit a work (owner, contributor,
/// or operator with trust_level >= 5).
pub async fn can_edit_work(
    db: &Database,
    work_id: &str,
    account_id: &str,
    pseud_id: &str,
) -> Result<bool, sqlx::Error> {
    let is_operator = {
        let level: i64 = match db.backend() {
            Backend::Sqlite => {
                sqlx::query_scalar("SELECT level FROM trust_levels WHERE account = ?")
                    .bind(account_id)
                    .fetch_optional(db.sqlite_pool().expect("sqlite"))
                    .await?
                    .unwrap_or(0)
            }
            Backend::Postgres => {
                sqlx::query_scalar("SELECT level FROM trust_levels WHERE account = $1")
                    .bind(account_id)
                    .fetch_optional(db.postgres_pool().expect("postgres"))
                    .await?
                    .map(|v: i32| v as i64)
                    .unwrap_or(0)
            }
        };
        level >= 5
    };

    let is_owner_or_contributor: bool = match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM works w
                 WHERE w.id = ? AND (
                   w.owner_pseud_id = ?
                   OR EXISTS (
                     SELECT 1 FROM work_contributors c
                     WHERE c.work_id = w.id AND c.pseud_id = ?
                   )
                 )",
            )
            .bind(work_id)
            .bind(pseud_id)
            .bind(pseud_id)
            .fetch_one(pool)
            .await?;
            count > 0
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM works w
                 WHERE w.id = $1 AND (
                   w.owner_pseud_id = $2
                   OR EXISTS (
                     SELECT 1 FROM work_contributors c
                     WHERE c.work_id = w.id AND c.pseud_id = $3
                   )
                 )",
            )
            .bind(work_id)
            .bind(pseud_id)
            .bind(pseud_id)
            .fetch_one(pool)
            .await?;
            count > 0
        }
    };

    Ok(is_operator || is_owner_or_contributor)
}

/// Get the best available URL for a media reference, or None if all dead.
pub async fn get_best_available_link(
    db: &Database,
    reference_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let url: Option<String> = sqlx::query_scalar(
                "SELECT url FROM availability_links
                 WHERE media_reference_id = ?
                   AND status != 'dead'
                   AND last_result != 'failed'
                 ORDER BY priority DESC
                 LIMIT 1",
            )
            .bind(reference_id)
            .fetch_optional(pool)
            .await?;
            Ok(url)
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let url: Option<String> = sqlx::query_scalar(
                "SELECT url FROM availability_links
                 WHERE media_reference_id = $1
                   AND status != 'dead'
                   AND last_result != 'failed'
                 ORDER BY priority DESC
                 LIMIT 1",
            )
            .bind(reference_id)
            .fetch_optional(pool)
            .await?;
            Ok(url)
        }
    }
}

/// Count total links for a reference.
pub async fn count_total_links(
    db: &Database,
    reference_id: &str,
) -> Result<i64, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM availability_links WHERE media_reference_id = ?",
            )
            .bind(reference_id)
            .fetch_one(pool)
            .await?;
            Ok(count)
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM availability_links WHERE media_reference_id = $1",
            )
            .bind(reference_id)
            .fetch_one(pool)
            .await?;
            Ok(count)
        }
    }
}

/// List all media references for a work with link counts and best URL.
pub async fn list_work_media_references(
    db: &Database,
    work_id: &str,
) -> Result<Vec<WorkMediaReferenceView>, sqlx::Error> {
    // First fetch the rows (dialect-specific), then map to the view.
    // Avoids the match-arm type mismatch between SqliteRow and PgRow.
    let mut out = Vec::new();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let rows = sqlx::query(
                "SELECT id, work_id, chapter_id, context, display_url, author_note, inserted_at
                 FROM work_media_references
                 WHERE work_id = ? AND deleted_at IS NULL
                 ORDER BY inserted_at",
            )
            .bind(work_id)
            .fetch_all(pool)
            .await?;
            for row in rows {
                let id: String = row.get("id");
                let work_id: String = row.get("work_id");
                let chapter_id: Option<String> = row.get("chapter_id");
                let context: String = row.get("context");
                let display_url: String = row.get("display_url");
                let author_note: Option<String> = row.get("author_note");
                let inserted_at: String = row.get("inserted_at");
                let healthy = count_healthy_links(db, &id).await.unwrap_or(0);
                let total = count_total_links(db, &id).await.unwrap_or(0);
                let best = get_best_available_link(db, &id).await?;
                out.push(build_media_ref_view(
                    id, work_id, chapter_id, context, display_url, author_note, inserted_at, healthy, total, best,
                ));
            }
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let rows = sqlx::query(
                "SELECT id, work_id, chapter_id, context, display_url, author_note, inserted_at
                 FROM work_media_references
                 WHERE work_id = $1 AND deleted_at IS NULL
                 ORDER BY inserted_at",
            )
            .bind(work_id)
            .fetch_all(pool)
            .await?;
            for row in rows {
                let id: String = row.get("id");
                let work_id: String = row.get("work_id");
                let chapter_id: Option<String> = row.get("chapter_id");
                let context: String = row.get("context");
                let display_url: String = row.get("display_url");
                let author_note: Option<String> = row.get("author_note");
                let inserted_at: String = row.get("inserted_at");
                let healthy = count_healthy_links(db, &id).await.unwrap_or(0);
                let total = count_total_links(db, &id).await.unwrap_or(0);
                let best = get_best_available_link(db, &id).await?;
                out.push(build_media_ref_view(
                    id, work_id, chapter_id, context, display_url, author_note, inserted_at, healthy, total, best,
                ));
            }
        }
    }
    Ok(out)
}

fn build_media_ref_view(
    id: String,
    work_id: String,
    chapter_id: Option<String>,
    context: String,
    display_url: String,
    author_note: Option<String>,
    inserted_at: String,
    healthy_links: i64,
    total_links: i64,
    best_url: Option<String>,
) -> WorkMediaReferenceView {
    WorkMediaReferenceView {
        id,
        work_id,
        chapter_id,
        context,
        display_url,
        author_note,
        inserted_at,
        healthy_links,
        total_links,
        best_url,
    }
}

/// Check whether an account is an active curator.
pub async fn is_active_curator(db: &Database, account_id: &str) -> Result<bool, sqlx::Error> {
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM curator_roles WHERE account_id = ? AND opted_out_at IS NULL",
            )
            .bind(account_id)
            .fetch_one(pool)
            .await?
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM curator_roles WHERE account_id = $1 AND opted_out_at IS NULL",
            )
            .bind(account_id)
            .fetch_one(pool)
            .await?
        }
    };
    Ok(count > 0)
}

/// List all active curators.
pub async fn list_active_curators(db: &Database) -> Result<Vec<String>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let rows = sqlx::query(
                "SELECT account_id FROM curator_roles WHERE opted_out_at IS NULL ORDER BY opted_in_at",
            )
            .fetch_all(pool)
            .await?;
            Ok(rows.iter().map(|r| r.get::<String, _>("account_id")).collect())
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let rows = sqlx::query(
                "SELECT account_id FROM curator_roles WHERE opted_out_at IS NULL ORDER BY opted_in_at",
            )
            .fetch_all(pool)
            .await?;
            Ok(rows.iter().map(|r| r.get::<String, _>("account_id")).collect())
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 2 (§32.7.5): Link verifications (quorum)
// ---------------------------------------------------------------------------

/// Record a curator verification of an availability link.
pub async fn record_link_verification(
    db: &Database,
    id: &str,
    availability_link_id: &str,
    media_reference_id: &str,
    curator_id: &str,
    verification_type: VerificationType,
    confidence: f64,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query(
                "INSERT INTO link_verifications
                    (id, availability_link_id, media_reference_id, curator_id,
                     verification_type, confidence, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(availability_link_id, curator_id) DO UPDATE SET
                     verification_type = ?, confidence = ?, created_at = ?",
            )
            .bind(id)
            .bind(availability_link_id)
            .bind(media_reference_id)
            .bind(curator_id)
            .bind(verification_type.as_str())
            .bind(confidence)
            .bind(&now)
            .bind(verification_type.as_str())
            .bind(confidence)
            .bind(&now)
            .execute(pool)
            .await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query(
                "INSERT INTO link_verifications
                    (id, availability_link_id, media_reference_id, curator_id,
                     verification_type, confidence, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT(availability_link_id, curator_id) DO UPDATE SET
                     verification_type = $5, confidence = $6, created_at = $7",
            )
            .bind(id)
            .bind(availability_link_id)
            .bind(media_reference_id)
            .bind(curator_id)
            .bind(verification_type.as_str())
            .bind(confidence)
            .bind(&now)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// Count the number of independent curators who verified a specific link.
pub async fn count_link_verifiers(
    db: &Database,
    availability_link_id: &str,
) -> Result<i64, sqlx::Error> {
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query_scalar(
                "SELECT COUNT(DISTINCT curator_id) FROM link_verifications WHERE availability_link_id = ?",
            )
            .bind(availability_link_id)
            .fetch_one(pool)
            .await?
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query_scalar(
                "SELECT COUNT(DISTINCT curator_id) FROM link_verifications WHERE availability_link_id = $1",
            )
            .bind(availability_link_id)
            .fetch_one(pool)
            .await?
        }
    };
    Ok(count)
}

/// Check whether a link has reached quorum (2+ independent verifications).
pub async fn has_quorum(
    db: &Database,
    availability_link_id: &str,
) -> Result<bool, sqlx::Error> {
    let count = count_link_verifiers(db, availability_link_id).await?;
    Ok(count >= 2)
}

/// Check whether a specific curator has verified a link (prevents self-verification gaming).
pub async fn curator_verified_link(
    db: &Database,
    curator_id: &str,
    availability_link_id: &str,
) -> Result<bool, sqlx::Error> {
    let count: i64 = match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM link_verifications WHERE curator_id = ? AND availability_link_id = ?",
            )
            .bind(curator_id)
            .bind(availability_link_id)
            .fetch_one(pool)
            .await?
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM link_verifications WHERE curator_id = $1 AND availability_link_id = $2",
            )
            .bind(curator_id)
            .bind(availability_link_id)
            .fetch_one(pool)
            .await?
        }
    };
    Ok(count > 0)
}

// ---------------------------------------------------------------------------
// Phase 2 (§32.7.5): Standing bounty matching
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MatchedBounty {
    pub bounty_id: String,
    pub name: String,
    pub reward: i64,
    pub provider: Option<String>,
    pub healthy_links_below: Option<i64>,
}

/// Find active standing bounties that match a given media reference.
pub async fn find_matching_standing_bounties(
    db: &Database,
    media_reference_id: &str,
    healthy_count: i64,
    _admin_rating: i64,
    has_archive_link: bool,
) -> Result<Vec<MatchedBounty>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let rows = sqlx::query(
                "SELECT id, name, reward, provider, healthy_links_below
                 FROM curator_standing_bounties
                 WHERE enabled = 1
                   AND (healthy_links_below IS NULL OR healthy_links_below > ?)
                   AND (has_archive_link = 0 OR ? = 0)
                 ORDER BY reward DESC",
            )
            .bind(healthy_count)
            .bind(if has_archive_link { 1 } else { 0 })
            .fetch_all(pool)
            .await?;
            Ok(rows.iter().map(|r| MatchedBounty {
                bounty_id: r.get::<String, _>("id"),
                name: r.get::<String, _>("name"),
                reward: r.get::<i64, _>("reward"),
                provider: r.get::<Option<String>, _>("provider"),
                healthy_links_below: r.get::<Option<i64>, _>("healthy_links_below"),
            }).collect())
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let rows = sqlx::query(
                "SELECT id, name, reward, provider, healthy_links_below
                 FROM curator_standing_bounties
                 WHERE enabled = true
                   AND (healthy_links_below IS NULL OR healthy_links_below > $1)
                   AND (has_archive_link = false OR $2 = false)
                 ORDER BY reward DESC",
            )
            .bind(healthy_count)
            .bind(has_archive_link)
            .fetch_all(pool)
            .await?;
            Ok(rows.iter().map(|r| MatchedBounty {
                bounty_id: r.get::<String, _>("id"),
                name: r.get::<String, _>("name"),
                reward: r.get::<i64, _>("reward"),
                provider: r.get::<Option<String>, _>("provider"),
                healthy_links_below: r.get::<Option<i64>, _>("healthy_links_below"),
            }).collect())
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 3 (§32.7.8): Author media preferences
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AuthorPreferences {
    pub account_id: String,
    pub auto_submit_to_archive: bool,
    pub prefer_curator_verified: bool,
    pub broken_link_notifications: String,
    pub allow_curator_edits: bool,
    pub minimum_healthy_links: i64,
}

/// Upsert author media preferences.
pub async fn upsert_author_preferences(
    db: &Database,
    account_id: &str,
    auto_submit: bool,
    prefer_verified: bool,
    notifications: &str,
    allow_edits: bool,
    min_healthy: i64,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query(
                "INSERT INTO author_media_preferences
                    (account_id, auto_submit_to_archive, prefer_curator_verified,
                     broken_link_notifications, allow_curator_edits, minimum_healthy_links,
                     created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(account_id) DO UPDATE SET
                    auto_submit_to_archive = ?,
                    prefer_curator_verified = ?,
                    broken_link_notifications = ?,
                    allow_curator_edits = ?,
                    minimum_healthy_links = ?,
                    updated_at = ?",
            )
            .bind(account_id)
            .bind(auto_submit as i64)
            .bind(prefer_verified as i64)
            .bind(notifications)
            .bind(allow_edits as i64)
            .bind(min_healthy)
            .bind(&now)
            .bind(&now)
            .bind(auto_submit as i64)
            .bind(prefer_verified as i64)
            .bind(notifications)
            .bind(allow_edits as i64)
            .bind(min_healthy)
            .bind(&now)
            .execute(pool)
            .await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query(
                "INSERT INTO author_media_preferences
                    (account_id, auto_submit_to_archive, prefer_curator_verified,
                     broken_link_notifications, allow_curator_edits, minimum_healthy_links,
                     created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $7)
                 ON CONFLICT(account_id) DO UPDATE SET
                    auto_submit_to_archive = $2,
                    prefer_curator_verified = $3,
                    broken_link_notifications = $4,
                    allow_curator_edits = $5,
                    minimum_healthy_links = $6,
                    updated_at = $7",
            )
            .bind(account_id)
            .bind(auto_submit)
            .bind(prefer_verified)
            .bind(notifications)
            .bind(allow_edits)
            .bind(min_healthy)
            .bind(&now)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// Get author media preferences.
pub async fn get_author_preferences(
    db: &Database,
    account_id: &str,
) -> Result<AuthorPreferences, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let row = sqlx::query(
                "SELECT account_id, auto_submit_to_archive, prefer_curator_verified,
                        broken_link_notifications, allow_curator_edits, minimum_healthy_links
                 FROM author_media_preferences WHERE account_id = ?",
            )
            .bind(account_id)
            .fetch_one(pool)
            .await?;
            Ok(AuthorPreferences {
                account_id: row.get::<String, _>("account_id"),
                auto_submit_to_archive: row.get::<i64, _>("auto_submit_to_archive") != 0,
                prefer_curator_verified: row.get::<i64, _>("prefer_curator_verified") != 0,
                broken_link_notifications: row.get::<String, _>("broken_link_notifications"),
                allow_curator_edits: row.get::<i64, _>("allow_curator_edits") != 0,
                minimum_healthy_links: row.get::<i64, _>("minimum_healthy_links"),
            })
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let row = sqlx::query(
                "SELECT account_id, auto_submit_to_archive, prefer_curator_verified,
                        broken_link_notifications, allow_curator_edits, minimum_healthy_links
                 FROM author_media_preferences WHERE account_id = $1",
            )
            .bind(account_id)
            .fetch_one(pool)
            .await?;
            Ok(AuthorPreferences {
                account_id: row.get::<String, _>("account_id"),
                auto_submit_to_archive: row.get::<bool, _>("auto_submit_to_archive"),
                prefer_curator_verified: row.get::<bool, _>("prefer_curator_verified"),
                broken_link_notifications: row.get::<String, _>("broken_link_notifications"),
                allow_curator_edits: row.get::<bool, _>("allow_curator_edits"),
                minimum_healthy_links: row.get::<i64, _>("minimum_healthy_links"),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 3 (§32.7.8): Targeted bounties (author-funded)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TargetedBounty {
    pub id: String,
    pub work_id: String,
    pub chapter_id: Option<String>,
    pub media_reference_id: Option<String>,
    pub account_id: String,
    pub reward: i64,
    pub status: String,
    pub description: Option<String>,
    pub claimed_by: Option<String>,
    pub created_at: String,
}

/// Post a targeted bounty for a specific work/media reference.
pub async fn post_targeted_bounty(
    db: &Database,
    id: &str,
    work_id: &str,
    chapter_id: Option<&str>,
    media_reference_id: Option<&str>,
    account_id: &str,
    reward: i64,
    description: Option<&str>,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query(
                "INSERT INTO targeted_bounties
                    (id, work_id, chapter_id, media_reference_id, account_id,
                     reward, status, description, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, 'open', ?, ?, ?)",
            )
            .bind(id)
            .bind(work_id)
            .bind(chapter_id)
            .bind(media_reference_id)
            .bind(account_id)
            .bind(reward)
            .bind(description)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query(
                "INSERT INTO targeted_bounties
                    (id, work_id, chapter_id, media_reference_id, account_id,
                     reward, status, description, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'open', $7, $8, $8)",
            )
            .bind(id)
            .bind(work_id)
            .bind(chapter_id)
            .bind(media_reference_id)
            .bind(account_id)
            .bind(reward)
            .bind(description)
            .bind(&now)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// Claim a targeted bounty.
pub async fn claim_targeted_bounty(
    db: &Database,
    bounty_id: &str,
    claimant: &str,
) -> Result<bool, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let rows_affected = match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let result = sqlx::query(
                "UPDATE targeted_bounties
                 SET status = 'claimed', claimed_by = ?, claimed_at = ?, updated_at = ?
                 WHERE id = ? AND status = 'open'",
            )
            .bind(claimant)
            .bind(&now)
            .bind(&now)
            .bind(bounty_id)
            .execute(pool)
            .await?;
            result.rows_affected()
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let result = sqlx::query(
                "UPDATE targeted_bounties
                 SET status = 'claimed', claimed_by = $1, claimed_at = $2, updated_at = $2
                 WHERE id = $3 AND status = 'open'",
            )
            .bind(claimant)
            .bind(&now)
            .bind(bounty_id)
            .execute(pool)
            .await?;
            result.rows_affected()
        }
    };
    Ok(rows_affected > 0)
}

/// List targeted bounties for a work.
pub async fn list_targeted_bounties_for_work(
    db: &Database,
    work_id: &str,
) -> Result<Vec<TargetedBounty>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let rows = sqlx::query(
                "SELECT id, work_id, chapter_id, media_reference_id, account_id,
                        reward, status, description, claimed_by, created_at
                 FROM targeted_bounties WHERE work_id = ? ORDER BY created_at DESC",
            )
            .bind(work_id)
            .fetch_all(pool)
            .await?;
            Ok(rows.iter().map(|r| TargetedBounty {
                id: r.get::<String, _>("id"),
                work_id: r.get::<String, _>("work_id"),
                chapter_id: r.get::<Option<String>, _>("chapter_id"),
                media_reference_id: r.get::<Option<String>, _>("media_reference_id"),
                account_id: r.get::<String, _>("account_id"),
                reward: r.get::<i64, _>("reward"),
                status: r.get::<String, _>("status"),
                description: r.get::<Option<String>, _>("description"),
                claimed_by: r.get::<Option<String>, _>("claimed_by"),
                created_at: r.get::<String, _>("created_at"),
            }).collect())
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let rows = sqlx::query(
                "SELECT id, work_id, chapter_id, media_reference_id, account_id,
                        reward, status, description, claimed_by, created_at
                 FROM targeted_bounties WHERE work_id = $1 ORDER BY created_at DESC",
            )
            .bind(work_id)
            .fetch_all(pool)
            .await?;
            Ok(rows.iter().map(|r| TargetedBounty {
                id: r.get::<String, _>("id"),
                work_id: r.get::<String, _>("work_id"),
                chapter_id: r.get::<Option<String>, _>("chapter_id"),
                media_reference_id: r.get::<Option<String>, _>("media_reference_id"),
                account_id: r.get::<String, _>("account_id"),
                reward: r.get::<i64, _>("reward"),
                status: r.get::<String, _>("status"),
                description: r.get::<Option<String>, _>("description"),
                claimed_by: r.get::<Option<String>, _>("claimed_by"),
                created_at: r.get::<String, _>("created_at"),
            }).collect())
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 4 (§32.7.6): Advanced mirroring — local mirrors, IPFS, federation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LocalMirror {
    pub id: String,
    pub media_reference_id: String,
    pub storage_path: String,
    pub original_url: String,
    pub file_size_bytes: i64,
    pub content_type: String,
    pub checksum_sha256: String,
    pub mirrored_by: String,
    pub status: String,
    pub mirrored_at: String,
}

/// Record a new local mirror.
pub async fn insert_local_mirror(
    db: &Database,
    id: &str,
    media_reference_id: &str,
    storage_path: &str,
    original_url: &str,
    file_size: i64,
    content_type: &str,
    checksum: &str,
    mirrored_by: &str,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query(
                "INSERT INTO local_mirrors
                    (id, media_reference_id, storage_path, original_url, file_size_bytes,
                     content_type, checksum_sha256, mirrored_by, status, mirrored_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'active', ?)",
            )
            .bind(id)
            .bind(media_reference_id)
            .bind(storage_path)
            .bind(original_url)
            .bind(file_size)
            .bind(content_type)
            .bind(checksum)
            .bind(mirrored_by)
            .bind(&now)
            .execute(pool)
            .await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query(
                "INSERT INTO local_mirrors
                    (id, media_reference_id, storage_path, original_url, file_size_bytes,
                     content_type, checksum_sha256, mirrored_by, status, mirrored_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'active', $9)",
            )
            .bind(id)
            .bind(media_reference_id)
            .bind(storage_path)
            .bind(original_url)
            .bind(file_size)
            .bind(content_type)
            .bind(checksum)
            .bind(mirrored_by)
            .bind(&now)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// List active local mirrors for a media reference.
pub async fn list_local_mirrors(
    db: &Database,
    media_reference_id: &str,
) -> Result<Vec<LocalMirror>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let rows = sqlx::query(
                "SELECT id, media_reference_id, storage_path, original_url, file_size_bytes,
                        content_type, checksum_sha256, mirrored_by, status, mirrored_at
                 FROM local_mirrors WHERE media_reference_id = ? AND status = 'active'
                 ORDER BY mirrored_at DESC",
            )
            .bind(media_reference_id)
            .fetch_all(pool)
            .await?;
            Ok(rows.iter().map(|r| LocalMirror {
                id: r.get::<String, _>("id"),
                media_reference_id: r.get::<String, _>("media_reference_id"),
                storage_path: r.get::<String, _>("storage_path"),
                original_url: r.get::<String, _>("original_url"),
                file_size_bytes: r.get::<i64, _>("file_size_bytes"),
                content_type: r.get::<String, _>("content_type"),
                checksum_sha256: r.get::<String, _>("checksum_sha256"),
                mirrored_by: r.get::<String, _>("mirrored_by"),
                status: r.get::<String, _>("status"),
                mirrored_at: r.get::<String, _>("mirrored_at"),
            }).collect())
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let rows = sqlx::query(
                "SELECT id, media_reference_id, storage_path, original_url, file_size_bytes,
                        content_type, checksum_sha256, mirrored_by, status, mirrored_at
                 FROM local_mirrors WHERE media_reference_id = $1 AND status = 'active'
                 ORDER BY mirrored_at DESC",
            )
            .bind(media_reference_id)
            .fetch_all(pool)
            .await?;
            Ok(rows.iter().map(|r| LocalMirror {
                id: r.get::<String, _>("id"),
                media_reference_id: r.get::<String, _>("media_reference_id"),
                storage_path: r.get::<String, _>("storage_path"),
                original_url: r.get::<String, _>("original_url"),
                file_size_bytes: r.get::<i64, _>("file_size_bytes"),
                content_type: r.get::<String, _>("content_type"),
                checksum_sha256: r.get::<String, _>("checksum_sha256"),
                mirrored_by: r.get::<String, _>("mirrored_by"),
                status: r.get::<String, _>("status"),
                mirrored_at: r.get::<String, _>("mirrored_at"),
            }).collect())
        }
    }
}

/// Mark a local mirror as removed (DMCA takedown or expiry).
pub async fn deactivate_local_mirror(db: &Database, mirror_id: &str) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query("UPDATE local_mirrors SET status = 'removed', expires_at = ? WHERE id = ?")
                .bind(&now)
                .bind(mirror_id)
                .execute(pool)
                .await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query("UPDATE local_mirrors SET status = 'removed', expires_at = $1 WHERE id = $2")
                .bind(&now)
                .bind(mirror_id)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Phase 4 (§32.7.6): IPFS pin tracking
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct IpfsPin {
    pub id: String,
    pub media_reference_id: String,
    pub cid: String,
    pub pin_service: String,
    pub status: String,
    pub file_size_bytes: i64,
    pub pinned_at: String,
}

/// Record an IPFS pin.
pub async fn insert_ipfs_pin(
    db: &Database,
    id: &str,
    media_reference_id: &str,
    cid: &str,
    pin_service: &str,
    file_size: i64,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query(
                "INSERT INTO ipfs_pins
                    (id, media_reference_id, cid, pin_service, status, file_size_bytes, pinned_at)
                 VALUES (?, ?, ?, ?, 'pinned', ?, ?)",
            )
            .bind(id)
            .bind(media_reference_id)
            .bind(cid)
            .bind(pin_service)
            .bind(file_size)
            .bind(&now)
            .execute(pool)
            .await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query(
                "INSERT INTO ipfs_pins
                    (id, media_reference_id, cid, pin_service, status, file_size_bytes, pinned_at)
                 VALUES ($1, $2, $3, $4, 'pinned', $5, $6)",
            )
            .bind(id)
            .bind(media_reference_id)
            .bind(cid)
            .bind(pin_service)
            .bind(file_size)
            .bind(&now)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// List IPFS pins for a media reference.
pub async fn list_ipfs_pins(
    db: &Database,
    media_reference_id: &str,
) -> Result<Vec<IpfsPin>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let rows = sqlx::query(
                "SELECT id, media_reference_id, cid, pin_service, status, file_size_bytes, pinned_at
                 FROM ipfs_pins WHERE media_reference_id = ? AND status = 'pinned'
                 ORDER BY pinned_at DESC",
            )
            .bind(media_reference_id)
            .fetch_all(pool)
            .await?;
            Ok(rows.iter().map(|r| IpfsPin {
                id: r.get::<String, _>("id"),
                media_reference_id: r.get::<String, _>("media_reference_id"),
                cid: r.get::<String, _>("cid"),
                pin_service: r.get::<String, _>("pin_service"),
                status: r.get::<String, _>("status"),
                file_size_bytes: r.get::<i64, _>("file_size_bytes"),
                pinned_at: r.get::<String, _>("pinned_at"),
            }).collect())
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let rows = sqlx::query(
                "SELECT id, media_reference_id, cid, pin_service, status, file_size_bytes, pinned_at
                 FROM ipfs_pins WHERE media_reference_id = $1 AND status = 'pinned'
                 ORDER BY pinned_at DESC",
            )
            .bind(media_reference_id)
            .fetch_all(pool)
            .await?;
            Ok(rows.iter().map(|r| IpfsPin {
                id: r.get::<String, _>("id"),
                media_reference_id: r.get::<String, _>("media_reference_id"),
                cid: r.get::<String, _>("cid"),
                pin_service: r.get::<String, _>("pin_service"),
                status: r.get::<String, _>("status"),
                file_size_bytes: r.get::<i64, _>("file_size_bytes"),
                pinned_at: r.get::<String, _>("pinned_at"),
            }).collect())
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 4 (§32.7.6): DMCA takedowns
// ---------------------------------------------------------------------------

/// File a DMCA takedown for a local mirror.
pub async fn file_dmca_takedown(
    db: &Database,
    id: &str,
    mirror_id: &str,
    claimant_name: &str,
    claimant_email: &str,
    work_description: &str,
    complaint: &str,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query(
                "INSERT INTO dmca_takedowns
                    (id, local_mirror_id, claimant_name, claimant_email,
                     original_work_description, complaint_text, status, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, 'pending', ?)",
            )
            .bind(id)
            .bind(mirror_id)
            .bind(claimant_name)
            .bind(claimant_email)
            .bind(work_description)
            .bind(complaint)
            .bind(&now)
            .execute(pool)
            .await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query(
                "INSERT INTO dmca_takedowns
                    (id, local_mirror_id, claimant_name, claimant_email,
                     original_work_description, complaint_text, status, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7)",
            )
            .bind(id)
            .bind(mirror_id)
            .bind(claimant_name)
            .bind(claimant_email)
            .bind(work_description)
            .bind(complaint)
            .bind(&now)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// Resolve a DMCA takedown (approve = remove mirror, reject = restore).
pub async fn resolve_dmca_takedown(
    db: &Database,
    takedown_id: &str,
    approved: bool,
    resolved_by: &str,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let status = if approved { "approved" } else { "rejected" };
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query(
                "UPDATE dmca_takedowns SET status = ?, resolved_at = ?, resolved_by = ? WHERE id = ?",
            )
            .bind(status)
            .bind(&now)
            .bind(resolved_by)
            .bind(takedown_id)
            .execute(pool)
            .await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query(
                "UPDATE dmca_takedowns SET status = $1, resolved_at = $2, resolved_by = $3 WHERE id = $4",
            )
            .bind(status)
            .bind(&now)
            .bind(resolved_by)
            .bind(takedown_id)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Phase 5 (§32.7.3 & §32.7.7): Reverse search & discovery
// ---------------------------------------------------------------------------

/// Find media references by perceptual hash (dedup & reverse lookup).
/// Searches for hashes within `distance` (Hamming distance threshold).
pub async fn find_by_perceptual_hash(
    db: &Database,
    hash: &str,
    max_distance: i32,
) -> Result<Vec<MediaReference>, sqlx::Error> {
    // Exact match first; fuzzy match is a placeholder for perceptual hash hamming distance
    // which would require a pgcrypto extension or application-side comparison.
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let rows = sqlx::query(
                "SELECT id, perceptual_hash, content_hash, media_kind, first_seen_at,
                        width, height, duration_seconds, format, file_size_bytes,
                        content_notes, curator_verified, created_at, updated_at
                 FROM media_references
                 WHERE perceptual_hash = ?",
            )
            .bind(hash)
            .fetch_all(pool)
            .await?;
            Ok(rows.iter().map(|r| MediaReference {
                id: r.get::<String, _>("id"),
                perceptual_hash: r.get::<Option<String>, _>("perceptual_hash"),
                content_hash: r.get::<String, _>("content_hash"),
                media_kind: r.get::<String, _>("media_kind"),
                first_seen_at: r.get::<String, _>("first_seen_at"),
                width: r.get::<Option<i64>, _>("width"),
                height: r.get::<Option<i64>, _>("height"),
                duration_seconds: r.get::<Option<i64>, _>("duration_seconds"),
                format: r.get::<Option<String>, _>("format"),
                file_size_bytes: r.get::<Option<i64>, _>("file_size_bytes"),
                content_notes: r.get::<String, _>("content_notes"),
                curator_verified: r.get::<bool, _>("curator_verified"),
                created_at: r.get::<String, _>("created_at"),
                updated_at: r.get::<String, _>("updated_at"),
            }).collect())
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let rows = sqlx::query(
                "SELECT id, perceptual_hash, content_hash, media_kind, first_seen_at,
                        width, height, duration_seconds, format, file_size_bytes,
                        content_notes, curator_verified, created_at, updated_at
                 FROM media_references
                 WHERE perceptual_hash = $1",
            )
            .bind(hash)
            .fetch_all(pool)
            .await?;
            Ok(rows.iter().map(|r| MediaReference {
                id: r.get::<String, _>("id"),
                perceptual_hash: r.get::<Option<String>, _>("perceptual_hash"),
                content_hash: r.get::<String, _>("content_hash"),
                media_kind: r.get::<String, _>("media_kind"),
                first_seen_at: r.get::<String, _>("first_seen_at"),
                width: r.get::<Option<i64>, _>("width"),
                height: r.get::<Option<i64>, _>("height"),
                duration_seconds: r.get::<Option<i64>, _>("duration_seconds"),
                format: r.get::<Option<String>, _>("format"),
                file_size_bytes: r.get::<Option<i64>, _>("file_size_bytes"),
                content_notes: r.get::<String, _>("content_notes"),
                curator_verified: r.get::<bool, _>("curator_verified"),
                created_at: r.get::<String, _>("created_at"),
                updated_at: r.get::<String, _>("updated_at"),
            }).collect())
        }
    }
}

/// Find media references linked to a curator with low healthy link counts (curator bounty queue).
pub async fn find_curator_bounty_queue(
    db: &Database,
    healthy_links_below: i32,
    limit: i32,
) -> Result<Vec<MediaReference>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let rows = sqlx::query(
                "SELECT m.id, m.perceptual_hash, m.content_hash, m.media_kind, m.first_seen_at,
                        m.width, m.height, m.duration_seconds, m.format, m.file_size_bytes,
                        m.content_notes, m.curator_verified, m.created_at, m.updated_at
                 FROM media_references m
                 WHERE (SELECT COUNT(*) FROM availability_links
                        WHERE media_reference_id = m.id AND status = 'healthy') < ?
                 ORDER BY (SELECT COUNT(*) FROM availability_links
                           WHERE media_reference_id = m.id AND status = 'healthy') ASC
                 LIMIT ?",
            )
            .bind(healthy_links_below)
            .bind(limit)
            .fetch_all(pool)
            .await?;
            Ok(rows.iter().map(|r| MediaReference {
                id: r.get::<String, _>("id"),
                perceptual_hash: r.get::<Option<String>, _>("perceptual_hash"),
                content_hash: r.get::<String, _>("content_hash"),
                media_kind: r.get::<String, _>("media_kind"),
                first_seen_at: r.get::<String, _>("first_seen_at"),
                width: r.get::<Option<i64>, _>("width"),
                height: r.get::<Option<i64>, _>("height"),
                duration_seconds: r.get::<Option<i64>, _>("duration_seconds"),
                format: r.get::<Option<String>, _>("format"),
                file_size_bytes: r.get::<Option<i64>, _>("file_size_bytes"),
                content_notes: r.get::<String, _>("content_notes"),
                curator_verified: r.get::<bool, _>("curator_verified"),
                created_at: r.get::<String, _>("created_at"),
                updated_at: r.get::<String, _>("updated_at"),
            }).collect())
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let rows = sqlx::query(
                "SELECT m.id, m.perceptual_hash, m.content_hash, m.media_kind, m.first_seen_at,
                        m.width, m.height, m.duration_seconds, m.format, m.file_size_bytes,
                        m.content_notes, m.curator_verified, m.created_at, m.updated_at
                 FROM media_references m
                 WHERE (SELECT COUNT(*) FROM availability_links
                        WHERE media_reference_id = m.id AND status = 'healthy') < $1
                 ORDER BY (SELECT COUNT(*) FROM availability_links
                           WHERE media_reference_id = m.id AND status = 'healthy') ASC
                 LIMIT $2",
            )
            .bind(healthy_links_below)
            .bind(limit)
            .fetch_all(pool)
            .await?;
            Ok(rows.iter().map(|r| MediaReference {
                id: r.get::<String, _>("id"),
                perceptual_hash: r.get::<Option<String>, _>("perceptual_hash"),
                content_hash: r.get::<String, _>("content_hash"),
                media_kind: r.get::<String, _>("media_kind"),
                first_seen_at: r.get::<String, _>("first_seen_at"),
                width: r.get::<Option<i64>, _>("width"),
                height: r.get::<Option<i64>, _>("height"),
                duration_seconds: r.get::<Option<i64>, _>("duration_seconds"),
                format: r.get::<Option<String>, _>("format"),
                file_size_bytes: r.get::<Option<i64>, _>("file_size_bytes"),
                content_notes: r.get::<String, _>("content_notes"),
                curator_verified: r.get::<bool, _>("curator_verified"),
                created_at: r.get::<String, _>("created_at"),
                updated_at: r.get::<String, _>("updated_at"),
            }).collect())
        }
    }
}

// ---------------------------------------------------------------------------
// Admin dashboard metrics (§32.7.11)
// ---------------------------------------------------------------------------

/// Count media references that have at least `min_healthy` healthy links.
pub async fn count_well_mirrored(db: &Database, min_healthy: i64) -> Result<i64> {
    let sql = sql_owned(
        db,
        "SELECT COUNT(*) AS cnt FROM (
            SELECT m.id FROM media_references m
            LEFT JOIN availability_links al ON al.media_reference_id = m.id AND al.status = 'healthy'
            GROUP BY m.id HAVING COUNT(al.id) >= ?
        )".to_string(),
        "SELECT COUNT(*) AS cnt FROM (
            SELECT m.id FROM media_references m
            LEFT JOIN availability_links al ON al.media_reference_id = m.id AND al.status = 'healthy'
            GROUP BY m.id HAVING COUNT(al.id) >= $1
        )".to_string(),
    );
    let row: (i64,) = match db.backend() {
        Backend::Sqlite => sqlx::query_as(&sql).bind(min_healthy)
            .fetch_one(db.sqlite_pool().expect("sqlite")).await?,
        Backend::Postgres => sqlx::query_as(&sql).bind(min_healthy)
            .fetch_one(db.postgres_pool().expect("postgres")).await?,
    };
    Ok(row.0)
}

/// Total number of media references.
pub async fn count_total_references(db: &Database) -> Result<i64> {
    let sql = sql_owned(
        db,
        "SELECT COUNT(*) AS cnt FROM media_references".to_string(),
        "SELECT COUNT(*) AS cnt FROM media_references".to_string(),
    );
    let row: (i64,) = match db.backend() {
        Backend::Sqlite => sqlx::query_as(&sql)
            .fetch_one(db.sqlite_pool().expect("sqlite")).await?,
        Backend::Postgres => sqlx::query_as(&sql)
            .fetch_one(db.postgres_pool().expect("postgres")).await?,
    };
    Ok(row.0)
}

/// Link rot: for each provider, how many links went from healthy to non-healthy
/// since the given datetime (ISO-8601 string). Returns Vec<(provider, rot_count)>.
pub async fn link_rot_by_provider(
    db: &Database,
    since: &str,
) -> Result<Vec<(String, i64)>> {
    let sql = sql_owned(
        db,
        "SELECT provider, COUNT(*) AS cnt FROM availability_links
         WHERE updated_at > ? AND status != 'healthy' AND last_healthy_at IS NOT NULL
         AND last_healthy_at < updated_at
         GROUP BY provider ORDER BY cnt DESC".to_string(),
        "SELECT provider, COUNT(*) AS cnt FROM availability_links
         WHERE updated_at > $1 AND status != 'healthy' AND last_healthy_at IS NOT NULL
         AND last_healthy_at < updated_at
         GROUP BY provider ORDER BY cnt DESC".to_string(),
    );
    let rows: Vec<(String, i64)> = match db.backend() {
        Backend::Sqlite => sqlx::query_as(&sql).bind(since)
            .fetch_all(db.sqlite_pool().expect("sqlite")).await?,
        Backend::Postgres => sqlx::query_as(&sql).bind(since)
            .fetch_all(db.postgres_pool().expect("postgres")).await?,
    };
    Ok(rows)
}

/// Count of references below the healthy-link threshold (need rescue).
pub async fn count_references_below_threshold(db: &Database, min_healthy: i64) -> Result<i64> {
    let sql = sql_owned(
        db,
        "SELECT COUNT(*) AS cnt FROM media_references m
         WHERE (
            SELECT COUNT(*) FROM availability_links al
            WHERE al.media_reference_id = m.id AND al.status = 'healthy'
         ) < ?".to_string(),
        "SELECT COUNT(*) AS cnt FROM media_references m
         WHERE (
            SELECT COUNT(*) FROM availability_links al
            WHERE al.media_reference_id = m.id AND al.status = 'healthy'
         ) < $1".to_string(),
    );
    let row: (i64,) = match db.backend() {
        Backend::Sqlite => sqlx::query_as(&sql).bind(min_healthy)
            .fetch_one(db.sqlite_pool().expect("sqlite")).await?,
        Backend::Postgres => sqlx::query_as(&sql).bind(min_healthy)
            .fetch_one(db.postgres_pool().expect("postgres")).await?,
    };
    Ok(row.0)
}

/// Curator leaderboard: top curators by reward count, with their total amount.
/// Returns Vec<(account_id, reward_count, total_amount)>.
pub async fn curator_leaderboard(
    db: &Database,
    limit: i64,
) -> Result<Vec<(String, i64, i64)>> {
    let sql = sql_owned(
        db,
        "SELECT account_id,
                COUNT(*) AS reward_count,
                SUM(amount) AS total_amount
         FROM curator_rewards
         GROUP BY account_id
         ORDER BY total_amount DESC
         LIMIT ?".to_string(),
        "SELECT account_id,
                COUNT(*) AS reward_count,
                SUM(amount) AS total_amount
         FROM curator_rewards
         GROUP BY account_id
         ORDER BY total_amount DESC
         LIMIT $1".to_string(),
    );
    let rows: Vec<(String, i64, i64)> = match db.backend() {
        Backend::Sqlite => sqlx::query_as(&sql).bind(limit)
            .fetch_all(db.sqlite_pool().expect("sqlite")).await?,
        Backend::Postgres => sqlx::query_as(&sql).bind(limit)
            .fetch_all(db.postgres_pool().expect("postgres")).await?,
    };
    Ok(rows)
}

/// Count of active standing bounties and total amount available.
pub async fn standing_bounty_status(db: &Database) -> Result<(i64, i64)> {
    let sql = sql_owned(
        db,
        "SELECT COUNT(*) AS cnt, COALESCE(SUM(reward), 0) AS total
         FROM targeted_bounties WHERE status = 'active'".to_string(),
        "SELECT COUNT(*) AS cnt, COALESCE(SUM(reward), 0) AS total
         FROM targeted_bounties WHERE status = 'active'".to_string(),
    );
    let row: (i64, i64) = match db.backend() {
        Backend::Sqlite => sqlx::query_as(&sql)
            .fetch_one(db.sqlite_pool().expect("sqlite")).await?,
        Backend::Postgres => sqlx::query_as(&sql)
            .fetch_one(db.postgres_pool().expect("postgres")).await?,
    };
    Ok(row)
}

/// Local mirror storage consumed: total file size in bytes and count of mirrors.
pub async fn local_mirror_storage(db: &Database) -> Result<(i64, i64)> {
    let sql = sql_owned(
        db,
        "SELECT COUNT(*) AS cnt, COALESCE(SUM(file_size_bytes), 0) AS total_bytes
         FROM local_mirrors".to_string(),
        "SELECT COUNT(*) AS cnt, COALESCE(SUM(file_size_bytes), 0) AS total_bytes
         FROM local_mirrors".to_string(),
    );
    let row: (i64, i64) = match db.backend() {
        Backend::Sqlite => sqlx::query_as(&sql)
            .fetch_one(db.sqlite_pool().expect("sqlite")).await?,
        Backend::Postgres => sqlx::query_as(&sql)
            .fetch_one(db.postgres_pool().expect("postgres")).await?,
    };
    Ok(row)
}

/// Count of active IPFS pins.
pub async fn count_active_ipfs_pins(db: &Database) -> Result<i64> {
    let sql = sql_owned(
        db,
        "SELECT COUNT(*) AS cnt FROM ipfs_pins".to_string(),
        "SELECT COUNT(*) AS cnt FROM ipfs_pins".to_string(),
    );
    let row: (i64,) = match db.backend() {
        Backend::Sqlite => sqlx::query_as(&sql)
            .fetch_one(db.sqlite_pool().expect("sqlite")).await?,
        Backend::Postgres => sqlx::query_as(&sql)
            .fetch_one(db.postgres_pool().expect("postgres")).await?,
    };
    Ok(row.0)
}

/// Provider reliability: ranked by health rate (healthy / total).
/// Returns Vec<(provider, healthy_count, total_count)>.
pub async fn provider_reliability(db: &Database) -> Result<Vec<(String, i64, i64)>> {
    let sql = sql_owned(
        db,
        "SELECT provider,
                SUM(CASE WHEN status = 'healthy' THEN 1 ELSE 0 END) AS healthy,
                COUNT(*) AS total
         FROM availability_links
         GROUP BY provider
         ORDER BY (healthy * 1.0 / total) DESC".to_string(),
        "SELECT provider,
                SUM(CASE WHEN status = 'healthy' THEN 1 ELSE 0 END) AS healthy,
                COUNT(*) AS total
         FROM availability_links
         GROUP BY provider
         ORDER BY (healthy::float / total) DESC".to_string(),
    );
    let rows: Vec<(String, i64, i64)> = match db.backend() {
        Backend::Sqlite => sqlx::query_as(&sql)
            .fetch_all(db.sqlite_pool().expect("sqlite")).await?,
        Backend::Postgres => sqlx::query_as(&sql)
            .fetch_all(db.postgres_pool().expect("postgres")).await?,
    };
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Author media health report (§32.7.8)
// ---------------------------------------------------------------------------

/// One row in the per-work author media health report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthorWorkMediaHealth {
    pub work_id: String,
    pub work_title: String,
    pub total_references: i64,
    pub healthy_references: i64,
    pub at_risk_references: i64,
    pub broken_references: i64,
}

/// Build the media health report for all works owned by an account.
pub async fn author_media_health_report(
    db: &Database,
    account_id: &str,
) -> Result<Vec<AuthorWorkMediaHealth>> {
    let sql = sql_owned(
        db,
        "SELECT w.id AS work_id,
                w.title AS work_title,
                COUNT(wmr.id) AS total_references,
                SUM(CASE WHEN COALESCE(healthy.cnt, 0) >= 3 THEN 1 ELSE 0 END) AS healthy_references,
                SUM(CASE WHEN COALESCE(healthy.cnt, 0) BETWEEN 1 AND 2 THEN 1 ELSE 0 END) AS at_risk_references,
                SUM(CASE WHEN COALESCE(healthy.cnt, 0) = 0 THEN 1 ELSE 0 END) AS broken_references
         FROM works w
         JOIN work_media_references wmr ON wmr.work_id = w.id AND wmr.deleted_at IS NULL
         LEFT JOIN (
             SELECT al.media_reference_id, COUNT(al.id) AS cnt
             FROM availability_links al
             WHERE al.status = 'healthy'
             GROUP BY al.media_reference_id
         ) healthy ON healthy.media_reference_id = wmr.media_reference_id
         WHERE w.owner_account_id = ?
         GROUP BY w.id, w.title
         ORDER BY w.title"
            .to_string(),
        "SELECT w.id AS work_id,
                w.title AS work_title,
                COUNT(wmr.id) AS total_references,
                SUM(CASE WHEN COALESCE(healthy.cnt, 0) >= 3 THEN 1 ELSE 0 END) AS healthy_references,
                SUM(CASE WHEN COALESCE(healthy.cnt, 0) BETWEEN 1 AND 2 THEN 1 ELSE 0 END) AS at_risk_references,
                SUM(CASE WHEN COALESCE(healthy.cnt, 0) = 0 THEN 1 ELSE 0 END) AS broken_references
         FROM works w
         JOIN work_media_references wmr ON wmr.work_id = w.id AND wmr.deleted_at IS NULL
         LEFT JOIN (
             SELECT al.media_reference_id, COUNT(al.id) AS cnt
             FROM availability_links al
             WHERE al.status = 'healthy'
             GROUP BY al.media_reference_id
         ) healthy ON healthy.media_reference_id = wmr.media_reference_id
         WHERE w.owner_account_id = $1
         GROUP BY w.id, w.title
         ORDER BY w.title"
            .to_string(),
    );
    let rows = match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            sqlx::query(&sql)
                .bind(account_id)
                .fetch_all(pool)
                .await?
                .iter()
                .map(|row| {
                    Ok::<AuthorWorkMediaHealth, anyhow::Error>(AuthorWorkMediaHealth {
                        work_id: row.get("work_id"),
                        work_title: row.get("work_title"),
                        total_references: row.get("total_references"),
                        healthy_references: row.get("healthy_references"),
                        at_risk_references: row.get("at_risk_references"),
                        broken_references: row.get("broken_references"),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query(&sql)
                .bind(account_id)
                .fetch_all(pool)
                .await?
                .iter()
                .map(|row| {
                    Ok::<AuthorWorkMediaHealth, anyhow::Error>(AuthorWorkMediaHealth {
                        work_id: row.get("work_id"),
                        work_title: row.get("work_title"),
                        total_references: row.get("total_references"),
                        healthy_references: row.get("healthy_references"),
                        at_risk_references: row.get("at_risk_references"),
                        broken_references: row.get("broken_references"),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    Ok(rows)
}
