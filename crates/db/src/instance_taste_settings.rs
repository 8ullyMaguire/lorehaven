//! The instance's taste *knobs*, stored (spec §0.4 as amended by
//! `docs/spec-amendments/taste-gravitational-system.md`).
//!
//! `PUT /operator/taste-profile` used to answer `{"status": "updated"}` and
//! persist nothing, so an operator's change to the instance's taste model was
//! accepted, reported as done, and forgotten by the next request. The config
//! file cannot be the home for it either — a running instance does not rewrite
//! its own configuration, and the API has no way to edit a file it may not have
//! permission to write.
//!
//! Two homes, deliberately. The dimensions live in `admin_taste_profile`
//! (migration 0054, written by `taste_health::save_admin_taste_profile` and
//! already round-trip tested since M17); the scalar knobs live here, because
//! config carries them and no table did. Adding a `dimensions` JSON column to
//! this table as well would have produced two stores for one concept, and the
//! one that lost would be whichever a future reader happened to find.

use anyhow::Result;
use uuid::Uuid;

use crate::{Backend, Database};

/// The stored knobs, as the row holds them.
#[derive(Debug, Clone)]
pub struct StoredTasteSettings {
    pub gravity_strength: i64,
    pub signal_weight_mode: String,
    pub admin_weight: i64,
    pub diversity_injection_percent: i64,
    pub updated_by: Option<String>,
    pub updated_at: String,
    pub version: i64,
}

/// Read the knobs, or `None` when the operator has never set them.
///
/// A missing row is not an error: the config file is the starting point, and an
/// instance nobody has edited is running on it. The caller decides what a
/// missing row means for a given surface.
pub async fn read(db: &Database) -> Result<Option<StoredTasteSettings>> {
    // The integer columns are read into i64, which decodes from INTEGER and
    // BIGINT alike, so they need no cast; `updated_by` and `updated_at` are cast
    // to text on the PostgreSQL side so both backends return strings.
    let sql = db.sql(
        "SELECT gravity_strength, signal_weight_mode, admin_weight,
                diversity_injection_percent, updated_by, updated_at, version
           FROM instance_taste_settings WHERE id = 1",
        "SELECT gravity_strength::bigint AS gravity_strength, signal_weight_mode,
                admin_weight::bigint AS admin_weight,
                diversity_injection_percent::bigint AS diversity_injection_percent,
                updated_by::text AS updated_by, updated_at::text AS updated_at,
                version::bigint AS version
           FROM instance_taste_settings WHERE id = 1",
    );
    let row: Option<TasteSettingsRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(TasteSettingsRow::into_stored))
}

/// The fields an operator may change.
#[derive(Debug, Clone)]
pub struct TasteSettingsUpdate {
    pub gravity_strength: i64,
    pub signal_weight_mode: String,
    pub admin_weight: i64,
    pub diversity_injection_percent: i64,
}

/// Write the knobs, bumping the version.
///
/// An upsert rather than an insert-then-retry: the row is a singleton, so the
/// common case is the second write, and making that the awkward one would invite
/// a check-then-insert race between two operators.
pub async fn write(db: &Database, update: &TasteSettingsUpdate, updated_by: Uuid) -> Result<i64> {
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO instance_taste_settings
           (id, gravity_strength, signal_weight_mode, admin_weight,
            diversity_injection_percent, updated_by, updated_at, version)
         VALUES (1, ?, ?, ?, ?, ?, ?, 1)
         ON CONFLICT (id) DO UPDATE SET
            gravity_strength = excluded.gravity_strength,
            signal_weight_mode = excluded.signal_weight_mode,
            admin_weight = excluded.admin_weight,
            diversity_injection_percent = excluded.diversity_injection_percent,
            updated_by = excluded.updated_by,
            updated_at = excluded.updated_at,
            version = instance_taste_settings.version + 1",
        "INSERT INTO instance_taste_settings
           (id, gravity_strength, signal_weight_mode, admin_weight,
            diversity_injection_percent, updated_by, updated_at, version)
         VALUES (1, ?, ?, ?, ?, ?::uuid, ?::timestamptz, 1)
         ON CONFLICT (id) DO UPDATE SET
            gravity_strength = excluded.gravity_strength,
            signal_weight_mode = excluded.signal_weight_mode,
            admin_weight = excluded.admin_weight,
            diversity_injection_percent = excluded.diversity_injection_percent,
            updated_by = excluded.updated_by,
            updated_at = excluded.updated_at,
            version = instance_taste_settings.version + 1",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(update.gravity_strength)
                .bind(&update.signal_weight_mode)
                .bind(update.admin_weight)
                .bind(update.diversity_injection_percent)
                .bind(updated_by.to_string())
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(update.gravity_strength)
                .bind(&update.signal_weight_mode)
                .bind(update.admin_weight)
                .bind(update.diversity_injection_percent)
                .bind(updated_by)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }

    // The new version, read back rather than computed: an upsert that bumped
    // twice under contention would otherwise report a version the row lacks.
    let sql_version = db.sql(
        "SELECT version FROM instance_taste_settings WHERE id = 1",
        "SELECT version::bigint AS version FROM instance_taste_settings WHERE id = 1",
    );
    let version: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (i64,)>(&sql_version)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
                .0
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (i64,)>(&sql_version)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
                .0
        }
    };
    Ok(version)
}

/// One `instance_taste_settings` row.
///
/// `FromRow` so the SELECT's column names are the field names and the compiler
/// checks one against the other — the same reason `recommendation_slots` uses
/// named rows instead of a ten-element tuple spelled out four times.
#[derive(Debug, Clone, sqlx::FromRow)]
struct TasteSettingsRow {
    gravity_strength: i64,
    signal_weight_mode: String,
    admin_weight: i64,
    diversity_injection_percent: i64,
    updated_by: Option<String>,
    updated_at: String,
    version: i64,
}

impl TasteSettingsRow {
    fn into_stored(self) -> StoredTasteSettings {
        StoredTasteSettings {
            gravity_strength: self.gravity_strength,
            signal_weight_mode: self.signal_weight_mode,
            admin_weight: self.admin_weight,
            diversity_injection_percent: self.diversity_injection_percent,
            updated_by: self.updated_by,
            updated_at: self.updated_at,
            version: self.version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_decodes_into_the_stored_shape() {
        // The column list and the struct are the same contract; this pins the
        // conversion that sqlx does for us at runtime, so a renamed column
        // fails a unit test rather than a request.
        let row = TasteSettingsRow {
            gravity_strength: 750,
            signal_weight_mode: "balanced".to_string(),
            admin_weight: 1,
            diversity_injection_percent: 10,
            updated_by: Some("11111111-1111-1111-1111-111111111111".to_string()),
            updated_at: "2026-09-26T00:00:00Z".to_string(),
            version: 3,
        };
        let stored = row.into_stored();
        assert_eq!(stored.gravity_strength, 750);
        assert_eq!(stored.version, 3);
        assert_eq!(stored.signal_weight_mode, "balanced");
        assert!(stored.updated_by.is_some());
    }

    #[test]
    fn a_row_with_no_author_still_decodes() {
        // `updated_by` is nullable because a row seeded by a migration has no
        // author. A NOT NULL here would make the table unseedable.
        let row = TasteSettingsRow {
            gravity_strength: 0,
            signal_weight_mode: "taste_weighted".to_string(),
            admin_weight: 1,
            diversity_injection_percent: 10,
            updated_by: None,
            updated_at: "2026-09-26T00:00:00Z".to_string(),
            version: 1,
        };
        assert!(row.into_stored().updated_by.is_none());
    }
}
