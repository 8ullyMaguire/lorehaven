//! Create media edition (M26 / spec §32.5) and supporting narration logic.

use anyhow::Result;

use crate::{sql_owned, Backend, Database};

/// Create a `narration` edition record for a work.
///
/// The edition is created in `draft` state (no published_at) until the
/// author approves it. The label names the machine producer per §22.6/§30.8.
pub async fn create_narration_edition(
    db: &Database,
    work_id: &str,
    label: &str,
    parent_edition_id: Option<&str>,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = sql_owned(
        db,
        "INSERT INTO media_editions (id, work_id, edition_kind, label, parent_edition_id, created_at, updated_at, version)
         VALUES (?, ?, 'narration', ?, ?, ?, ?, 1)"
            .to_string(),
        "INSERT INTO media_editions (id, work_id, edition_kind, label, parent_edition_id, created_at, updated_at, version)
         VALUES (?::uuid, ?::uuid, 'narration', ?, ?::uuid, ?, ?, 1)"
            .to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            let _ = sqlx::query(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(label)
                .bind(parent_edition_id)
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            let _ = sqlx::query(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(label)
                .bind(parent_edition_id)
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    };
    Ok(id)
}

/// Record that a narration edition's audio has been stored.
pub async fn mark_narration_audio_stored(
    db: &Database,
    edition_id: &str,
    audio_checksum: &str,
) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    let sql = sql_owned(
        db,
        "UPDATE media_editions SET audio_checksum = ?, updated_at = ? WHERE id = ?".to_string(),
        "UPDATE media_editions SET audio_checksum = ?, updated_at = ? WHERE id = ?::uuid"
            .to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            let _ = sqlx::query(&sql)
                .bind(audio_checksum)
                .bind(&now)
                .bind(edition_id)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            let _ = sqlx::query(&sql)
                .bind(audio_checksum)
                .bind(&now)
                .bind(edition_id)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    };
    Ok(())
}

/// Record a creator credit for a narration edition.
///
/// For TTS narration, the creator is the AI provider (named in `label`)
/// and the role is "narrator". The `creator_id` column stores the
/// provider name (external, not a local pseud).
pub async fn add_narration_creator(
    db: &Database,
    edition_id: &str,
    creator_id: &str,
    role: &str,
) -> Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = sql_owned(
        db,
        "INSERT INTO media_edition_creators (id, edition_id, creator_id, role, created_at)
         VALUES (?, ?, ?, ?, ?)"
            .to_string(),
        "INSERT INTO media_edition_creators (id, edition_id, creator_id, role, created_at)
         VALUES (?::uuid, ?::uuid, ?, ?, ?)"
            .to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            let _ = sqlx::query(&sql)
                .bind(&id)
                .bind(edition_id)
                .bind(creator_id)
                .bind(role)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            let _ = sqlx::query(&sql)
                .bind(&id)
                .bind(edition_id)
                .bind(creator_id)
                .bind(role)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    };
    Ok(())
}

/// Decode the audio checksum recorded on an edition, if it has one.
///
/// `None` means "no audio yet" — the job has not run, or it failed. An empty
/// string is normalised to `None` because an edition that has never been
/// narrated should read as absent, not as pointed at nothing.
pub async fn narration_audio_checksum(db: &Database, edition_id: &str) -> Result<Option<String>> {
    let sql = sql_owned(
        db,
        "SELECT audio_checksum FROM media_editions WHERE id = ?".to_string(),
        "SELECT audio_checksum FROM media_editions WHERE id = ?::uuid".to_string(),
    );
    let row: Option<Option<String>> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(edition_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(edition_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.flatten().filter(|checksum| !checksum.is_empty()))
}

/// Publish a narration edition: the author's approval (spec §32.5).
///
/// Returns true when this call is what moved the edition out of draft, so a
/// second approval is an idempotent no-op rather than a second event. An
/// edition with no audio yet is refused: approving silence would put a
/// download door in front of nothing.
pub async fn approve_narration_edition(db: &Database, edition_id: &str) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let sql = sql_owned(
        db,
        "UPDATE media_editions
         SET published_at = ?, updated_at = ?, version = version + 1
         WHERE id = ? AND edition_kind = 'narration' AND published_at IS NULL
           AND audio_checksum IS NOT NULL AND audio_checksum != ''"
            .to_string(),
        "UPDATE media_editions
         SET published_at = ?, updated_at = ?, version = version + 1
         WHERE id = ?::uuid AND edition_kind = 'narration' AND published_at IS NULL
           AND audio_checksum IS NOT NULL AND audio_checksum != ''"
            .to_string(),
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&now)
            .bind(&now)
            .bind(edition_id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&now)
            .bind(&now)
            .bind(edition_id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}
