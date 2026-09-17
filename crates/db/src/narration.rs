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
                .bind(&id).bind(work_id).bind(label)
                .bind(parent_edition_id).bind(&now).bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            let _ = sqlx::query(&sql)
                .bind(&id).bind(work_id).bind(label)
                .bind(parent_edition_id).bind(&now).bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    };
    Ok(id)
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
        "INSERT INTO media_edition_creators (id::uuid, edition_id::uuid, creator_id, role, created_at)
         VALUES (?::uuid, ?::uuid, ?, ?, ?)"
            .to_string(),
    );
    match db.backend() {
        Backend::Sqlite => {
            let _ = sqlx::query(&sql)
                .bind(&id).bind(edition_id).bind(creator_id).bind(role).bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            let _ = sqlx::query(&sql)
                .bind(&id).bind(edition_id).bind(creator_id).bind(role).bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    };
    Ok(())
}
