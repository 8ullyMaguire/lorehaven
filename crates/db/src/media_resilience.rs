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
         VALUES (?, ?, ?, datetime('now'))"
            .to_string(),
        // `media_references.id` is UUID on PostgreSQL.
        "INSERT INTO media_references (id, content_hash, media_kind, first_seen_at)
         VALUES ($1::uuid, $2, $3, NOW())"
            .to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(id)
                .bind(content_hash)
                .bind(media_kind.as_str())
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(id)
                .bind(content_hash)
                .bind(media_kind.as_str())
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// What a fetch learned about a media reference's bytes.
///
/// Deliberately a local type rather than a dependency on the app layer: the
/// repository is below the fetcher, not above it, and a db function that took
/// `app::media_fetch::MediaFingerprint` would invert the dependency for the
/// sake of a type alias. The fields are what the row needs and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    /// `sha256:` plus hex of the fetched bytes.
    pub content_hash: String,
    /// The perceptual hash, or `None` when the body could not be decoded to
    /// pixels. Never an empty string: the dedup search skips `NULL` and skips
    /// malformed values, but two empty strings are distance 0 apart and would
    /// merge every undecodable image into one reference.
    pub perceptual_hash: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
}

/// Record a fetched media reference's hashes and dimensions.
///
/// This is what makes the perceptual search real: `insert_media_reference`
/// writes the placeholder `pending` because the bytes are not known until they
/// are fetched, and this is the call that replaces it. The `content_hash`
/// update and the `perceptual_hash` update are the same statement, because a
/// reference that carries a perceptual hash while still claiming to be
/// `pending` would be a row that lies about being unfetched.
///
/// # Errors
/// An error when no such reference exists. A fetch that completed for a row
/// that has since been deleted must say so rather than report success, or the
/// caller records the media as mirrored when the reference it belonged to is
/// gone.
pub async fn record_fingerprint(
    db: &Database,
    reference_id: &str,
    fingerprint: &Fingerprint,
) -> Result<()> {
    let sql = sql_owned(
        db,
        "UPDATE media_references
         SET content_hash = ?, perceptual_hash = ?, width = ?, height = ?,
             updated_at = datetime('now')
         WHERE id = ?"
            .to_string(),
        "UPDATE media_references
         SET content_hash = $1, perceptual_hash = $2, width = $3, height = $4,
             updated_at = NOW()
         WHERE id = $5::uuid"
            .to_string(),
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&fingerprint.content_hash)
            .bind(&fingerprint.perceptual_hash)
            .bind(fingerprint.width)
            .bind(fingerprint.height)
            .bind(reference_id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&fingerprint.content_hash)
            .bind(&fingerprint.perceptual_hash)
            .bind(fingerprint.width)
            .bind(fingerprint.height)
            .bind(reference_id)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    if affected == 0 {
        // The crate's Result is anyhow's, not sqlx's, so the "no such row"
        // signal is a message rather than a sentinel error type.
        anyhow::bail!("no media reference with id {reference_id}");
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
         FROM media_references WHERE content_hash = ?"
            .to_string(),
        // The row reader takes Strings, so every column PostgreSQL does not
        // already store as text has to be cast: `id` is UUID, the three
        // timestamps are TIMESTAMPTZ, `content_notes` is JSONB, and the
        // dimensions are INT4. The SQLite arm needs none of this.
        "SELECT id::text, perceptual_hash, content_hash, media_kind,
                first_seen_at::text, width::bigint, height::bigint,
                duration_seconds::bigint, format,
                file_size_bytes::bigint, content_notes::text, curator_verified,
                created_at::text, updated_at::text
         FROM media_references WHERE content_hash = $1"
            .to_string(),
    );
    Ok(match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, MediaReference>(&sql)
                .bind(content_hash)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, MediaReference>(&sql)
                .bind(content_hash)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    })
}

pub async fn find_media_reference_by_id(db: &Database, id: &str) -> Result<Option<MediaReference>> {
    let sql = sql_owned(
        db,
        "SELECT id, perceptual_hash, content_hash, media_kind, first_seen_at,
                width, height, duration_seconds, format, file_size_bytes,
                content_notes,
                CASE WHEN curator_verified THEN 1 ELSE 0 END AS curator_verified,
                created_at, updated_at
         FROM media_references WHERE id = ?"
            .to_string(),
        // `id` is read back into a String, so cast it to text: sqlx will not
        // decode UUID into String.
        // Row reader takes Strings: `id` is UUID and the three timestamps are
        // TIMESTAMPTZ on PostgreSQL, so both need casting.
        "SELECT id::text, perceptual_hash, content_hash, media_kind,
                first_seen_at::text, width::bigint, height::bigint,
                duration_seconds::bigint, format,
                file_size_bytes::bigint, content_notes::text, curator_verified,
                created_at::text, updated_at::text
         FROM media_references WHERE id = $1::uuid"
            .to_string(),
    );
    Ok(match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, MediaReference>(&sql)
                .bind(id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, MediaReference>(&sql)
                .bind(id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
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
         VALUES (?, ?, ?, ?, ?, ?)"
            .to_string(),
        // `availability_links.id` and `.media_reference_id` are UUID, and
        // `.added_by` is a pseud uuid.
        "INSERT INTO availability_links
            (id, media_reference_id, url, provider, added_by, priority)
         VALUES ($1::uuid, $2::uuid, $3, $4, $5::uuid, $6)"
            .to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(id)
                .bind(media_reference_id)
                .bind(url)
                .bind(provider.as_str())
                .bind(added_by)
                .bind(priority)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(id)
                .bind(media_reference_id)
                .bind(url)
                .bind(provider.as_str())
                .bind(added_by)
                .bind(priority)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
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
         ORDER BY priority DESC"
            .to_string(),
        // The row struct is all String/i64, so cast everything PostgreSQL does
        // not already store as text: two uuid columns, three timestamptz, two
        // int4, one jsonb, and the two uuid[] columns. Those last two are
        // rendered with `array_to_json` so the value is the same JSON array
        // string the SQLite arm holds, and the row struct stays identical.
        "SELECT id::text, media_reference_id::text, url, provider, status,
                last_checked_at::text, last_healthy_at::text,
                consecutive_failures::bigint, added_by::text,
                array_to_json(verified_by)::text AS verified_by,
                array_to_json(reported_broken_by)::text AS reported_broken_by,
                priority::bigint, failure_details::text,
                created_at::text, updated_at::text
         FROM availability_links
         WHERE media_reference_id = $1::uuid
         ORDER BY priority DESC"
            .to_string(),
    );
    Ok(match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, AvailabilityLinkRow>(&sql)
                .bind(media_reference_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, AvailabilityLinkRow>(&sql)
                .bind(media_reference_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
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
         WHERE id = ?"
            .to_string(),
        "UPDATE availability_links
         SET status = $1, consecutive_failures = $2,
             last_checked_at = NOW(),
             updated_at = NOW()
         WHERE id = $3::uuid"
            .to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(status.as_str())
                .bind(consecutive_failures)
                .bind(link_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(status.as_str())
                .bind(consecutive_failures)
                .bind(link_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Work-media references
// ---------------------------------------------------------------------------
// DB functions take their parameters explicitly rather than a builder:
// a builder here would only move the same fields one call deeper.
#[allow(clippy::too_many_arguments)]
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
         VALUES (?, ?, ?, ?, ?, ?, ?)"
            .to_string(),
        // Four UUID columns.
        "INSERT INTO work_media_references
            (id, work_id, chapter_id, media_reference_id, context, display_url, author_note)
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, $5, $6, $7)"
            .to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(id)
                .bind(work_id)
                .bind(chapter_id)
                .bind(media_reference_id)
                .bind(context.as_str())
                .bind(display_url)
                .bind(author_note)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(id)
                .bind(work_id)
                .bind(chapter_id)
                .bind(media_reference_id)
                .bind(context.as_str())
                .bind(display_url)
                .bind(author_note)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
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
         WHERE work_id = ? AND deleted_at IS NULL"
            .to_string(),
        // `WorkMediaReference` is all String, so the three UUIDs and the two
        // TIMESTAMPTZs are cast here as well as in the SQLite arm above.
        "SELECT id::text, work_id::text, chapter_id::text, media_reference_id::text,
                context, display_url, author_note,
                inserted_at::text, deleted_at::text
         FROM work_media_references
         WHERE work_id = $1::uuid AND deleted_at IS NULL"
            .to_string(),
    );
    Ok(match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, WorkMediaReference>(&sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, WorkMediaReference>(&sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
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
         WHERE al.url = ? LIMIT 1"
            .to_string(),
        "SELECT mr.id::text
         FROM availability_links al
         JOIN media_references mr ON al.media_reference_id = mr.id
         WHERE al.url = $1 LIMIT 1"
            .to_string(),
    );
    let existing_id = match db.backend() {
        Backend::Sqlite => sqlx::query(&existing)
            .bind(url)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
            .map(|row| row.get::<String, _>("id")),
        Backend::Postgres => sqlx::query(&existing)
            .bind(url)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?
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
         VALUES (?, ?, ?, ?, ?, ?)"
            .to_string(),
        // Four uuid columns and an INTEGER `amount` that the bound value has
        // to be told about explicitly.
        "INSERT INTO curator_rewards
            (id, account_id, action, media_reference_id, availability_link_id, amount)
         VALUES ($1::uuid, $2::uuid, $3, $4::uuid, $5::uuid, $6)"
            .to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(Uuid::new_v4().to_string())
                .bind(account_id)
                .bind(action.as_str())
                .bind(media_reference_id)
                .bind(availability_link_id)
                .bind(amount)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(Uuid::new_v4().to_string())
                .bind(account_id)
                .bind(action.as_str())
                .bind(media_reference_id)
                .bind(availability_link_id)
                .bind(amount)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

pub async fn sum_curator_rewards_today(db: &Database, account_id: &str) -> Result<i64> {
    let sql = sql_owned(
        db,
        "SELECT COALESCE(SUM(amount), 0) AS total
         FROM curator_rewards
         WHERE account_id = ?
           AND created_at >= datetime('now', 'start of day')"
            .to_string(),
        // `account_id` is UUID and `SUM(integer)` is NUMERIC against an i64 row type.
        "SELECT COALESCE(SUM(amount), 0)::bigint AS total
         FROM curator_rewards
         WHERE account_id = $1::uuid
           AND created_at >= date_trunc('day', NOW())"
            .to_string(),
    );
    let row: (i64,) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.0)
}

// ---------------------------------------------------------------------------
// Query helpers for monitoring
// ---------------------------------------------------------------------------

pub async fn count_healthy_links(db: &Database, media_reference_id: &str) -> Result<i64> {
    let sql = sql_owned(
        db,
        "SELECT COUNT(*) AS cnt FROM availability_links
         WHERE media_reference_id = ? AND status = 'healthy'"
            .to_string(),
        "SELECT COUNT(*) AS cnt FROM availability_links
         WHERE media_reference_id = $1::uuid AND status = 'healthy'"
            .to_string(),
    );
    let row: (i64,) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(media_reference_id)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(media_reference_id)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
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
         LIMIT ?"
            .to_string(),
        // Same casts as the other availability_links select: see
        // find_availability_links_for_reference.
        "SELECT id::text, media_reference_id::text, url, provider, status,
                last_checked_at::text, last_healthy_at::text,
                consecutive_failures::bigint, added_by::text,
                array_to_json(verified_by)::text AS verified_by,
                array_to_json(reported_broken_by)::text AS reported_broken_by,
                priority::bigint, failure_details::text,
                created_at::text, updated_at::text
         FROM availability_links
         WHERE status IN ('healthy', 'degraded', 'pending_verification')
         ORDER BY last_checked_at ASC
         LIMIT $1"
            .to_string(),
    );
    Ok(match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, AvailabilityLinkRow>(&sql)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, AvailabilityLinkRow>(&sql)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
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
         LIMIT ?"
            .to_string(),
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
         LIMIT $2"
            .to_string(),
    );
    Ok(match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, MediaReference>(&sql)
                .bind(threshold)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, MediaReference>(&sql)
                .bind(threshold)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    })
}

// ---------------------------------------------------------------------------
// Phase 2 (§32.7.5): Curator role management
// ---------------------------------------------------------------------------

use lorehaven_domain::media_resilience::VerificationType;

/// Opt an account into the curator role.
pub async fn opt_in_curator(db: &Database, account_id: &str) -> Result<(), sqlx::Error> {
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
                // `account_id` and `opted_in_by` are TEXT here; the timestamp
                // is TIMESTAMPTZ and `now` is an RFC 3339 string.
                "INSERT INTO curator_roles (account_id, opted_in_at, opted_in_by)
                 VALUES ($1, $2::timestamptz, 'self')
                 ON CONFLICT(account_id)
                 DO UPDATE SET opted_in_at = $2::timestamptz, opted_out_at = NULL",
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
pub async fn opt_out_curator(db: &Database, account_id: &str) -> Result<(), sqlx::Error> {
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
            sqlx::query(
                "UPDATE curator_roles SET opted_out_at = $1::timestamptz WHERE account_id = $2",
            )
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
                // `works.id`, `works.owner_pseud_id` and
                // `work_contributors.pseud_id` are all UUID on PostgreSQL.
                "SELECT COUNT(*) FROM works w
                 WHERE w.id = $1::uuid AND (
                   w.owner_pseud_id = $2::uuid
                   OR EXISTS (
                     SELECT 1 FROM work_contributors c
                     WHERE c.work_id = w.id AND c.pseud_id = $3::uuid
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
                 WHERE media_reference_id = $1::uuid
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
pub async fn count_total_links(db: &Database, reference_id: &str) -> Result<i64, sqlx::Error> {
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
                "SELECT COUNT(*) FROM availability_links WHERE media_reference_id = $1::uuid",
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
                    id,
                    work_id,
                    chapter_id,
                    context,
                    display_url,
                    author_note,
                    inserted_at,
                    healthy,
                    total,
                    best,
                ));
            }
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let rows = sqlx::query(
                // Read into Strings: three UUIDs and a TIMESTAMPTZ.
                "SELECT id::text, work_id::text, chapter_id::text, context,
                        display_url, author_note, inserted_at::text
                 FROM work_media_references
                 WHERE work_id = $1::uuid AND deleted_at IS NULL
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
                    id,
                    work_id,
                    chapter_id,
                    context,
                    display_url,
                    author_note,
                    inserted_at,
                    healthy,
                    total,
                    best,
                ));
            }
        }
    }
    Ok(out)
}
// DB functions take their parameters explicitly rather than a builder:
// a builder here would only move the same fields one call deeper.

#[allow(clippy::too_many_arguments)]
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
            Ok(rows
                .iter()
                .map(|r| r.get::<String, _>("account_id"))
                .collect())
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let rows = sqlx::query(
                "SELECT account_id FROM curator_roles WHERE opted_out_at IS NULL ORDER BY opted_in_at",
            )
            .fetch_all(pool)
            .await?;
            Ok(rows
                .iter()
                .map(|r| r.get::<String, _>("account_id"))
                .collect())
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
                 VALUES ($1, $2, $3, $4, $5, $6, $7::timestamptz)
                 ON CONFLICT(availability_link_id, curator_id) DO UPDATE SET
                     verification_type = $5, confidence = $6,
                     created_at = $7::timestamptz",
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
pub async fn has_quorum(db: &Database, availability_link_id: &str) -> Result<bool, sqlx::Error> {
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

/// Find active standing bounties whose conditions the given state meets.
///
/// These are *standing* bounties, not per-reference ones: the table has no
/// media reference column, so every caller sees the same global set, filtered
/// only by health and archive state. An earlier signature took a
/// `media_reference_id` and never used it, so the doc comment promised a
/// per-reference match the query never performed.
pub async fn find_matching_standing_bounties(
    db: &Database,
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
            Ok(rows
                .iter()
                .map(|r| MatchedBounty {
                    bounty_id: r.get::<String, _>("id"),
                    name: r.get::<String, _>("name"),
                    reward: r.get::<i64, _>("reward"),
                    provider: r.get::<Option<String>, _>("provider"),
                    healthy_links_below: r.get::<Option<i64>, _>("healthy_links_below"),
                })
                .collect())
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let rows = sqlx::query(
                // `id` is UUID and the row reader takes Strings.
                "SELECT id::text, name, reward, provider, healthy_links_below
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
            Ok(rows
                .iter()
                .map(|r| MatchedBounty {
                    bounty_id: r.get::<String, _>("id"),
                    name: r.get::<String, _>("name"),
                    reward: r.get::<i64, _>("reward"),
                    provider: r.get::<Option<String>, _>("provider"),
                    healthy_links_below: r.get::<Option<i64>, _>("healthy_links_below"),
                })
                .collect())
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
                // `created_at`/`updated_at` are TIMESTAMPTZ and `now` is bound
                // as an RFC 3339 string; `account_id` is TEXT on this table and
                // needs nothing.
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
                 VALUES ($1, $2, $3, $4, $5, $6, $7::timestamptz, $7::timestamptz)
                 ON CONFLICT(account_id) DO UPDATE SET
                    auto_submit_to_archive = $2,
                    prefer_curator_verified = $3,
                    broken_link_notifications = $4,
                    allow_curator_edits = $5,
                    minimum_healthy_links = $6,
                    updated_at = $7::timestamptz",
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
                // The three flags are BOOLEAN on PostgreSQL (INTEGER on SQLite,
                // which is why the two arms read them differently), and
                // `minimum_healthy_links` is INTEGER against an i64 field.
                "SELECT account_id, auto_submit_to_archive, prefer_curator_verified,
                        broken_link_notifications, allow_curator_edits,
                        minimum_healthy_links::bigint
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
// DB functions take their parameters explicitly rather than a builder:
// a builder here would only move the same fields one call deeper.

/// Post a targeted bounty for a specific work/media reference.
#[allow(clippy::too_many_arguments)]
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
                // The five ids are TEXT on this table; the two timestamps are
                // TIMESTAMPTZ and `now` arrives as an RFC 3339 string.
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
                 VALUES ($1, $2, $3, $4, $5, $6, 'open', $7, $8::timestamptz, $8::timestamptz)",
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
                 SET status = 'claimed', claimed_by = $1,
                     claimed_at = $2::timestamptz, updated_at = $2::timestamptz
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
            Ok(rows
                .iter()
                .map(|r| TargetedBounty {
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
                })
                .collect())
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let rows = sqlx::query(
                // Read into String/i64: `reward` is INTEGER, so widen it, and
                // `created_at` is TIMESTAMPTZ, so render it as text.
                "SELECT id, work_id, chapter_id, media_reference_id, account_id,
                        reward::bigint, status, description, claimed_by,
                        created_at::text
                 FROM targeted_bounties WHERE work_id = $1 ORDER BY created_at DESC",
            )
            .bind(work_id)
            .fetch_all(pool)
            .await?;
            Ok(rows
                .iter()
                .map(|r| TargetedBounty {
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
                })
                .collect())
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
// DB functions take their parameters explicitly rather than a builder:
// a builder here would only move the same fields one call deeper.

/// Record a new local mirror.
#[allow(clippy::too_many_arguments)]
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
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'active', $9::timestamptz)",
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
            Ok(rows
                .iter()
                .map(|r| LocalMirror {
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
                })
                .collect())
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
            Ok(rows
                .iter()
                .map(|r| LocalMirror {
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
                })
                .collect())
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
            sqlx::query(
                "UPDATE local_mirrors SET status = 'removed', expires_at = $1::timestamptz WHERE id = $2",
            )
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
                 VALUES ($1, $2, $3, $4, 'pinned', $5, $6::timestamptz)",
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
            Ok(rows
                .iter()
                .map(|r| IpfsPin {
                    id: r.get::<String, _>("id"),
                    media_reference_id: r.get::<String, _>("media_reference_id"),
                    cid: r.get::<String, _>("cid"),
                    pin_service: r.get::<String, _>("pin_service"),
                    status: r.get::<String, _>("status"),
                    file_size_bytes: r.get::<i64, _>("file_size_bytes"),
                    pinned_at: r.get::<String, _>("pinned_at"),
                })
                .collect())
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
            Ok(rows
                .iter()
                .map(|r| IpfsPin {
                    id: r.get::<String, _>("id"),
                    media_reference_id: r.get::<String, _>("media_reference_id"),
                    cid: r.get::<String, _>("cid"),
                    pin_service: r.get::<String, _>("pin_service"),
                    status: r.get::<String, _>("status"),
                    file_size_bytes: r.get::<i64, _>("file_size_bytes"),
                    pinned_at: r.get::<String, _>("pinned_at"),
                })
                .collect())
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
                 VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7::timestamptz)",
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
                "UPDATE dmca_takedowns SET status = $1, resolved_at = $2::timestamptz, resolved_by = $3 WHERE id = $4",
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
// §32.7.2 Deduplication: the proposal half of a perceptual match
// ---------------------------------------------------------------------------
//
// The spec is explicit about the shape of a perceptual match: "present the
// curator with a match confidence score; they confirm or reject the linkage."
// Everything upstream of that sentence already existed -- `record_fingerprint`
// stores the dHash, `find_by_perceptual_hash` scores it, and
// `perceptual_match_confidence` turns a distance into the number the curator
// reads. What was missing is the row between them: without it a near-duplicate
// and a distinct image are indistinguishable after the fetch, because the only
// record of "these two looked alike" is the fingerprint itself.
//
// So this section is deliberately small. It does not re-implement the search or
// the scoring, and it does not try to decide whether two images are the same --
// pHash agrees across re-encodes and disagrees across distinct images that
// happen to share structure, which is exactly why the spec routes this through
// a human.

/// A proposed linkage between a freshly fetched reference and an existing one.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchProposal {
    pub id: String,
    /// The reference that was just fetched and hashed.
    pub candidate_reference_id: String,
    /// The existing reference the candidate appears to duplicate.
    pub existing_reference_id: String,
    pub content_hash: String,
    pub perceptual_hash: Option<String>,
    /// The Hamming distance the search matched on.
    pub hamming_distance: i32,
    /// `perceptual_match_confidence(hamming_distance)`, stored rather than
    /// recomputed: the curator sees the number the search actually used, and a
    /// later threshold change must not retroactively rewrite the history of what
    /// they were shown.
    pub match_confidence: f64,
    /// `pending` | `confirmed` | `rejected`.
    pub status: String,
    pub resolved_by: Option<String>,
    pub resolution_note: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub resolved_at: Option<String>,
}

/// Proposals a curator still has to act on, best candidate first.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingProposal {
    pub proposal: MatchProposal,
    /// The existing reference's title-less description for the curator's list:
    /// its content hash, so the curator can tell two proposals apart when the
    /// images are not rendered.
    pub existing_content_hash: String,
}

/// `Some` when the id is well formed, so a caller can reject a bad id before
/// spending a round trip.
///
/// A match proposal id is a UUID, because both dialect schemas declare the
/// column that way and SQLite will store the string regardless -- which means
/// SQLite accepts a non-UUID here and PostgreSQL answers 22P02. Validating in
/// the domain keeps the route's error a 400 instead of a 500 on one backend.
pub fn match_proposal_id_is_valid(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok()
}

/// Record a proposed linkage. Idempotent per `(candidate, existing)` pair.
///
/// The pair is unique in both schemas, so re-running a fetch that finds the
/// same near-match updates the pending row rather than accumulating duplicates
/// -- which is what makes the `UNIQUE` constraint load-bearing rather than
/// decorative. A pair a curator already *rejected* is left alone: re-proposing
/// it on every fetch would be a curator-approval flow that never terminates,
/// and the row is kept precisely so the rejection is remembered.
///
/// # Errors
/// An error when no such reference exists, which is what a fetch completing
/// against a deleted row looks like. Returning `Ok(false)` for that would let
/// the caller record the media as mirrored.
pub async fn record_match_proposal(
    db: &Database,
    candidate_reference_id: &str,
    existing_reference_id: &str,
    content_hash: &str,
    perceptual_hash: Option<&str>,
    hamming_distance: u32,
) -> Result<bool> {
    // A distance wider than the score can express has no confidence worth
    // storing, and `i32` is what the column is declared as.
    let Ok(distance) = i32::try_from(hamming_distance) else {
        return Ok(false);
    };
    let confidence =
        lorehaven_domain::media_resilience::perceptual_match_confidence(hamming_distance);
    let now = crate::identity::now_rfc3339();

    let sql = sql_owned(
        db,
        "INSERT INTO media_match_proposals
             (id, candidate_reference_id, existing_reference_id, content_hash,
              perceptual_hash, hamming_distance, match_confidence, status,
              created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?)
         ON CONFLICT (candidate_reference_id, existing_reference_id) DO UPDATE
            SET hamming_distance = excluded.hamming_distance,
                match_confidence = excluded.match_confidence,
                perceptual_hash = excluded.perceptual_hash,
                content_hash = excluded.content_hash,
                updated_at = excluded.updated_at
         WHERE media_match_proposals.status = 'pending'"
            .to_string(),
        "INSERT INTO media_match_proposals
             (id, candidate_reference_id, existing_reference_id, content_hash,
              perceptual_hash, hamming_distance, match_confidence, status,
              created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $6, $7, 'pending',
                 $8::timestamptz, $9::timestamptz)
         ON CONFLICT (candidate_reference_id, existing_reference_id) DO UPDATE
            SET hamming_distance = excluded.hamming_distance,
                match_confidence = excluded.match_confidence,
                perceptual_hash = excluded.perceptual_hash,
                content_hash = excluded.content_hash,
                updated_at = excluded.updated_at
         WHERE media_match_proposals.status = 'pending'"
            .to_string(),
    );
    let id = Uuid::new_v4().to_string();
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&id)
            .bind(candidate_reference_id)
            .bind(existing_reference_id)
            .bind(content_hash)
            .bind(perceptual_hash)
            .bind(distance)
            .bind(confidence)
            .bind(&now)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&id)
            .bind(candidate_reference_id)
            .bind(existing_reference_id)
            .bind(content_hash)
            .bind(perceptual_hash)
            .bind(distance)
            .bind(confidence)
            .bind(&now)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    // 0 rows means one of the two references does not exist, or the pair was
    // already rejected. Both are legitimate "nothing to do"; neither is an error,
    // and reporting a failure for a remembered rejection would make every
    // subsequent fetch of a known-bad pair look broken.
    Ok(affected > 0)
}

/// The §32.7.2 exact-match branch: attach a reference's availability links to
/// the reference that already holds the same bytes, and drop the duplicate.
///
/// This is the automatic half of deduplication, and the spec is unambiguous that
/// it needs no human: "Exact match (content_hash identical): Attach as a new
/// AvailabilityLink to the existing MediaReference. Zero new storage cost."
///
/// The same merge `resolve_match_proposal` performs, factored out because the two
/// callers reach it by different routes -- one from a curator's click, one from
/// the fetch job -- and two copies of a five-statement move would drift.
///
/// Returns `false` when the ids name the same reference or either is missing,
/// which is what a re-fetch looks like.
pub async fn attach_reference_to_existing(
    db: &Database,
    existing_reference_id: &str,
    candidate_reference_id: &str,
) -> Result<bool, sqlx::Error> {
    if existing_reference_id == candidate_reference_id
        || !match_proposal_id_is_valid(existing_reference_id)
        || !match_proposal_id_is_valid(candidate_reference_id)
    {
        return Ok(false);
    }
    // Both references must exist. Checking first turns a partial move -- links
    // repointed at nothing -- into a refusal.
    for id in [existing_reference_id, candidate_reference_id] {
        let found = match db.backend() {
            Backend::Sqlite => sqlx::query("SELECT id FROM media_references WHERE id = ?")
                .bind(id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
                .is_some(),
            Backend::Postgres => {
                sqlx::query("SELECT id::text FROM media_references WHERE id = $1::uuid")
                    .bind(id)
                    .fetch_optional(db.postgres_pool().expect("postgres"))
                    .await?
                    .is_some()
            }
        };
        if !found {
            return Ok(false);
        }
    }
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            // `OR IGNORE` / `OR REPLACE` because a URL already attached to the
            // surviving reference must not collide with the one being moved --
            // the common case, since the exact-match branch runs *because* the
            // bytes matched and the first link is often the same URL.
            sqlx::query(
                "UPDATE OR IGNORE work_media_references
                    SET media_reference_id = ?
                  WHERE media_reference_id = ?",
            )
            .bind(existing_reference_id)
            .bind(candidate_reference_id)
            .execute(pool)
            .await?;
            sqlx::query(
                "UPDATE OR REPLACE availability_links
                    SET media_reference_id = ?
                  WHERE media_reference_id = ?",
            )
            .bind(existing_reference_id)
            .bind(candidate_reference_id)
            .execute(pool)
            .await?;
            // Proposals naming this reference as a candidate are repointed at
            // the survivor rather than left to a foreign-key violation, and
            // proposals naming it as the existing side are dropped along with
            // the reference by the cascade.
            sqlx::query(
                "UPDATE OR REPLACE media_match_proposals
                    SET candidate_reference_id = ?
                  WHERE candidate_reference_id = ?",
            )
            .bind(existing_reference_id)
            .bind(candidate_reference_id)
            .execute(pool)
            .await?;
            sqlx::query("DELETE FROM media_references WHERE id = ?")
                .bind(candidate_reference_id)
                .execute(pool)
                .await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query(
                "UPDATE work_media_references
                    SET media_reference_id = $1::uuid
                  WHERE media_reference_id = $2::uuid",
            )
            .bind(existing_reference_id)
            .bind(candidate_reference_id)
            .execute(pool)
            .await?;
            sqlx::query(
                "UPDATE availability_links
                    SET media_reference_id = $1::uuid
                  WHERE media_reference_id = $2::uuid",
            )
            .bind(existing_reference_id)
            .bind(candidate_reference_id)
            .execute(pool)
            .await?;
            sqlx::query(
                "UPDATE media_match_proposals
                    SET candidate_reference_id = $1::uuid
                  WHERE candidate_reference_id = $2::uuid",
            )
            .bind(existing_reference_id)
            .bind(candidate_reference_id)
            .execute(pool)
            .await?;
            sqlx::query("DELETE FROM media_references WHERE id = $1::uuid")
                .bind(candidate_reference_id)
                .execute(pool)
                .await?;
        }
    }
    Ok(true)
}

/// A curator's decision on a proposal.
///
/// `Confirm` merges the candidate into the existing reference; `Reject` keeps
/// them apart. The enum rather than a bool because `resolve_match_proposal`
/// also acts on the confirmation, and a bare `true` there would read as
/// "resolved?" rather than "same image?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalDecision {
    Confirm,
    Reject,
}

impl ProposalDecision {
    /// The value the `status` CHECK constraint allows.
    pub fn as_status(self) -> &'static str {
        match self {
            ProposalDecision::Confirm => "confirmed",
            ProposalDecision::Reject => "rejected",
        }
    }

    pub fn is_confirmation(self) -> bool {
        matches!(self, ProposalDecision::Confirm)
    }
}

/// Resolve a proposal.
///
/// A confirmation is more than a status change: the candidate reference's
/// availability links move to the existing one, so the work that pointed at the
/// candidate is now served by the reference that has every other copy of the
/// image. That is the whole point of deduplicating -- "one dying link doesn't
/// affect the others" -- and doing it here rather than in the route keeps the
/// move in the same transaction as the decision.
///
/// A rejection moves nothing.
///
/// Returns `false` when there is no pending proposal with that id, which covers
/// a missing row, an already-resolved one, and a malformed id. The caller
/// answers 404 for all three rather than distinguishing them, which is right:
/// a curator cannot act on a proposal that is not there, and telling them it
/// was already decided leaks whether someone else got there first.
pub async fn resolve_match_proposal(
    db: &Database,
    proposal_id: &str,
    decision: ProposalDecision,
    resolved_by: &str,
    note: Option<&str>,
) -> Result<bool, sqlx::Error> {
    // PostgreSQL raises 22P02 on a malformed id because the placeholder is cast
    // to uuid, and SQLite happily matches nothing. Returning early is what makes
    // the two agree, and it means a caller cannot turn a bad path parameter into
    // a 500 by forgetting to validate it.
    if !match_proposal_id_is_valid(proposal_id) {
        return Ok(false);
    }
    let now = crate::identity::now_rfc3339();
    let status = decision.as_status();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            // The status guard is what makes this safe against two curators
            // clicking at once: the second UPDATE matches no row and the link
            // move never happens. Doing it in one statement keeps that true even
            // without an explicit transaction.
            let changed = sqlx::query(
                "UPDATE media_match_proposals
                    SET status = ?, resolved_by = ?, resolution_note = ?,
                        resolved_at = ?, updated_at = ?
                  WHERE id = ? AND status = 'pending'",
            )
            .bind(status)
            .bind(resolved_by)
            .bind(note)
            .bind(&now)
            .bind(&now)
            .bind(proposal_id)
            .execute(pool)
            .await?
            .rows_affected();
            if changed == 0 {
                return Ok(false);
            }
            if decision.is_confirmation() {
                move_links_to_existing(db, proposal_id).await?;
            }
            Ok(true)
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let changed = sqlx::query(
                "UPDATE media_match_proposals
                    SET status = $1, resolved_by = $2, resolution_note = $3,
                        resolved_at = $4::timestamptz, updated_at = $5::timestamptz
                  WHERE id = $6::uuid AND status = 'pending'",
            )
            .bind(status)
            .bind(resolved_by)
            .bind(note)
            .bind(&now)
            .bind(&now)
            .bind(proposal_id)
            .execute(pool)
            .await?
            .rows_affected();
            if changed == 0 {
                return Ok(false);
            }
            if decision.is_confirmation() {
                move_links_to_existing(db, proposal_id).await?;
            }
            Ok(true)
        }
    }
}

/// Point every availability link at the existing reference, and drop the now
/// duplicate candidate row.
///
/// The `ON CONFLICT DO UPDATE` is the point: two works can have attached the
/// same URL to the same candidate, and moving both must not fail on the unique
/// `(media_reference_id, url)`. A link that already exists on the target -- the
/// exact-match branch attaches one, so this is the common case -- is updated
/// rather than inserted twice.
///
/// `ON DELETE CASCADE` then removes the candidate reference itself, and with it
/// any `work_media_references` rows pointing at it, which is why the work
/// associations are repointed first.
/// Resolve a proposal id to the pair of references it names, then perform the
/// merge. A thin wrapper so the fetch job's exact-match branch and the curator's
/// confirm share one implementation of the move.
async fn move_links_to_existing(db: &Database, proposal_id: &str) -> Result<(), sqlx::Error> {
    // Fetch, then map, per backend: the two arms return `SqliteRow` and `PgRow`,
    // which have no common type, and `sqlx::any` is not enabled here. Building the
    // tuple inside each arm after the `?` would still unify the two row types.
    let pair: Option<(String, String)> = match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(
                "SELECT candidate_reference_id, existing_reference_id
                   FROM media_match_proposals WHERE id = ?",
            )
            .bind(proposal_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?;
            row.map(|r| {
                (
                    r.get::<String, _>("candidate_reference_id"),
                    r.get::<String, _>("existing_reference_id"),
                )
            })
        }
        Backend::Postgres => {
            let row = sqlx::query(
                "SELECT candidate_reference_id::text, existing_reference_id::text
                   FROM media_match_proposals WHERE id = $1::uuid",
            )
            .bind(proposal_id)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?;
            row.map(|r| {
                (
                    r.get::<String, _>("candidate_reference_id"),
                    r.get::<String, _>("existing_reference_id"),
                )
            })
        }
    };
    let Some((candidate, existing)) = pair else {
        return Err(sqlx::Error::RowNotFound);
    };
    attach_reference_to_existing(db, &existing, &candidate).await?;
    Ok(())
}

/// The proposals a curator still has to act on.
///
/// Ordered by confidence descending, then by id, so two equally-confident
/// proposals have a stable order across calls and a paginating caller cannot
/// loop. `LIMIT` is required: a fetch storm on one popular image produces a
/// proposal per importing work, and a curator queue that grows without bound is
/// the failure mode this function exists to avoid.
pub async fn list_pending_match_proposals(
    db: &Database,
    limit: i64,
) -> Result<Vec<PendingProposal>, sqlx::Error> {
    // A negative limit is a caller's bug, not a query; `LIMIT -1` means "no
    // limit" in both dialects, which is the opposite of what was asked for.
    let limit = limit.clamp(1, 200);
    // Fetch per backend, map in each arm, push into one `out`. A shared row
    // mapper would need `sqlx::any::AnyRow`, and the `any` feature is not
    // enabled in this workspace -- the same reason the neighbouring readers are
    // written this way.
    let mut out: Vec<PendingProposal> = Vec::new();
    match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(
                "SELECT p.id, p.candidate_reference_id, p.existing_reference_id,
                        p.content_hash, p.perceptual_hash, p.hamming_distance,
                        p.match_confidence, p.status, p.resolved_by,
                        p.resolution_note, p.created_at, p.updated_at,
                        p.resolved_at, mr.content_hash AS existing_content_hash
                   FROM media_match_proposals p
                   JOIN media_references mr ON mr.id = p.existing_reference_id
                  WHERE p.status = 'pending'
                  ORDER BY p.match_confidence DESC, p.id
                  LIMIT ?",
            )
            .bind(limit)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await?;
            for row in rows {
                let id: String = row.get("id");
                let candidate_reference_id: String = row.get("candidate_reference_id");
                let existing_reference_id: String = row.get("existing_reference_id");
                let content_hash: String = row.get("content_hash");
                let perceptual_hash: Option<String> = row.get("perceptual_hash");
                let hamming_distance: i32 = row.get("hamming_distance");
                let match_confidence: f64 = row.get("match_confidence");
                let status: String = row.get("status");
                let resolved_by: Option<String> = row.get("resolved_by");
                let resolution_note: Option<String> = row.get("resolution_note");
                let created_at: String = row.get("created_at");
                let updated_at: String = row.get("updated_at");
                let resolved_at: Option<String> = row.get("resolved_at");
                let existing_content_hash: String = row.get("existing_content_hash");
                out.push(PendingProposal {
                    proposal: MatchProposal {
                        id,
                        candidate_reference_id,
                        existing_reference_id,
                        content_hash,
                        perceptual_hash,
                        hamming_distance,
                        match_confidence,
                        status,
                        resolved_by,
                        resolution_note,
                        created_at,
                        updated_at,
                        resolved_at,
                    },
                    existing_content_hash,
                });
            }
        }
        Backend::Postgres => {
            // Read by name into Strings, so every column PostgreSQL does not
            // already store as text is cast: the ids are UUID, the timestamps
            // TIMESTAMPTZ, `hamming_distance` INT4 and `match_confidence` REAL.
            let rows = sqlx::query(
                "SELECT p.id::text, p.candidate_reference_id::text,
                        p.existing_reference_id::text, p.content_hash,
                        p.perceptual_hash, p.hamming_distance::int,
                        p.match_confidence::float8, p.status, p.resolved_by,
                        p.resolution_note, p.created_at::text,
                        p.updated_at::text, p.resolved_at::text,
                        mr.content_hash AS existing_content_hash
                   FROM media_match_proposals p
                   JOIN media_references mr ON mr.id = p.existing_reference_id
                  WHERE p.status = 'pending'
                  ORDER BY p.match_confidence DESC, p.id
                  LIMIT $1",
            )
            .bind(limit)
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await?;
            for row in rows {
                let id: String = row.get("id");
                let candidate_reference_id: String = row.get("candidate_reference_id");
                let existing_reference_id: String = row.get("existing_reference_id");
                let content_hash: String = row.get("content_hash");
                let perceptual_hash: Option<String> = row.get("perceptual_hash");
                let hamming_distance: i32 = row.get("hamming_distance");
                let match_confidence: f64 = row.get("match_confidence");
                let status: String = row.get("status");
                let resolved_by: Option<String> = row.get("resolved_by");
                let resolution_note: Option<String> = row.get("resolution_note");
                let created_at: String = row.get("created_at");
                let updated_at: String = row.get("updated_at");
                let resolved_at: Option<String> = row.get("resolved_at");
                let existing_content_hash: String = row.get("existing_content_hash");
                out.push(PendingProposal {
                    proposal: MatchProposal {
                        id,
                        candidate_reference_id,
                        existing_reference_id,
                        content_hash,
                        perceptual_hash,
                        hamming_distance,
                        match_confidence,
                        status,
                        resolved_by,
                        resolution_note,
                        created_at,
                        updated_at,
                        resolved_at,
                    },
                    existing_content_hash,
                });
            }
        }
    }
    Ok(out)
}

/// Read one pending proposal's distance and confidence, for the auto-attach
/// decision the spec's `require_curator_confirmation_above` config drives.
///
/// `None` when there is no such pending proposal.
pub async fn find_match_proposal(
    db: &Database,
    proposal_id: &str,
) -> Result<Option<MatchProposal>, sqlx::Error> {
    // Same reason as `resolve_match_proposal`: `$1::uuid` raises on a malformed
    // id while SQLite matches nothing, and the two must answer the same way.
    if !match_proposal_id_is_valid(proposal_id) {
        return Ok(None);
    }
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(
                "SELECT id, candidate_reference_id, existing_reference_id, content_hash,
                        perceptual_hash, hamming_distance, match_confidence, status,
                        resolved_by, resolution_note, created_at, updated_at, resolved_at
                   FROM media_match_proposals WHERE id = ?",
            )
            .bind(proposal_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?;
            Ok(row.map(|r| MatchProposal {
                id: r.get("id"),
                candidate_reference_id: r.get("candidate_reference_id"),
                existing_reference_id: r.get("existing_reference_id"),
                content_hash: r.get("content_hash"),
                perceptual_hash: r.get("perceptual_hash"),
                hamming_distance: r.get("hamming_distance"),
                match_confidence: r.get("match_confidence"),
                status: r.get("status"),
                resolved_by: r.get("resolved_by"),
                resolution_note: r.get("resolution_note"),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
                resolved_at: r.get("resolved_at"),
            }))
        }
        Backend::Postgres => {
            let row = sqlx::query(
                "SELECT id::text, candidate_reference_id::text,
                        existing_reference_id::text, content_hash, perceptual_hash,
                        hamming_distance::int, match_confidence::float8, status,
                        resolved_by, resolution_note, created_at::text,
                        updated_at::text, resolved_at::text
                   FROM media_match_proposals WHERE id = $1::uuid",
            )
            .bind(proposal_id)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?;
            Ok(row.map(|r| MatchProposal {
                id: r.get("id"),
                candidate_reference_id: r.get("candidate_reference_id"),
                existing_reference_id: r.get("existing_reference_id"),
                content_hash: r.get("content_hash"),
                perceptual_hash: r.get("perceptual_hash"),
                hamming_distance: r.get("hamming_distance"),
                match_confidence: r.get("match_confidence"),
                status: r.get("status"),
                resolved_by: r.get("resolved_by"),
                resolution_note: r.get("resolution_note"),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
                resolved_at: r.get("resolved_at"),
            }))
        }
    }
}

/// How many proposals are waiting. Distinct from `list_pending` because a count
/// does not have to be bounded, and an unbounded count is what tells a curator
/// the queue is growing.
pub async fn count_pending_match_proposals(db: &Database) -> Result<i64, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(
                "SELECT COUNT(*) AS n FROM media_match_proposals WHERE status = 'pending'",
            )
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await?;
            Ok(row.get::<i64, _>("n"))
        }
        Backend::Postgres => {
            let row = sqlx::query(
                "SELECT COUNT(*)::bigint AS n
                   FROM media_match_proposals WHERE status = 'pending'",
            )
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await?;
            Ok(row.get::<i64, _>("n"))
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 5 (§32.7.3 & §32.7.7): Reverse search & discovery
// ---------------------------------------------------------------------------

/// Find media references by perceptual hash (dedup & reverse lookup).
///
/// Find the media references whose perceptual hash is within `max_distance` bits
/// of `hash` (spec §32.7.2), ordered closest first.
///
/// This is a Hamming-distance search, not an equality check: two images that
/// differ by a re-encode, a resize or a mild edit produce different bytes and
/// nearly the same fingerprint, and deduplicating them is the point. Exact
/// matches are distance 0 and therefore always found.
///
/// The comparison is application-side, not in SQL. Neither backend can compute
/// a Hamming distance on a hex string without a dialect-specific extension, and
/// a query that half-works on one backend is worse than one that works on both.
/// The candidate set is narrowed in SQL by `perceptual_hash IS NOT NULL`, which
/// is also the one predicate worth an index, and the distance is then computed
/// in Rust through `domain::media_resilience::hamming_distance`.
///
/// Three cases deliberately return nothing rather than guessing:
///
/// - a `NULL` perceptual_hash, which is a reference whose bytes were never
///   fetched — not a hash to compare;
/// - a stored or query hash that is not hex, which is not a fingerprint;
/// - hashes of different digit counts, since a 64-bit pHash and a 128-bit wHash
///   have no distance between them.
///
/// A malformed row is skipped and the search continues, so one bad value cannot
/// hide every good match. `max_distance` is honoured: it is the threshold the
/// caller configured, and a result here genuinely means "within N bits".
pub async fn find_by_perceptual_hash(
    db: &Database,
    hash: &str,
    max_distance: i32,
) -> Result<Vec<MediaReference>, sqlx::Error> {
    // An unparseable query hash has no comparable candidate, and a negative
    // threshold admits nothing. Both are answered without touching the table.
    if hash.is_empty() || max_distance < 0 {
        return Ok(Vec::new());
    }
    if lorehaven_domain::media_resilience::hamming_distance(hash, hash).is_none() {
        return Ok(Vec::new());
    }
    let threshold = u32::try_from(max_distance).unwrap_or(u32::MAX);

    let rows = sql_owned(
        db,
        "SELECT id, perceptual_hash, content_hash, media_kind, first_seen_at,
                width, height, duration_seconds, format, file_size_bytes,
                content_notes, curator_verified, created_at, updated_at
         FROM media_references
         WHERE perceptual_hash IS NOT NULL"
            .to_string(),
        // The row reader takes Strings, so every column PostgreSQL does not
        // already store as text has to be cast: `id` is UUID, and
        // `first_seen_at`/`created_at`/`updated_at` are TIMESTAMPTZ. The SQLite
        // arm needs none of this and stays as written.
        "SELECT id::text, perceptual_hash, content_hash, media_kind,
                first_seen_at::text, width::bigint, height::bigint,
                duration_seconds::bigint, format,
                file_size_bytes::bigint, content_notes::text, curator_verified,
                created_at::text, updated_at::text
         FROM media_references
         WHERE perceptual_hash IS NOT NULL"
            .to_string(),
    );
    // `sqlx::any::AnyRow` is unavailable in this workspace and the two backends
    // return different concrete row types, so each arm reads its own columns
    // and hands plain values to one shared scoring path. The `media_references`
    // column list is repeated per arm rather than shared as a constant because
    // the row types cannot be shared, and a single wrong column name here fails
    // at runtime with a 500, not at compile time.
    let candidates: Vec<(String, MediaReference)> = match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(&rows)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            rows.iter()
                .filter_map(|r| {
                    let stored = r.get::<Option<String>, _>("perceptual_hash")?;
                    Some((
                        stored.clone(),
                        MediaReference {
                            id: r.get::<String, _>("id"),
                            perceptual_hash: Some(stored),
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
                        },
                    ))
                })
                .collect()
        }
        Backend::Postgres => {
            let rows = sqlx::query(&rows)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            rows.iter()
                .filter_map(|r| {
                    let stored = r.get::<Option<String>, _>("perceptual_hash")?;
                    Some((
                        stored.clone(),
                        MediaReference {
                            id: r.get::<String, _>("id"),
                            perceptual_hash: Some(stored),
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
                        },
                    ))
                })
                .collect()
        }
    };

    // Score in one place: a malformed or absent hash is skipped, not folded
    // into a large distance, and one bad row cannot hide every good match.
    let mut scored: Vec<(u32, MediaReference)> = candidates
        .into_iter()
        .filter_map(|(stored, reference)| {
            let distance = lorehaven_domain::media_resilience::hamming_distance(hash, &stored)?;
            lorehaven_domain::media_resilience::is_within_perceptual_threshold(distance, threshold)
                .then_some((distance, reference))
        })
        .collect();

    // Closest first, then by id so two equally-close candidates have a stable
    // order across calls and a paginating caller cannot loop.
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
    Ok(scored.into_iter().map(|(_, reference)| reference).collect())
}

/// §32.7.3: find all works that reference a given media reference ID.
/// Returns (work_id, work_title, display_url) tuples.
pub async fn find_works_by_media_reference(
    db: &Database,
    media_reference_id: &str,
) -> Result<Vec<(String, String, Option<String>)>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let rows = sqlx::query(
                "SELECT w.id AS work_id, w.title AS work_title, wmr.display_url
                 FROM work_media_references wmr
                 JOIN works w ON w.id = wmr.work_id
                 WHERE wmr.media_reference_id = ?
                   AND wmr.deleted_at IS NULL
                 ORDER BY w.title",
            )
            .bind(media_reference_id)
            .fetch_all(pool)
            .await?;
            Ok(rows
                .iter()
                .map(|r| {
                    (
                        r.get::<String, _>("work_id"),
                        r.get::<String, _>("work_title"),
                        r.get::<Option<String>, _>("display_url"),
                    )
                })
                .collect())
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let rows = sqlx::query(
                // `work_id` is read into a String, so cast it to text; the
                // bound `media_reference_id` is UUID.
                "SELECT w.id::text AS work_id, w.title AS work_title, wmr.display_url
                 FROM work_media_references wmr
                 JOIN works w ON w.id = wmr.work_id
                 WHERE wmr.media_reference_id = $1::uuid
                   AND wmr.deleted_at IS NULL
                 ORDER BY w.title",
            )
            .bind(media_reference_id)
            .fetch_all(pool)
            .await?;
            Ok(rows
                .iter()
                .map(|r| {
                    (
                        r.get::<String, _>("work_id"),
                        r.get::<String, _>("work_title"),
                        r.get::<Option<String>, _>("display_url"),
                    )
                })
                .collect())
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
            Ok(rows
                .iter()
                .map(|r| MediaReference {
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
                })
                .collect())
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
            Ok(rows
                .iter()
                .map(|r| MediaReference {
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
                })
                .collect())
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
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(min_healthy)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(min_healthy)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
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
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.0)
}

/// Link rot: for each provider, how many links went from healthy to non-healthy
/// since the given datetime (ISO-8601 string). Returns Vec<(provider, rot_count)>.
pub async fn link_rot_by_provider(db: &Database, since: &str) -> Result<Vec<(String, i64)>> {
    let sql = sql_owned(
        db,
        "SELECT provider, COUNT(*) AS cnt FROM availability_links
         WHERE updated_at > ? AND status != 'healthy' AND last_healthy_at IS NOT NULL
         AND last_healthy_at < updated_at
         GROUP BY provider ORDER BY cnt DESC"
            .to_string(),
        "SELECT provider, COUNT(*) AS cnt FROM availability_links
         WHERE updated_at > $1::timestamptz AND status != 'healthy'
           AND last_healthy_at IS NOT NULL
         AND last_healthy_at < updated_at
         GROUP BY provider ORDER BY cnt DESC"
            .to_string(),
    );
    let rows: Vec<(String, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(since)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(since)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
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
         ) < ?"
            .to_string(),
        "SELECT COUNT(*) AS cnt FROM media_references m
         WHERE (
            SELECT COUNT(*) FROM availability_links al
            WHERE al.media_reference_id = m.id AND al.status = 'healthy'
         ) < $1"
            .to_string(),
    );
    let row: (i64,) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(min_healthy)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(min_healthy)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.0)
}

/// Curator leaderboard: top curators by reward count, with their total amount.
/// Returns Vec<(account_id, reward_count, total_amount)>.
pub async fn curator_leaderboard(db: &Database, limit: i64) -> Result<Vec<(String, i64, i64)>> {
    let sql = sql_owned(
        db,
        "SELECT account_id,
                COUNT(*) AS reward_count,
                SUM(amount) AS total_amount
         FROM curator_rewards
         GROUP BY account_id
         ORDER BY total_amount DESC
         LIMIT ?"
            .to_string(),
        "SELECT account_id,
                COUNT(*) AS reward_count,
                SUM(amount) AS total_amount
         FROM curator_rewards
         GROUP BY account_id
         ORDER BY total_amount DESC
         LIMIT $1"
            .to_string(),
    );
    let rows: Vec<(String, i64, i64)> = match db.backend() {
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
    Ok(rows)
}

/// Count of active standing bounties and total amount available.
pub async fn standing_bounty_status(db: &Database) -> Result<(i64, i64)> {
    let sql = sql_owned(
        db,
        "SELECT COUNT(*) AS cnt, COALESCE(SUM(reward), 0) AS total
         FROM targeted_bounties WHERE status = 'active'"
            .to_string(),
        "SELECT COUNT(*) AS cnt, COALESCE(SUM(reward), 0) AS total
         FROM targeted_bounties WHERE status = 'active'"
            .to_string(),
    );
    let row: (i64, i64) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row)
}

/// Local mirror storage consumed: total file size in bytes and count of mirrors.
pub async fn local_mirror_storage(db: &Database) -> Result<(i64, i64)> {
    let sql = sql_owned(
        db,
        "SELECT COUNT(*) AS cnt, COALESCE(SUM(file_size_bytes), 0) AS total_bytes
         FROM local_mirrors"
            .to_string(),
        "SELECT COUNT(*) AS cnt, COALESCE(SUM(file_size_bytes), 0) AS total_bytes
         FROM local_mirrors"
            .to_string(),
    );
    let row: (i64, i64) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
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
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?
        }
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
         ORDER BY (healthy * 1.0 / total) DESC"
            .to_string(),
        "SELECT provider,
                SUM(CASE WHEN status = 'healthy' THEN 1 ELSE 0 END) AS healthy,
                COUNT(*) AS total
         FROM availability_links
         GROUP BY provider
         ORDER BY (healthy::float / total) DESC"
            .to_string(),
    );
    let rows: Vec<(String, i64, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
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
         WHERE w.owner_pseud_id IN (SELECT p.id FROM pseuds p WHERE p.account_id = ?)
         GROUP BY w.id, w.title
         ORDER BY w.title"
            .to_string(),
        // `work_id` is read into a String, so cast it to text. The three SUMs
        // are NUMERIC on PostgreSQL, and the row type is i64, so cast them too.
        "SELECT w.id::text AS work_id,
                w.title AS work_title,
                COUNT(wmr.id) AS total_references,
                SUM(CASE WHEN COALESCE(healthy.cnt, 0) >= 3 THEN 1 ELSE 0 END)::bigint AS healthy_references,
                SUM(CASE WHEN COALESCE(healthy.cnt, 0) BETWEEN 1 AND 2 THEN 1 ELSE 0 END)::bigint AS at_risk_references,
                SUM(CASE WHEN COALESCE(healthy.cnt, 0) = 0 THEN 1 ELSE 0 END)::bigint AS broken_references
         FROM works w
         JOIN work_media_references wmr ON wmr.work_id = w.id AND wmr.deleted_at IS NULL
         LEFT JOIN (
             SELECT al.media_reference_id, COUNT(al.id) AS cnt
             FROM availability_links al
             WHERE al.status = 'healthy'
             GROUP BY al.media_reference_id
         ) healthy ON healthy.media_reference_id = wmr.media_reference_id
         WHERE w.owner_pseud_id IN (SELECT p.id FROM pseuds p WHERE p.account_id = $1::uuid)
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
