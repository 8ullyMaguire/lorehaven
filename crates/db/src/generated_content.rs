//! Reading and writing §51's generated-content posture (spec §51.1, M45-12).
//!
//! Two tables meet here, and the relationship between them is the design:
//!
//! | table | what it holds | who writes it |
//! |---|---|---|
//! | `generated_content_policy` | the INSTANCE's current posture | an operator |
//! | `works.generated_content_posture` | the posture a WORK was published under | [`stamp_work_posture`] |
//!
//! ## Why the posture is duplicated onto the work
//!
//! §51.1 requires it, and the reason is retroactivity. An operator who changes the
//! policy from `allow` to `disclose` is making a change about *future* writes. If a
//! read joined to the policy row, every work already in the corpus would be relabelled
//! — under terms its author did not agree to, decided by an operator who never
//! asked. [`generated_content_policy`] exists so there is one place to look for
//! "what is this instance's policy now", and it is deliberately NOT what a work
//! display reads.
//!
//! [`generated_content_policy`] reads default when no row exists. That is not a
//! second default: migration 0109 creates no row, and `forbid` is the answer for
//! an instance that has never set one — the same answer the column default gives,
//! reached without a row. Written as a `COALESCE` over an empty table so the two
//! paths cannot drift.

use crate::{Backend, Database, Result};
use lorehaven_domain::generated_content::{
    GeneratedContentDeclaration, GeneratedContentPosture, GeneratedContentPostureOnWork,
    UnknownPosture,
};

/// The singleton key for `generated_content_policy`.
///
/// A constant rather than a generated id, matching `instance_retention_policy`
/// (0087): a policy table that could hold two rows would need a rule for which one
/// wins, and every caller would have to remember it.
pub const POLICY_ID: &str = "default";

/// The instance's generated-content posture.
///
/// `forbid` when no row exists. §51.1's default, and the reason it is the default is
/// the same reason it is the migration's DEFAULT: `allow` is the only value that
/// changes what the corpus *is*, so an instance has to choose it deliberately.
pub async fn generated_content_policy(db: &Database) -> Result<GeneratedContentPosture> {
    let sql = db.sql(
        "SELECT posture FROM generated_content_policy WHERE id = ?",
        "SELECT posture FROM generated_content_policy WHERE id = $1",
    );

    let stored: Option<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(POLICY_ID)
                .fetch_optional(db.sqlite_pool().expect("sqlite pool"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(POLICY_ID)
                .fetch_optional(db.postgres_pool().expect("postgres pool"))
                .await?
        }
    };

    // No row is not an error and not `None`: §51.1's rule applies to an undecided
    // instance exactly as it applies to a decided one. A stored value outside the
    // fixed set IS an error, because it means a row was written without the CHECK
    // (a migration that skipped it, or a hand-edited row) and defaulting it would
    // hide that behind a policy nobody chose.
    match stored {
        None => Ok(GeneratedContentPosture::Forbid),
        Some(raw) => raw
            .parse::<GeneratedContentPosture>()
            .map_err(|UnknownPosture(raw)| {
                anyhow::anyhow!("generated_content_policy.posture = {raw:?}")
            }),
    }
}

/// Set the instance's posture, and return the version the row now holds.
///
/// Upsert rather than insert-or-error, because setting a posture is idempotent by
/// nature: an operator setting `disclose` twice has not done anything the second
/// time, and refusing the second call would make a retry after a timeout fail.
///
/// `version` increments so a concurrent change is detectable rather than silently
/// last-write-wins. The caller is expected to compare it, which is what
/// `instance_retention_policy` (0087) does.
pub async fn set_generated_content_policy(
    db: &Database,
    posture: GeneratedContentPosture,
    updated_by: Option<&str>,
    now: &str,
) -> Result<i64> {
    // `posture.as_str()` rather than a bind of the enum: the CHECK in 0109 compares
    // against these exact lowercase strings, and passing anything else would make the
    // error name the database instead of the caller's value. Going through `as_str`
    // means the string and the enum cannot drift.
    let sql = db.sql(
        "INSERT INTO generated_content_policy (id, posture, updated_by, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, 1)
         ON CONFLICT (id) DO UPDATE SET
             posture    = excluded.posture,
             updated_by = excluded.updated_by,
             updated_at = excluded.updated_at,
             version    = generated_content_policy.version + 1",
        "INSERT INTO generated_content_policy (id, posture, updated_by, created_at, updated_at, version)
         VALUES ($1, $2, $3::uuid, $4, $5, 1)
         ON CONFLICT (id) DO UPDATE SET
             posture    = excluded.posture,
             updated_by = excluded.updated_by,
             updated_at = excluded.updated_at,
             version    = generated_content_policy.version + 1",
    );

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(POLICY_ID)
                .bind(posture.as_str())
                .bind(updated_by)
                .bind(now)
                .bind(now)
                .execute(db.sqlite_pool().expect("sqlite pool"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(POLICY_ID)
                .bind(posture.as_str())
                .bind(updated_by)
                .bind(now)
                .bind(now)
                .execute(db.postgres_pool().expect("postgres pool"))
                .await?;
        }
    }

    policy_version(db).await
}

/// The policy row's `version`, or `0` when no row exists.
///
/// `0` rather than `1` for an absent row, so "no row" cannot be mistaken for "a row
/// at version 1" by a caller comparing versions for concurrency.
pub async fn policy_version(db: &Database) -> Result<i64> {
    let sql = db.sql(
        "SELECT version FROM generated_content_policy WHERE id = ?",
        "SELECT version FROM generated_content_policy WHERE id = $1",
    );
    // Per-dialect width. `version` is `INTEGER`, which is INT8 on SQLite (no width)
    // and INT4 on PostgreSQL, and sqlx checks the width -- so a shared `i64` arm
    // compiles, passes every SQLite test, and fails on the first row of the
    // PostgreSQL leg with "Rust type i64 (as SQL type INT8) is not compatible with
    // SQL type INT4". Widened here so callers get one `i64` regardless of engine.
    let version: i64 = match db.backend() {
        Backend::Sqlite => {
            let v: Option<i64> = sqlx::query_scalar(&sql)
                .bind(POLICY_ID)
                .fetch_optional(db.sqlite_pool().expect("sqlite pool"))
                .await?;
            v.unwrap_or(0)
        }
        Backend::Postgres => {
            let v: Option<i32> = sqlx::query_scalar(&sql)
                .bind(POLICY_ID)
                .fetch_optional(db.postgres_pool().expect("postgres pool"))
                .await?;
            i64::from(v.unwrap_or(0))
        }
    };
    Ok(version)
}

/// The posture a work was published under, with what the author declared.
///
/// `None` when the work has no such row — a work written before 0109, which 0109's
/// backfill prevents by filling every existing work with the policy in force, so in
/// practice `None` means the row was deleted rather than never written. Returned as
/// `None` rather than defaulted to `forbid` so a caller can tell the difference, per
/// §49.3's absent-is-not-a-zero rule.
pub async fn work_generated_content(
    db: &Database,
    work_id: &str,
) -> Result<Option<GeneratedContentPostureOnWork>> {
    let sql = db.sql(
        "SELECT generated_content_posture, generated_declared_at
         FROM works
         WHERE id = ?",
        "SELECT generated_content_posture, generated_declared_at
         FROM works
         WHERE id = $1::uuid",
    );

    // `posture` is NULL-able on SQLite (see 0109's note on why) and NOT NULL on
    // PostgreSQL, so the column is read as `Option<String>` and the SQLite arm's
    // NULL is resolved there. A shared `String` arm would fail to compile for the
    // SQLite query, which is the point of reading it as nullable on both.
    let row: Option<(Option<String>, Option<String>)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite pool"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_optional(db.postgres_pool().expect("postgres pool"))
                .await?
        }
    };

    let Some((posture, declared_at)) = row else {
        return Ok(None);
    };

    let posture = match posture {
        Some(raw) => raw
            .parse::<GeneratedContentPosture>()
            .map_err(|UnknownPosture(raw)| {
                anyhow::anyhow!("works.generated_content_posture = {raw:?}")
            })?,
        // Only reachable on SQLite, and only because that engine cannot enforce
        // NOT NULL on an added column. 0109 backfills every existing row, so this
        // is a row written without the column rather than a pre-0108 row.
        None => GeneratedContentPosture::Forbid,
    };

    let declaration = if declared_at.is_some() {
        GeneratedContentDeclaration::DeclaredGenerated
    } else {
        GeneratedContentDeclaration::Undeclared
    };

    Ok(Some(GeneratedContentPostureOnWork {
        declaration,
        posture,
    }))
}

/// Stamp a work with the posture it was published under.
///
/// ## Why this takes the posture as a parameter instead of reading it
///
/// A caller that is publishing a work reads [`generated_content_policy`] and passes
/// the result here, so the value written and the value the policy held at that
/// moment are the same value. Reading the policy inside this function would make the
/// two a race: the policy could change between the caller's read and this write, and
/// the work would carry a posture nobody decided for it.
///
/// # Errors
///
/// [`PostureRefusal`](lorehaven_domain::generated_content::PostureRefusal) when the
/// work declares itself generated under `forbid`.
/// §51.1 refuses **before the work row exists**, so the caller runs this before its
/// own INSERT — this function refuses so the caller does not have to remember to.
pub async fn stamp_work_posture(
    db: &Database,
    work_id: &str,
    declaration: GeneratedContentDeclaration,
    posture: GeneratedContentPosture,
    declared_at: &str,
) -> Result<GeneratedContentPostureOnWork> {
    let resolved = GeneratedContentPostureOnWork::resolve(declaration, posture)?;

    let sql = db.sql(
        "UPDATE works
            SET generated_content_posture = ?,
                generated_declared_at     = ?
          WHERE id = ?",
        "UPDATE works
            SET generated_content_posture = $1,
                generated_declared_at     = $2
          WHERE id = $3::uuid",
    );

    // `declared_at` is a parameter, not a clock read, and every other write in this
    // crate takes its timestamps the same way. Two reasons, and the second is the
    // real one: a function that read the clock would make the stored declaration
    // time differ between two runs that are otherwise identical, so a test could
    // not assert it; and a caller stamping a work on the author's behalf -- an
    // import, a migration of their old drafts -- would have no way to record when
    // the author actually declared it, and the import's timestamp would silently
    // become the author's statement.
    let declared_at = resolved.declaration.as_declared_at().map(|_| declared_at);

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(posture.as_str())
                .bind(declared_at)
                .bind(work_id)
                .execute(db.sqlite_pool().expect("sqlite pool"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(posture.as_str())
                .bind(declared_at)
                .bind(work_id)
                .execute(db.postgres_pool().expect("postgres pool"))
                .await?;
        }
    }

    Ok(resolved)
}
