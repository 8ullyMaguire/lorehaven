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

/// Write the knobs, bumping the version and appending the state it replaced.
///
/// The history insert and the settings upsert are one transaction, in that
/// order. Two separate statements would leave a window where a crash records a
/// change with no record of what it replaced — and since the settings row is a
/// singleton, that history row is the only copy of the old values.
///
/// An upsert rather than an insert-then-retry: the row is a singleton, so the
/// common case is the second write, and making that the awkward one would invite
/// a check-then-insert race between two operators.
pub async fn write(db: &Database, update: &TasteSettingsUpdate, updated_by: Uuid) -> Result<i64> {
    let now = crate::identity::now_rfc3339();

    // Read the current state first, so the history row can record it. Done
    // inside the transaction below as well, so the two reads cannot disagree
    // under a concurrent write; this one is only to decide whether there is a
    // previous version at all.
    let before = read(db).await?;

    let upsert = db.sql(
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
    let history = db.sql(
        "INSERT INTO instance_taste_settings_history
           (history_id, replaced_version, gravity_strength, signal_weight_mode,
            admin_weight, diversity_injection_percent, changed_by, changed_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO instance_taste_settings_history
           (history_id, replaced_version, gravity_strength, signal_weight_mode,
            admin_weight, diversity_injection_percent, changed_by, changed_at)
         VALUES (?::uuid, ?, ?, ?, ?, ?, ?::uuid, ?::timestamptz)",
    );

    let history_id = Uuid::new_v4();
    // `before` is the row as it stood, or the config's values for the first
    // version. Recording the config means "roll back past version 1" lands on
    // the instance's starting model rather than on a hole.
    let (prev_version, prev_strength, prev_mode, prev_admin, prev_diversity) = match &before {
        Some(b) => (
            Some(b.version),
            b.gravity_strength,
            b.signal_weight_mode.clone(),
            b.admin_weight,
            b.diversity_injection_percent,
        ),
        None => (None, 0, "taste_weighted".to_string(), 1, 10),
    };

    let version: i64 = match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            let mut tx = pool.begin().await?;
            sqlx::query(&history)
                .bind(history_id.to_string())
                .bind(prev_version)
                .bind(prev_strength)
                .bind(&prev_mode)
                .bind(prev_admin)
                .bind(prev_diversity)
                .bind(updated_by.to_string())
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            sqlx::query(&upsert)
                .bind(update.gravity_strength)
                .bind(&update.signal_weight_mode)
                .bind(update.admin_weight)
                .bind(update.diversity_injection_percent)
                .bind(updated_by.to_string())
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            let sql_version = "SELECT version FROM instance_taste_settings WHERE id = 1";
            let v: i64 = sqlx::query_as::<_, (i64,)>(sql_version)
                .fetch_one(&mut *tx)
                .await?
                .0;
            tx.commit().await?;
            v
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            let mut tx = pool.begin().await?;
            sqlx::query(&history)
                .bind(history_id)
                .bind(prev_version)
                .bind(prev_strength)
                .bind(&prev_mode)
                .bind(prev_admin)
                .bind(prev_diversity)
                .bind(updated_by)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            sqlx::query(&upsert)
                .bind(update.gravity_strength)
                .bind(&update.signal_weight_mode)
                .bind(update.admin_weight)
                .bind(update.diversity_injection_percent)
                .bind(updated_by)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            let sql_version = db.sql(
                "SELECT version FROM instance_taste_settings WHERE id = 1",
                "SELECT version::bigint AS version FROM instance_taste_settings WHERE id = 1",
            );
            let v: i64 = sqlx::query_as::<_, (i64,)>(&sql_version)
                .fetch_one(&mut *tx)
                .await?
                .0;
            tx.commit().await?;
            v
        }
    };
    Ok(version)
}

/// The most recent entries of the taste-knob history, newest first.
///
/// Newest first because the question an operator asks is "what did I just
/// change it from", and the answer is the first row.
///
/// Ordered by `replaced_version`, not by `changed_at`. `now_rfc3339()` has
/// second precision, so two writes inside one test -- or one operator clicking
/// save twice -- share a timestamp, and a timestamp ordering then returns the
/// two rows in whatever order the storage engine happened to put them. The
/// version each row replaced is unique and strictly increasing, so it is the
/// only ordering here that is correct rather than usually-correct.
pub async fn history(db: &Database, limit: i64) -> Result<Vec<TasteHistoryEntry>> {
    let sql = db.sql(
        "SELECT history_id, replaced_version, gravity_strength, signal_weight_mode,
                admin_weight, diversity_injection_percent, changed_by, changed_at
           FROM instance_taste_settings_history
          ORDER BY (replaced_version IS NULL), replaced_version DESC, rowid DESC LIMIT ?",
        "SELECT history_id::text, replaced_version::bigint, gravity_strength::bigint AS gravity_strength,
                signal_weight_mode, admin_weight::bigint AS admin_weight,
                diversity_injection_percent::bigint AS diversity_injection_percent,
                changed_by::text AS changed_by, changed_at::text AS changed_at
           FROM instance_taste_settings_history
          ORDER BY replaced_version DESC NULLS LAST, changed_at DESC, history_id DESC LIMIT ?",
    );
    let rows: Vec<TasteHistoryRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows.into_iter().map(TasteHistoryRow::into_entry).collect())
}

/// Restore the settings row to the state recorded by a history entry.
///
/// A rollback is a *write*, not a delete: the new state is appended to the
/// history in its own right, so the trail shows that a rollback happened and
/// who asked for it. Two rollbacks therefore walk backwards through the history
/// rather than toggling between two values.
pub async fn rollback(
    db: &Database,
    history_id: Uuid,
    rolled_back_by: Uuid,
) -> Result<RollbackOutcome> {
    let sql_read = db.sql(
        "SELECT replaced_version, gravity_strength, signal_weight_mode, admin_weight,
                diversity_injection_percent
           FROM instance_taste_settings_history WHERE history_id = ?",
        "SELECT replaced_version::bigint, gravity_strength::bigint AS gravity_strength,
                signal_weight_mode, admin_weight::bigint AS admin_weight,
                diversity_injection_percent::bigint AS diversity_injection_percent
           FROM instance_taste_settings_history WHERE history_id = ?::uuid",
    );
    let entry: Option<(Option<i64>, i64, String, i64, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql_read)
                .bind(history_id.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql_read)
                .bind(history_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    let Some((_, strength, mode, admin, diversity)) = entry else {
        return Err(TasteRollbackError::UnknownEntry.into());
    };

    let restored = TasteSettingsUpdate {
        gravity_strength: strength,
        signal_weight_mode: mode,
        admin_weight: admin,
        diversity_injection_percent: diversity,
    };
    // `write` appends the pre-rollback state to the history, so the trail keeps
    // what was undone as well as what was restored.
    // `write` appends the pre-rollback state to the history, so the trail keeps
    // what was undone as well as what was restored.
    let version = write(db, &restored, rolled_back_by).await?;
    Ok(RollbackOutcome { version, restored })
}

/// Why a rollback could not be performed.
#[derive(Debug, thiserror::Error)]
pub enum TasteRollbackError {
    #[error("no history entry with that id")]
    UnknownEntry,
}

/// What a rollback restored.
#[derive(Debug, Clone)]
pub struct RollbackOutcome {
    /// The settings version after the rollback.
    pub version: i64,
    /// The values that are now in force.
    pub restored: TasteSettingsUpdate,
}

/// One entry of the taste-knob history.
///
/// `Serialize` because the operator surface returns these directly: a history an
/// operator has to re-assemble from named fields is a history nobody reads.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TasteHistoryEntry {
    pub history_id: String,
    /// The version this entry replaced; `None` for the first write, which
    /// replaced the config file.
    pub replaced_version: Option<i64>,
    pub gravity_strength: i64,
    pub signal_weight_mode: String,
    pub admin_weight: i64,
    pub diversity_injection_percent: i64,
    pub changed_by: Option<String>,
    pub changed_at: String,
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

/// One `instance_taste_settings_history` row.
#[derive(Debug, Clone, sqlx::FromRow)]
struct TasteHistoryRow {
    history_id: String,
    replaced_version: Option<i64>,
    gravity_strength: i64,
    signal_weight_mode: String,
    admin_weight: i64,
    diversity_injection_percent: i64,
    changed_by: Option<String>,
    changed_at: String,
}

impl TasteHistoryRow {
    fn into_entry(self) -> TasteHistoryEntry {
        TasteHistoryEntry {
            history_id: self.history_id,
            replaced_version: self.replaced_version,
            gravity_strength: self.gravity_strength,
            signal_weight_mode: self.signal_weight_mode,
            admin_weight: self.admin_weight,
            diversity_injection_percent: self.diversity_injection_percent,
            changed_by: self.changed_by,
            changed_at: self.changed_at,
        }
    }
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

    #[test]
    fn a_history_entry_says_which_version_it_replaced() {
        // The first write replaced the config file, which is not a version, so
        // `replaced_version` is null rather than 0. Zero is a version an
        // operator could believe existed.
        let row = TasteHistoryRow {
            history_id: "22222222-2222-2222-2222-222222222222".to_string(),
            replaced_version: None,
            gravity_strength: 0,
            signal_weight_mode: "taste_weighted".to_string(),
            admin_weight: 1,
            diversity_injection_percent: 10,
            changed_by: Some("11111111-1111-1111-1111-111111111111".to_string()),
            changed_at: "2026-09-26T00:00:00Z".to_string(),
        };
        let entry = row.into_entry();
        assert_eq!(entry.replaced_version, None, "the config is not version 0");
        assert_eq!(entry.admin_weight, 1);
    }

    #[test]
    fn a_rollback_outcome_carries_what_it_restored() {
        // The door reports the values now in force rather than a bare "ok", so a
        // client can show the operator what the instance went back to without a
        // second round trip.
        let outcome = RollbackOutcome {
            version: 4,
            restored: TasteSettingsUpdate {
                gravity_strength: 500,
                signal_weight_mode: "balanced".to_string(),
                admin_weight: 2,
                diversity_injection_percent: 15,
            },
        };
        assert_eq!(outcome.version, 4);
        assert_eq!(outcome.restored.gravity_strength, 500);
        assert_eq!(outcome.restored.signal_weight_mode, "balanced");
    }
}
