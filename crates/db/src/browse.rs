use crate::sql_owned;
use crate::Backend;
use crate::Database;
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// A reader's remembered sort choice for one browse surface (spec §43.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortPreference {
    pub pseud_id: String,
    pub surface: String,
    pub sort_value: String,
    pub updated_at: String,
}

/// Upsert a sort preference for a pseud on a given surface.
pub async fn set_sort_preference(
    db: &Database,
    pseud_id: &str,
    surface: &str,
    sort_value: &str,
) -> Result<()> {
    let sql = sql_owned(
        db,
        r#"
        INSERT INTO reader_sort_preferences (pseud_id, surface, sort_value, updated_at)
        VALUES (?1, ?2, ?3, datetime('now'))
        ON CONFLICT (pseud_id, surface) DO UPDATE SET
            sort_value = excluded.sort_value,
            updated_at = excluded.updated_at
        "#
        .into(),
        r#"
        INSERT INTO reader_sort_preferences (pseud_id, surface, sort_value, updated_at)
        VALUES ($1::uuid, $2, $3, now())
        ON CONFLICT (pseud_id, surface) DO UPDATE SET
            sort_value = EXCLUDED.sort_value,
            updated_at = EXCLUDED.updated_at
        "#
        .into(),
    );

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(pseud_id)
                .bind(surface)
                .bind(sort_value)
                .execute(db.sqlite_pool().expect("sqlite pool for sqlite backend"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(pseud_id)
                .bind(surface)
                .bind(sort_value)
                .execute(
                    db.postgres_pool()
                        .expect("postgres pool for postgres backend"),
                )
                .await?;
        }
    }
    Ok(())
}

/// Read a pseud's remembered sort preference for a surface, if any.
pub async fn get_sort_preference(
    db: &Database,
    pseud_id: &str,
    surface: &str,
) -> Result<Option<SortPreference>> {
    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, SortPreferenceRow>(
                "SELECT pseud_id, surface, sort_value, updated_at FROM reader_sort_preferences WHERE pseud_id = ?1 AND surface = ?2",
            )
            .bind(pseud_id)
            .bind(surface)
            .fetch_optional(db.sqlite_pool().expect("sqlite pool"))
            .await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, SortPreferenceRow>(
                // `pseud_id` is UUID and SortPreferenceRow.pseud_id is a String,
                // so the SELECT list needs the cast -- the opposite direction from
                // the `$1::uuid` in the same statement's WHERE.
                "SELECT pseud_id::text AS pseud_id, surface, sort_value, updated_at::text \
                 FROM reader_sort_preferences WHERE pseud_id = $1::uuid AND surface = $2",
            )
            .bind(pseud_id)
            .bind(surface)
            .fetch_optional(db.postgres_pool().expect("postgres pool"))
            .await?
        }
    };
    Ok(row.map(|r| r.into_domain()))
}

/// Delete a pseud's remembered sort preference for a surface.
pub async fn delete_sort_preference(db: &Database, pseud_id: &str, surface: &str) -> Result<()> {
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("DELETE FROM reader_sort_preferences WHERE pseud_id = ?1 AND surface = ?2")
                .bind(pseud_id)
                .bind(surface)
                .execute(db.sqlite_pool().expect("sqlite pool"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "DELETE FROM reader_sort_preferences WHERE pseud_id = $1::uuid AND surface = $2",
            )
            .bind(pseud_id)
            .bind(surface)
            .execute(db.postgres_pool().expect("postgres pool"))
            .await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Row mapping (internal)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
struct SortPreferenceRow {
    pseud_id: String,
    surface: String,
    sort_value: String,
    updated_at: String,
}

impl SortPreferenceRow {
    fn into_domain(self) -> SortPreference {
        SortPreference {
            pseud_id: self.pseud_id,
            surface: self.surface,
            sort_value: self.sort_value,
            updated_at: self.updated_at,
        }
    }
}
