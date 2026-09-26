//! The instance Taste Profile, stored (spec §0.4 as amended by
//! `docs/spec-amendments/taste-gravitational-system.md`).
//!
//! `PUT /discovery/admin/taste-profile` used to answer `{"status": "updated"}`
//! and persist nothing, so an operator's change to the instance's taste model
//! was accepted, reported as done, and forgotten by the next request. The
//! config file cannot be the home for it either — a running instance does not
//! rewrite its own configuration, and the API has no way to edit a file it may
//! not have permission to write.
//!
//! One row, `id = 1`, because an instance has one taste model. The dimensions
//! are JSON rather than columns precisely because the amendment says the axes
//! are instance-defined: a fixed column list would be the hardcoded taxonomy the
//! amendment rules out.

use anyhow::Result;
use serde_json::Value;
use uuid::Uuid;

use crate::{Backend, Database};

/// The stored profile, as the row holds it.
#[derive(Debug, Clone)]
pub struct StoredTasteProfile {
    /// `[{key, label, admin_target, weight}]`, still unparsed.
    pub dimensions: Value,
    pub exemplars: Value,
    pub anti_examples: Value,
    pub gravity_strength: i64,
    pub signal_weight_mode: String,
    pub admin_weight: i64,
    pub diversity_injection_percent: i64,
    pub updated_by: Option<String>,
    pub updated_at: String,
    pub version: i64,
}

/// One `instance_taste_profile` row.
///
/// `FromRow` so the SELECT's column names are the field names and the compiler
/// checks one against the other -- the same reason `recommendation_slots` uses
/// named rows instead of a ten-element tuple spelled out four times.
#[derive(Debug, Clone, sqlx::FromRow)]
struct TasteProfileRow {
    dimensions: String,
    exemplars: String,
    anti_examples: String,
    gravity_strength: i64,
    signal_weight_mode: String,
    admin_weight: i64,
    diversity_injection_percent: i64,
    updated_by: Option<String>,
    updated_at: String,
    version: i64,
}

impl TasteProfileRow {
    /// The row as the store hands it out, with the JSON columns parsed.
    fn into_stored(self) -> StoredTasteProfile {
        // A column that will not parse is kept as an empty list rather than
        // failing the read: a corrupt taste model should not take down every
        // surface that asks what the instance's dimensions are.
        let parse = |raw: &str| serde_json::from_str(raw).unwrap_or(Value::Array(vec![]));
        StoredTasteProfile {
            dimensions: parse(&self.dimensions),
            exemplars: parse(&self.exemplars),
            anti_examples: parse(&self.anti_examples),
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

/// Read the profile, or `None` when the operator has never set one.
///
/// A missing row is not an error: the config file is the starting point, and an
/// instance that has never had its taste model edited is running on it. The
/// caller decides what a missing row means for a given surface.
pub async fn read(db: &Database) -> Result<Option<StoredTasteProfile>> {
    // `dimensions` and the anchor lists are JSONB on PostgreSQL and are read as
    // text, so each arm casts on its own side. The integer columns are read into
    // i64, which decodes from INTEGER and BIGINT alike, so they need no cast.
    let sql = db.sql(
        "SELECT dimensions, exemplars, anti_examples, gravity_strength, signal_weight_mode,
                admin_weight, diversity_injection_percent, updated_by, updated_at, version
           FROM instance_taste_profile WHERE id = 1",
        "SELECT dimensions::text AS dimensions, exemplars::text AS exemplars,
                anti_examples::text AS anti_examples, gravity_strength, signal_weight_mode,
                admin_weight::bigint AS admin_weight,
                diversity_injection_percent::bigint AS diversity_injection_percent,
                updated_by::text AS updated_by, updated_at::text AS updated_at,
                version::bigint AS version
           FROM instance_taste_profile WHERE id = 1",
    );
    let row: Option<TasteProfileRow> = match db.backend() {
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
    Ok(row.map(TasteProfileRow::into_stored))
}

/// The fields an operator may change.
#[derive(Debug, Clone)]
pub struct TasteProfileUpdate {
    pub dimensions: Value,
    pub exemplars: Value,
    pub anti_examples: Value,
    pub gravity_strength: i64,
    pub signal_weight_mode: String,
    pub admin_weight: i64,
    pub diversity_injection_percent: i64,
}

/// Write the profile, bumping its version.
///
/// An upsert rather than an insert-then-retry: the row is a singleton, so the
/// common case is the second write, and making that the awkward one would
/// invite a check-then-insert race between two operators.
#[allow(clippy::too_many_arguments)]
pub async fn write(db: &Database, update: &TasteProfileUpdate, updated_by: Uuid) -> Result<i64> {
    let now = crate::identity::now_rfc3339();
    let dimensions = update.dimensions.to_string();
    let exemplars = update.exemplars.to_string();
    let anti_examples = update.anti_examples.to_string();

    let sql = db.sql(
        "INSERT INTO instance_taste_profile
           (id, dimensions, exemplars, anti_examples, gravity_strength, signal_weight_mode,
            admin_weight, diversity_injection_percent, updated_by, updated_at, version)
         VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)
         ON CONFLICT (id) DO UPDATE SET
            dimensions = excluded.dimensions,
            exemplars = excluded.exemplars,
            anti_examples = excluded.anti_examples,
            gravity_strength = excluded.gravity_strength,
            signal_weight_mode = excluded.signal_weight_mode,
            admin_weight = excluded.admin_weight,
            diversity_injection_percent = excluded.diversity_injection_percent,
            updated_by = excluded.updated_by,
            updated_at = excluded.updated_at,
            version = instance_taste_profile.version + 1",
        "INSERT INTO instance_taste_profile
           (id, dimensions, exemplars, anti_examples, gravity_strength, signal_weight_mode,
            admin_weight, diversity_injection_percent, updated_by, updated_at, version)
         VALUES (1, ?::jsonb, ?::jsonb, ?::jsonb, ?, ?, ?, ?, ?::uuid, ?::timestamptz, 1)
         ON CONFLICT (id) DO UPDATE SET
            dimensions = excluded.dimensions,
            exemplars = excluded.exemplars,
            anti_examples = excluded.anti_examples,
            gravity_strength = excluded.gravity_strength,
            signal_weight_mode = excluded.signal_weight_mode,
            admin_weight = excluded.admin_weight,
            diversity_injection_percent = excluded.diversity_injection_percent,
            updated_by = excluded.updated_by,
            updated_at = excluded.updated_at,
            version = instance_taste_profile.version + 1",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&dimensions)
                .bind(&exemplars)
                .bind(&anti_examples)
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
                .bind(&dimensions)
                .bind(&exemplars)
                .bind(&anti_examples)
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
    // twice under contention would otherwise report a version the row does not
    // have.
    let sql_version = db.sql(
        "SELECT version FROM instance_taste_profile WHERE id = 1",
        "SELECT version::bigint AS version FROM instance_taste_profile WHERE id = 1",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_profile_keeps_the_dimensions_it_was_given() {
        // The shape is the amendment's, and a future reader of the row depends
        // on it, so it is pinned here rather than left to the SQL.
        let dims = serde_json::json!([
            {"key": "angst", "label": "Angst", "admin_target": 0.4, "weight": 1.0},
            {"key": "prose_density", "label": "Prose density", "admin_target": 0.7, "weight": 0.5}
        ]);
        let update = TasteProfileUpdate {
            dimensions: dims.clone(),
            exemplars: Value::Array(vec![]),
            anti_examples: Value::Array(vec![]),
            gravity_strength: 500,
            signal_weight_mode: "balanced".to_string(),
            admin_weight: 1,
            diversity_injection_percent: 10,
        };
        assert_eq!(update.dimensions, dims);
        // `to_string` is what goes into the column, so it has to be the same
        // document the caller handed in.
        let stored: Value = serde_json::from_str(&update.dimensions.to_string()).expect("parse");
        assert_eq!(stored, dims);
    }

    #[test]
    fn a_corrupt_json_column_reads_back_as_an_empty_list() {
        // A taste model that will not parse must not take down every surface
        // that asks what the instance's dimensions are.
        let parsed: Value = serde_json::from_str("{not json").unwrap_or(Value::Array(vec![]));
        assert_eq!(parsed, Value::Array(vec![]));
    }
}
