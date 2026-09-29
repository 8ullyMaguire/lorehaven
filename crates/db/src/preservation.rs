//! Preservation targets: the destinations, the targets recorded against them,
//! and the state each one is in (spec §11.12a, §9.7 as amended by §2).
//!
//! Three groups of functions, and the split is the design:
//!
//! * **destinations** — instance configuration (§3.2). Named archives with a
//!   base URL and the rule that identifies one of their item pages.
//! * **targets** — a crosspost recorded as a `story_identity_members` row
//!   carrying a destination and a state, per A0's decision that the concept of
//!   "this work also exists there" exists exactly once.
//! * **the recheck** — the job that finds a destination which has stopped
//!   answering and takes the credits back (§2.3).
//!
//! **Why the target is a member row and not its own table.** A
//! `preservation_targets` table and `story_identity_members` would both answer
//! "does this work exist at that location, and what state is it in". Two tables,
//! one fact, and the reconciliation lands on whoever builds §11.10 properly next
//! — at which point the preservation data has to be migrated, not just joined.
//! The cost of the chosen shape is that `story_identity_members` now carries
//! columns only this module uses, and that is stated in migration 0093 rather
//! than discovered by the next reader.
//!
//! **Why `credits_paid` lives on the member.** The credit is owed to *a
//! destination holding this work*, which is the member, and the clawback finds
//! it by `(destination_id, state)`: a dead member is a dead destination. A
//! `work_id -> credits` table would make the clawback a join that has to be
//! right about which destination died, which is the one thing the recheck job
//! knows for certain.

use anyhow::Result;
use lorehaven_domain::economy::TxnType;
use lorehaven_domain::preservation::{PreservationState, Redistribution};
use uuid::Uuid;

use crate::{Backend, Database};

/// One configured archive this instance may preserve into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreservationDestination {
    pub id: String,
    pub name: String,
    pub base_url: String,
    /// How one of this destination's item pages is identified. Stored verbatim
    /// because what an archive calls its item pages is its own business, and a
    /// normalised form here would be a second rule to keep in step.
    pub match_rule: String,
    /// Whether the archive accepts automated submission. Gates the *crosspost*,
    /// not the *record*: a reader who posted by hand and this instance verified
    /// it has still done a good thing.
    pub accepts_automated: bool,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

impl PreservationDestination {
    /// The public item URL for a record id at this destination.
    ///
    /// `match_rule` is appended to `base_url` with exactly one slash between
    /// them. The two are stored separately because an operator configures them
    /// separately and a `base_url` with a trailing slash is a normal thing to
    /// paste; the alternative — requiring the stored `base_url` to be slashless
    /// — puts a URL-normalisation rule in front of every reader of this row.
    ///
    /// Returns `None` rather than a joined string when either part is blank, so
    /// a half-configured destination produces a refusal at the call site instead
    /// of a fetch to a URL like `https://archive.example/item/`.
    #[must_use]
    pub fn item_url(&self, record_id: &str) -> Option<String> {
        if self.base_url.trim().is_empty() || record_id.trim().is_empty() {
            return None;
        }
        let base = self.base_url.trim_end_matches('/');
        let rule = self.match_rule.trim();
        if rule.is_empty() {
            Some(format!("{base}/{record_id}"))
        } else {
            Some(format!("{base}/{}/{record_id}", rule.trim_matches('/')))
        }
    }
}

/// One preservation record: a work, a destination, and what state it is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreservationTarget {
    /// The `story_identity_members` row id.
    pub member_id: String,
    pub identity_id: String,
    pub work_id: String,
    pub destination_id: String,
    pub state: PreservationState,
    /// When the destination last confirmed it carries a record naming this work.
    pub verified_at: Option<String>,
    /// When it stopped. A fact about the *destination*, never about the work.
    pub dead_at: Option<String>,
    /// A content hash of the destination page's identifying fields.
    ///
    /// Present so "the page says the same thing" is checkable later without
    /// keeping the page: a recheck that reads a different title has found a
    /// different record, and hashing what was actually compared is what lets a
    /// later reader tell "still the same work" from "the archive's page changed".
    pub evidence_hash: Option<String>,
    pub credits_paid: i64,
    /// Who crossposted it. `None` once that account is deleted, which is why the
    /// column is `ON DELETE RESTRICT` on the destination but nullable here: the
    /// credit was earned by a person and outlives their account.
    pub created_by: Option<String>,
    pub external_record_id: String,
    pub external_url: Option<String>,
    pub updated_at: Option<String>,
    pub version: i64,
}

impl PreservationTarget {
    /// Whether this target is currently earning (§2.1: the reward attaches to
    /// verification, not to the crosspost).
    #[must_use]
    pub const fn is_verified(&self) -> bool {
        matches!(self.state, PreservationState::Verified)
    }
}

/// The columns migration 0093 adds to `story_identity_members`.
///
/// Exported because the two dialects must declare the same set and the
/// migration parity test does not read `ALTER TABLE ... ADD COLUMN` — it reads
/// `CREATE TABLE` and `CREATE INDEX`. The assertion that the two migration
/// files agree is made against this list by
/// `a_preservation_member_row_carries_the_columns_both_dialects_declare`, which
/// reads the files. Without it, a column added to one dialect and not the other
/// would compile, pass the existing parity test, and fail at runtime on the
/// engine nobody tested.
pub const PRESERVATION_MEMBER_COLUMNS: &[&str] = &[
    "destination_id",
    "state",
    "verified_at",
    "dead_at",
    "evidence_hash",
    "credits_paid",
    "created_by",
    "updated_at",
    "version",
];

#[derive(sqlx::FromRow)]
struct DestinationRow {
    id: String,
    name: String,
    base_url: String,
    match_rule: String,
    accepts_automated: i64,
    enabled: i64,
    created_at: String,
    updated_at: String,
    version: i64,
}

#[derive(sqlx::FromRow)]
struct TargetRow {
    member_id: String,
    identity_id: String,
    work_id: String,
    destination_id: String,
    state: String,
    verified_at: Option<String>,
    dead_at: Option<String>,
    evidence_hash: Option<String>,
    credits_paid: i64,
    created_by: Option<String>,
    external_record_id: String,
    external_url: Option<String>,
    updated_at: Option<String>,
    version: i64,
}

fn destination_from_row(row: DestinationRow) -> PreservationDestination {
    // The flag columns are INTEGER in the migrations, not BOOLEAN — a fact this
    // repository has been bitten by (`work_view_log.is_automated` and
    // `collections.is_public` are both INTEGER), and the conversion is
    // therefore `!= 0` rather than a cast, which works on either spelling.
    PreservationDestination {
        id: row.id,
        name: row.name,
        base_url: row.base_url,
        match_rule: row.match_rule,
        accepts_automated: row.accepts_automated != 0,
        enabled: row.enabled != 0,
        created_at: row.created_at,
        updated_at: row.updated_at,
        version: row.version,
    }
}

/// The dialect's spelling of the columns this module reads.
///
/// **This is an enum and not a `db.sql(&a, &b)` pair because of a bug this file
/// shipped first.** The first version carried the PostgreSQL casts
/// (`version::bigint`, `verified_at::text`, `credits_paid::bigint`) in a single
/// shared `const` used by *both* arms, on the reasoning that the casts were
/// harmless. They are not: fourteen of this module's sixteen tests failed on
/// SQLite with
///
///     error returned from database: (code: 1) unrecognized token: ":"
///
/// because SQLite was handed `SELECT ... version::bigint AS version` and `::`
/// is not a token there. The casts are a *PostgreSQL* requirement satisfied in
/// *both* dialects, which is the mirror image of the `created_by` DDL fault
/// fixed in the same commit — there the two dialects' *schema* diverged and only
/// PostgreSQL noticed; here the two dialects' *queries* diverged and only SQLite
/// noticed. Neither is visible by reading one arm.
///
/// Resolving the list once per call, from the backend that is about to run the
/// statement, is what makes it impossible to pair a column list with the wrong
/// dialect: the two can no longer be chosen independently of each other.
///
/// The two lists are identical name for name, which is exactly what made the
/// mismatch undetectable — a reader diffing them sees only casts. The names
/// themselves are checked by
/// `migrate::tests::a_preservation_member_row_carries_the_columns_both_dialects_declare`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Sqlite,
    Postgres,
}

impl Dialect {
    const fn of(db: &Database) -> Self {
        match db.backend() {
            Backend::Sqlite => Self::Sqlite,
            Backend::Postgres => Self::Postgres,
        }
    }

    /// The destination columns, with the casts this dialect needs.
    const fn destinations(self) -> &'static str {
        match self {
            // `version` is INTEGER here and read as `i64`; SQLite has no
            // integer-width distinction, so the bare column decodes fine.
            Self::Sqlite => {
                "id, name, base_url, match_rule, accepts_automated, enabled, \
                             created_at, updated_at, version AS version"
            }
            // Two casts for the same reason, and the second is the one that cost
            // a PostgreSQL run: `version` AND `accepts_automated` are `INTEGER`
            // (`INT4`) columns, and sqlx refuses to decode an `INT4` into the
            // `i64` the `DestinationRow` struct asks for --
            //
            //     error occurred while decoding column "accepts_automated":
            //     mismatched types; Rust type `i64` (as SQL type `INT8`) is
            //     not compatible with SQL type `INT4`
            //
            // `version::bigint` was cast from the start because every other
            // module does. `accepts_automated` and `enabled` were not, on the
            // reasoning that a flag is not a counter and so does not need a
            // cast. It does. The `::int8` on all three is what makes the
            // decode legal, and it is the pair a reviewer would most plausibly
            // drop as redundant -- so it is spelled once here rather than in a
            // SELECT somebody has to notice.
            Self::Postgres => {
                "id, name, base_url, match_rule, \
                 accepts_automated::int8 AS accepts_automated, \
                 enabled::int8 AS enabled, \
                 created_at, updated_at, version::bigint AS version"
            }
        }
    }

    /// The target columns, with the casts this dialect needs.
    const fn targets(self) -> &'static str {
        match self {
            Self::Sqlite => {
                "m.id AS member_id, m.identity_id, i.work_id AS work_id, \
                 m.destination_id, m.state, m.verified_at, m.dead_at, \
                 m.evidence_hash, m.credits_paid, m.created_by, \
                 m.external_record_id, m.external_url, m.updated_at, \
                 m.version AS version"
            }
            // `work_id` is read from the IDENTITY, not from the member, and
            // that is the second time this file got it wrong. The member's own
            // `work_id` is NULL for an external member -- which every
            // preservation target is -- so decoding it as `String` fails on
            // PostgreSQL with
            //
            //   error occurred while decoding column "work_id": unexpected
            //   null; try decoding as an `Option`
            //
            // The earlier comment in `parse_target` said as much and the SELECT
            // did not do it: a comment describing a constraint the code does not
            // satisfy is worse than no comment, because the next reader trusts
            // it. `story_identities.work_id` is NOT NULL, which is what makes
            // the join the source of the answer rather than a convenience.
            //
            // `verified_at` and friends are TEXT in this schema (0085 spells them
            // TEXT to match `works`) and `credits_paid`/`version` are INTEGER
            // columns read as `i64`, so each needs its own cast. The timestamps
            // are cast to `::text` and NOT `::timestamptz` — they are not
            // timestamps, and binding them as such is the fault class the
            // handoff records.
            Self::Postgres => {
                "m.id AS member_id, m.identity_id, i.work_id::text AS work_id, \
                 m.destination_id, m.state, m.verified_at::text, \
                 m.dead_at::text, m.evidence_hash::text, \
                 m.credits_paid::bigint AS credits_paid, \
                 m.created_by::text, m.external_record_id, \
                 m.external_url::text, m.updated_at::text, \
                 m.version::bigint AS version"
            }
        }
    }
}

fn parse_target(row: TargetRow) -> PreservationTarget {
    PreservationTarget {
        member_id: row.member_id,
        identity_id: row.identity_id,
        // `work_id` is NULL-able on a member (an external member has no local
        // work) and every target is an external member, so this is `text` and
        // the emptiness is rejected by the CHECK rather than by a cast. An empty
        // string here would be a member with neither half, which the store
        // refuses to create.
        work_id: row.work_id,
        destination_id: row.destination_id,
        state: PreservationState::read(&row.state),
        verified_at: row.verified_at,
        dead_at: row.dead_at,
        evidence_hash: row.evidence_hash,
        credits_paid: row.credits_paid,
        created_by: row.created_by,
        external_record_id: row.external_record_id,
        external_url: row.external_url,
        updated_at: row.updated_at,
        version: row.version,
    }
}

// ---------------------------------------------------------------------------
// Destinations
// ---------------------------------------------------------------------------

/// Build a statement that differs only in its column list, once per dialect.
///
/// The bug the two-`const`-then-one-`db.sql` shape invited was picking a
/// column list independently of the arm it was interpolated into. Binding them
/// together here means the caller passes a template with `{destinations}` or
/// `{targets}` and this function is the only place that knows which spelling
fn select_by_dialect(db: &Database, template: &str) -> String {
    let dialect = Dialect::of(db);
    template
        .replace("{destinations}", dialect.destinations())
        .replace("{targets}", dialect.targets())
}

/// Every configured destination, enabled or not, in name order.
///
/// Both states are returned because the admin surface has to be able to show a
/// disabled row in order to re-enable it, and a list that hides disabled
/// destinations makes a disabled one indistinguishable from one that was
/// deleted. Name order, not id order: an operator reads this list by name, and
/// two identically configured instances reporting different orders is the kind
/// of difference that becomes a bug report.
pub async fn list_destinations(db: &Database) -> Result<Vec<PreservationDestination>> {
    let sql = select_by_dialect(
        db,
        "SELECT {destinations} FROM preservation_destinations ORDER BY name, id",
    );
    let rows: Vec<DestinationRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows.into_iter().map(destination_from_row).collect())
}

/// One destination by id, or `None`.
///
/// `None` for a *disabled* destination is deliberate and is the same answer as
/// for an absent one: a disabled destination is not eligible, and a caller that
/// has to tell them apart will treat the disabled one as eligible somewhere.
pub async fn enabled_destination(
    db: &Database,
    destination_id: &str,
) -> Result<Option<PreservationDestination>> {
    let sqlite_sql = "SELECT {destinations} FROM preservation_destinations \
                      WHERE id = ? AND enabled != 0";
    let postgres_sql = "SELECT {destinations} FROM preservation_destinations \
                        WHERE id = $1 AND enabled != 0";
    let sql = select_by_dialect(db, &db.sql(sqlite_sql, postgres_sql));
    let row: Option<DestinationRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(destination_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(destination_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(destination_from_row))
}

/// How many destinations this instance has enabled (§3.2's "an instance with no
/// destinations configured has no preservation targets").
///
/// This is the `eligible` input to `PreservationEligibility::evaluate`, and it
/// is an *enabled* count because a disabled destination is not one a reader can
/// act on — counting it would make a work look "short" against a threshold it
/// could never reach.
pub async fn enabled_destination_count(db: &Database) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM preservation_destinations WHERE enabled != 0",
        "SELECT COUNT(*)::bigint FROM preservation_destinations WHERE enabled != 0",
    );
    Ok(match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    })
}

/// Record a destination, or update the one with this id.
///
/// An upsert on `id` rather than on `(name, base_url)`: the operator identifies
/// a destination by the id they use in API calls, and a uniqueness rule on the
/// name would make "rename an archive" a delete-and-recreate that orphans every
/// target pointing at it. Re-pointing them is not possible — a target names a
/// *destination* and the credits against it — so the row has to survive the
/// rename.
pub async fn write_destination(
    db: &Database,
    id: &str,
    name: &str,
    base_url: &str,
    match_rule: &str,
    accepts_automated: bool,
    enabled: bool,
) -> Result<PreservationDestination> {
    let now = crate::identity::now_rfc3339();
    let accepts = i64::from(accepts_automated);
    let enabled = i64::from(enabled);
    let sql = db.sql(
        "INSERT INTO preservation_destinations
           (id, name, base_url, match_rule, accepts_automated, enabled,
            created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1)
         ON CONFLICT (id) DO UPDATE SET
            name = excluded.name,
            base_url = excluded.base_url,
            match_rule = excluded.match_rule,
            accepts_automated = excluded.accepts_automated,
            enabled = excluded.enabled,
            updated_at = excluded.updated_at,
            version = preservation_destinations.version + 1",
        "INSERT INTO preservation_destinations
           (id, name, base_url, match_rule, accepts_automated, enabled,
            created_at, updated_at, version)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1)
         ON CONFLICT (id) DO UPDATE SET
            name = excluded.name,
            base_url = excluded.base_url,
            match_rule = excluded.match_rule,
            accepts_automated = excluded.accepts_automated,
            enabled = excluded.enabled,
            updated_at = excluded.updated_at,
            version = preservation_destinations.version + 1",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(id)
                .bind(name)
                .bind(base_url)
                .bind(match_rule)
                .bind(accepts)
                .bind(enabled)
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(id)
                .bind(name)
                .bind(base_url)
                .bind(match_rule)
                .bind(accepts)
                .bind(enabled)
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    list_destinations(db)
        .await?
        .into_iter()
        .find(|row| row.id == id)
        .ok_or_else(|| {
            anyhow::anyhow!("the preservation destination {id} was written and then not found")
        })
}

/// Enable or disable a destination without deleting it.
///
/// The reversible action §3.2's own reasoning needs: a destination with paid,
/// verified targets is removed by disabling it, because `ON DELETE RESTRICT`
/// refuses the delete and a forced delete would orphan the credits.
pub async fn set_destination_enabled(db: &Database, id: &str, enabled: bool) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let flag = i64::from(enabled);
    let sql = db.sql(
        "UPDATE preservation_destinations
            SET enabled = ?, updated_at = ?, version = version + 1
          WHERE id = ?",
        "UPDATE preservation_destinations
            SET enabled = $1, updated_at = $2, version = version + 1
          WHERE id = $3",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(flag)
            .bind(&now)
            .bind(id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(flag)
            .bind(&now)
            .bind(id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

// ---------------------------------------------------------------------------
// Targets
// ---------------------------------------------------------------------------

/// Record a crosspost as a preservation target.
///
/// This is the only function that creates a target, and it is idempotent on
/// `(identity_id, destination_id)`: a second crosspost of the same work to the
/// same archive returns the existing row rather than creating a second one that
/// would be paid for again. The unique index on `destination_id` backs the
/// constraint, and this is the read-then-write that turns the index violation
/// into an answer.
///
/// **A member row is only ever created by an act this instance performed** —
/// which is A0's invariant and it holds here for the same reason it held there:
/// this function is called after a crosspost, and there is no constructor
/// anywhere that takes a title, an author or a similarity score. A target is
/// evidence of a thing that happened, not a guess that two things are the same.
///
/// **`work_id` is checked rather than accepted, and that is the whole reason it
/// is a parameter.** The first version took it and ignored it — the compiler said
/// `unused variable: work_id`, which was the correct report. An ignored `work_id`
/// would let a caller pass one work and an identity belonging to a *different*
/// one, and the target would be filed under the identity, so every read
/// (`targets_for_work`, `top_preservers`, the clawback) would report it against
/// the wrong work and pay the wrong reader. Every one of those reads joins
/// through `story_identities.work_id`, so the mismatch is invisible at the write
/// and visible only as a reward that went to somebody who preserved something
/// else. The check is one row read and it is the difference between the parameter
/// meaning something and merely existing.
pub async fn record_target(
    db: &Database,
    identity_id: &str,
    work_id: &str,
    destination_id: &str,
    external_record_id: &str,
    external_url: Option<&str>,
    created_by: Option<Uuid>,
) -> Result<PreservationTarget> {
    let identity = crate::story_identity::identity_by_id(db, identity_id)
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!("no story identity {identity_id} to record a preservation target on")
        })?;
    if identity.work_id != work_id {
        anyhow::bail!(
            "story identity {identity_id} is about work {} and not work {work_id}; a \
             preservation target filed under it would be counted against the wrong work",
            identity.work_id
        );
    }
    if let Some(existing) = target_for_destination(db, identity_id, destination_id).await? {
        return Ok(existing);
    }
    let member_id = crate::story_identity::record_crossposted_location(
        db,
        identity_id,
        external_record_id,
        Some(destination_id),
        external_url,
    )
    .await?;

    // `destination_id` is set HERE and not left to A0's function, and that is
    // deliberate. `record_crossposted_location` writes `external_source_key`,
    // which is a free-text label for "where else this lives" — it has no
    // foreign key and it is what the edition list renders. `destination_id` is
    // different in kind: it is a reference to a row in
    // `preservation_destinations`, and a target is a target *because* it points
    // at one. The first version of this function relied on A0 setting it, and
    // the symptom was `target_by_member` returning `None` for a row that had
    // just been written — every read here filters on
    // `destination_id IS NOT NULL`, so a target without one is invisible to
    // its own store while the INSERT reported success.
    //
    // So the UPDATE sets all three Phase D columns in one statement: the
    // destination that makes it a target, the timestamp, and the credits index
    // at zero. Setting `credits_paid` explicitly rather than relying on the
    // column default keeps the value correct if a future migration ever changes
    // that default to mean something else.
    let sql = db.sql(
        "UPDATE story_identity_members
            SET destination_id = ?, created_by = ?, updated_at = ?, credits_paid = 0
          WHERE id = ?",
        // `created_by` references `accounts(id)`, which IS a UUID in this
        // schema, so the bind takes the cast. The other two are TEXT columns
        // (0085) bound bare — a `::timestamptz` here is the fault class the
        // handoff records.
        "UPDATE story_identity_members
            SET destination_id = $1, created_by = $2::uuid, updated_at = $3, credits_paid = 0
          WHERE id = $4",
    );
    let now = crate::identity::now_rfc3339();
    let creator = created_by.map(|id| id.to_string());
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(destination_id)
                .bind(creator)
                .bind(&now)
                .bind(&member_id)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(destination_id)
                .bind(creator)
                .bind(&now)
                .bind(&member_id)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }

    target_by_member(db, &member_id).await?.ok_or_else(|| {
        anyhow::anyhow!("the preservation target {member_id} was written and then not found")
    })
}

/// The target for one (identity, destination) pair, or `None`.
pub async fn target_for_destination(
    db: &Database,
    identity_id: &str,
    destination_id: &str,
) -> Result<Option<PreservationTarget>> {
    let sqlite_sql = "SELECT {targets}
               FROM story_identity_members m
              JOIN story_identities i ON i.id = m.identity_id
             WHERE m.identity_id = ? AND m.destination_id = ?";
    let postgres_sql = "SELECT {targets}
               FROM story_identity_members m
              JOIN story_identities i ON i.id = m.identity_id
             WHERE m.identity_id = $1 AND m.destination_id = $2";
    let sql = select_by_dialect(db, &db.sql(sqlite_sql, postgres_sql));
    let row: Option<TargetRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(identity_id)
                .bind(destination_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(identity_id)
                .bind(destination_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(parse_target))
}

/// One target by its member id, or `None`.
pub async fn target_by_member(
    db: &Database,
    member_id: &str,
) -> Result<Option<PreservationTarget>> {
    let sqlite_sql = "SELECT {targets}
               FROM story_identity_members m
              JOIN story_identities i ON i.id = m.identity_id
             WHERE m.id = ? AND m.destination_id IS NOT NULL";
    let postgres_sql = "SELECT {targets}
               FROM story_identity_members m
              JOIN story_identities i ON i.id = m.identity_id
             WHERE m.id = $1 AND m.destination_id IS NOT NULL";
    let sql = select_by_dialect(db, &db.sql(sqlite_sql, postgres_sql));
    let row: Option<TargetRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(member_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(member_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(parse_target))
}

/// Every target recorded for one work, oldest verified first.
///
/// **The order is load-bearing and is not cosmetic.** §2.2's decay is over
/// *position*, so the reward for the next verified destination depends on how
/// many already are, and that count has to come from a stable ordering. Ordered
/// by `verified_at` with `member_id` breaking the tie, because `verified_at` is
/// monotone in the thing being rewarded — when the archive confirmed it — and
/// unlike insertion order it does not move when an unrelated column is updated.
/// A target that has never been verified sorts first among the unverified
/// (NULL), which is irrelevant to the reward because only verified targets
/// count toward it.
pub async fn targets_for_work(db: &Database, work_id: &str) -> Result<Vec<PreservationTarget>> {
    let sqlite_sql = "SELECT {targets}
               FROM story_identity_members m
               JOIN story_identities i ON i.id = m.identity_id
              WHERE i.work_id = ? AND m.destination_id IS NOT NULL
              ORDER BY (m.verified_at IS NULL), m.verified_at, m.id";
    let postgres_sql = "SELECT {targets}
               FROM story_identity_members m
               JOIN story_identities i ON i.id = m.identity_id
              WHERE i.work_id = $1::uuid AND m.destination_id IS NOT NULL
              ORDER BY (m.verified_at IS NULL), m.verified_at, m.id";
    let sql = select_by_dialect(db, &db.sql(sqlite_sql, postgres_sql));
    let rows: Vec<TargetRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows.into_iter().map(parse_target).collect())
}

/// How many of a work's targets are currently verified.
///
/// Reads the same rows `targets_for_work` does rather than a `COUNT`, because
/// the reward is computed from the *list* and a count that could disagree with
/// the list by a row written between the two queries would pay a different
/// amount than the list implies. The cost is one query instead of a scalar, and
/// the benefit is that the number and the sequence cannot disagree.
pub async fn verified_count_for_work(db: &Database, work_id: &str) -> Result<i64> {
    Ok(targets_for_work(db, work_id)
        .await?
        .iter()
        .filter(|target| target.is_verified())
        .count() as i64)
}

/// Mark a target verified or dead, with its evidence.
///
/// `evidence_hash` is a content hash of the destination page's identifying
/// fields. It is stored rather than the page because the page is the thing this
/// instance must not keep: §11.5's metadata ceiling bounds the read, and a
/// preservation check has no reason to retain what it read once the comparison
/// is done.
///
/// Moving a target to `dead` records `dead_at` and **clears `verified_at`**.
/// That is not a cosmetic choice: `verified_at` is the ordering the reward
/// ladder is built on, and leaving it set on a dead target would mean the next
/// verified destination for that work is positioned by a confirmation that no
/// longer holds.
pub async fn set_state(
    db: &Database,
    member_id: &str,
    state: PreservationState,
    evidence_hash: Option<&str>,
) -> Result<bool> {
    let now = crate::identity::now_rfc3339();
    let (verified_at, dead_at) = match state {
        PreservationState::Verified => (Some(now.as_str()), None),
        PreservationState::Dead => (None, Some(now.as_str())),
        // Unverified and refused carry neither timestamp: an unverified target
        // was never confirmed and a refused one was never attempted.
        _ => (None, None),
    };
    let sql = db.sql(
        "UPDATE story_identity_members
            SET state = ?, verified_at = ?, dead_at = ?, evidence_hash = ?,
                updated_at = ?, version = version + 1
          WHERE id = ? AND destination_id IS NOT NULL",
        "UPDATE story_identity_members
            SET state = $1, verified_at = $2, dead_at = $3, evidence_hash = $4,
                updated_at = $5, version = version + 1
          WHERE id = $6 AND destination_id IS NOT NULL",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(state.as_str())
            .bind(verified_at)
            .bind(dead_at)
            .bind(evidence_hash)
            .bind(&now)
            .bind(member_id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(state.as_str())
            .bind(verified_at)
            .bind(dead_at)
            .bind(evidence_hash)
            .bind(&now)
            .bind(member_id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// Record the credits paid for a target on the member.
///
/// **Separate from the ledger write and deliberately so.** The ledger entry is
/// the auditable fact; this column is the *index* into it, so the clawback can
/// find what to reverse without scanning the whole ledger for a reference. If
/// the two could disagree, the clawback would either reverse nothing or reverse
/// twice, and both are worse than an operator re-running a grant.
pub async fn set_credits_paid(db: &Database, member_id: &str, credits: i64) -> Result<bool> {
    let sql = db.sql(
        "UPDATE story_identity_members
            SET credits_paid = ?, updated_at = ?, version = version + 1
          WHERE id = ? AND destination_id IS NOT NULL",
        "UPDATE story_identity_members
            SET credits_paid = $1, updated_at = $2, version = version + 1
          WHERE id = $3 AND destination_id IS NOT NULL",
    );
    let now = crate::identity::now_rfc3339();
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(credits)
            .bind(&now)
            .bind(member_id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(credits)
            .bind(&now)
            .bind(member_id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// Every target currently in state `verified`, newest confirmation first.
///
/// The recheck job's input (§2.3). Only `verified` rows are candidates: a
/// target that is `unverified` has nothing to claw back and a `dead` one has
/// already been dealt with, so re-fetching them would spend a network request
/// on a row whose answer is already recorded.
pub async fn verified_targets(db: &Database) -> Result<Vec<PreservationTarget>> {
    let sql = select_by_dialect(
        db,
        "SELECT {targets}
               FROM story_identity_members m
              JOIN story_identities i ON i.id = m.identity_id
             WHERE m.state = 'verified' AND m.destination_id IS NOT NULL
              ORDER BY m.verified_at DESC, m.id",
    );
    let rows: Vec<TargetRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows.into_iter().map(parse_target).collect())
}

/// §2.4's leaderboard metric: distinct preservation destinations currently
/// verified, per reader, over the window.
///
/// **Distinct destinations, never crosspost actions**, and never an all-time
/// total (§9.7.1). A destination that has gone dark stops counting, which is
/// the same anti-farm property the clawback applies, applied to the ranking: a
/// spammer's farms die, the rows leave this count, and the leaderboard corrects
/// itself without a moderator noticing.
///
/// The credit goes to the **work's owner**, not the account that performed the
/// crosspost. §2.1's reward is for preserving *a work*, and the corpus being
/// preserved is the author's. Paying whoever clicked would make the metric a
/// measure of clicking.
pub async fn top_preservers(db: &Database, since: &str, limit: i64) -> Result<Vec<(String, i64)>> {
    let sql = db.sql(
        // `pseuds.owner` is the work's owner (ADR 0003: ownership belongs to
        // the pseud), and `pseuds.account_id` is the column that reaches an
        // account. Grouped by account because a reader who writes under three
        // pseudonyms is one person on a leaderboard.
        "SELECT p.account_id, COUNT(DISTINCT m.destination_id)
           FROM story_identity_members m
           JOIN story_identities i ON i.id = m.identity_id
           JOIN works w ON w.id = i.work_id
           JOIN pseuds p ON p.id = w.owner_pseud_id
          WHERE m.state = 'verified' AND m.verified_at >= ?
          GROUP BY p.account_id
          ORDER BY COUNT(DISTINCT m.destination_id) DESC, p.account_id
          LIMIT ?",
        "SELECT p.account_id::text, COUNT(DISTINCT m.destination_id)::bigint
           FROM story_identity_members m
           JOIN story_identities i ON i.id = m.identity_id
           JOIN works w ON w.id = i.work_id
           JOIN pseuds p ON p.id = w.owner_pseud_id
          WHERE m.state = 'verified' AND m.verified_at >= $1
          GROUP BY p.account_id
          ORDER BY COUNT(DISTINCT m.destination_id) DESC, p.account_id
          LIMIT $2",
    );
    match db.backend() {
        Backend::Sqlite => Ok(sqlx::query_as(&sql)
            .bind(since)
            .bind(limit)
            .fetch_all(db.sqlite_pool().expect("sqlite handle"))
            .await?),
        Backend::Postgres => Ok(sqlx::query_as(&sql)
            .bind(since)
            .bind(limit)
            .fetch_all(db.postgres_pool().expect("postgres handle"))
            .await?),
    }
}

/// Whether this work is an import — which §2.7 refuses to preserve.
///
/// **A query and not a column, and the reason is in migration 0093**: an
/// imported work is one with a `library_items` row pointing at it, and the
/// moment that import is deleted the work stops being imported. A
/// `works.imported` column would keep answering "imported" for a work whose
/// import is gone, and the refusal would be permanent for a work that is now
/// local.
pub async fn is_imported(db: &Database, work_id: &str) -> Result<bool> {
    let sql = db.sql(
        "SELECT EXISTS (SELECT 1 FROM library_items WHERE work_id = ?)",
        "SELECT EXISTS (SELECT 1 FROM library_items WHERE work_id = $1::uuid)",
    );
    Ok(match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    })
}

/// The work's redistribution assertion, as the column holds it.
///
/// Read through the domain's `Redistribution::read`, so an unrecognised value
/// — which SQLite permits and PostgreSQL's CHECK refuses — resolves to the same
/// cautious thing on both engines.
pub async fn read_redistribution(db: &Database, work_id: &str) -> Result<Redistribution> {
    let sql = db.sql(
        "SELECT redistribution FROM works WHERE id = ?",
        "SELECT redistribution::text FROM works WHERE id = $1::uuid",
    );
    let value: Option<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(Redistribution::read(value.as_deref()))
}

/// Record a work's redistribution assertion.
///
/// Only the owning pseud may set it, and the check is in the caller — the same
/// shape `works.rs` uses for every other author-editable column, because the
/// store is not the door and a store that re-derives ownership would have to
/// answer a question the door already answered.
pub async fn write_redistribution(
    db: &Database,
    work_id: &str,
    value: Redistribution,
) -> Result<()> {
    let sql = db.sql(
        "UPDATE works SET redistribution = ?, updated_at = ?, version = version + 1 WHERE id = ?",
        "UPDATE works SET redistribution = $1, updated_at = $2, version = version + 1 WHERE id = $3::uuid",
    );
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(value.as_str())
                .bind(&now)
                .bind(work_id)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(value.as_str())
                .bind(&now)
                .bind(work_id)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Credits: the grant and the clawback
// ---------------------------------------------------------------------------

/// What a grant or a clawback did, so a caller can report it without
/// re-deriving it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerOutcome {
    /// A new entry was posted. `credits` is the amount.
    Posted { credits: i64 },
    /// The idempotency key had already been used; the existing entry stands.
    ///
    /// A distinct answer from `Posted { credits: 0 }`, and deliberately so: the
    /// first means "this target is now worth nothing" and the second means "this
    /// target already paid and I am not paying again". A caller reporting the
    /// first as the second would tell a reader their fourth destination earned
    /// nothing when it in fact earned a quarter of full, an hour ago.
    AlreadyPosted { credits: i64 },
}

impl LedgerOutcome {
    /// The amount, whichever way the call went.
    #[must_use]
    pub const fn credits(self) -> i64 {
        match self {
            Self::Posted { credits } | Self::AlreadyPosted { credits } => credits,
        }
    }

    /// Whether this call actually wrote a ledger entry.
    #[must_use]
    pub const fn wrote(self) -> bool {
        matches!(self, Self::Posted { .. })
    }
}

/// The account a work's preservation credits are paid to.
///
/// §2.1 pays for preserving a *work*, and the corpus being preserved is the
/// author's — so the destination is the work's owner, not the account that
/// performed the crosspost. Paying whoever clicked would make the reward a
/// measure of clicking, and it would pay a stranger to preserve somebody else's
/// work.
///
/// A work with no owner pseud cannot happen (`works.owner_pseud_id` is NOT
/// NULL), so this returns an error rather than an `Option`: a work that exists
/// and cannot name an owner is a corrupt row, and silently paying nobody would
/// make the reward vanish without a trace.
pub async fn reward_account_for_work(db: &Database, work_id: &str) -> Result<String> {
    let sql = db.sql(
        "SELECT p.account_id FROM works w JOIN pseuds p ON p.id = w.owner_pseud_id WHERE w.id = ?",
        "SELECT p.account_id::text FROM works w JOIN pseuds p ON p.id = w.owner_pseud_id WHERE w.id = $1::uuid",
    );
    let row: Option<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(work_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    row.ok_or_else(|| {
        anyhow::anyhow!("work {work_id} has no owner pseud to pay its preservation reward to")
    })
}

/// Pay a verified target's reward, exactly once.
///
/// **Two entries are never balanced against each other here, and the grant is
/// a single-sided entry.** `post_transaction` takes a list of entries and
/// `entries_balanced` exists for double-entry bookkeeping; the grant posts one
/// positive entry into `credits` with no offsetting debit, because the
/// *source* of credits in this system is not another account. That is
/// deliberate and it is what the clawback has to mirror: the clawback posts a
/// single negative entry, so the pair reads as "paid N" then "reclaimed N" and a
/// reader who spent the N sees the debt appear as its own row.
///
/// The idempotency key is `preservation:grant:<member_id>`, so a replayed
/// verification — a retried job, a double-clicked button, a recheck that
/// re-confirms a live target — returns `AlreadyPosted` and pays nothing.
///
/// A zero-credit reward is **not** posted at all. A destination past the cap
/// "records and displays and pays nothing" (§2.2), and posting a zero entry
/// would be a ledger row with no meaning that a reader's statement has to
/// render and explain.
pub async fn grant_credits(
    db: &Database,
    member_id: &str,
    account: &str,
    credits: i64,
) -> Result<LedgerOutcome> {
    if credits <= 0 {
        return Ok(LedgerOutcome::Posted { credits: 0 });
    }
    let key = format!("preservation:grant:{member_id}");
    let already = ledger_entry_for(db, &key).await?;
    if let Some(amount) = already {
        return Ok(LedgerOutcome::AlreadyPosted { credits: amount });
    }
    crate::economy::post_transaction(
        db,
        TxnType::Preservation,
        &key,
        member_id,
        &[(account.to_owned(), "credits".to_owned(), credits)],
    )
    .await?;
    set_credits_paid(db, member_id, credits).await?;
    Ok(LedgerOutcome::Posted { credits })
}

/// Take back a dead target's reward, exactly once.
///
/// **A second ledger entry and never a balance edit.** §2.3 requires it and the
/// reason is reader-facing: a reader who has already *spent* the credits has to
/// see that they now owe them, and a balance mutation cannot show a debt —
/// it just makes the number smaller, indistinguishable from having never been
/// paid. So the clawback posts `preservation_reclaim` with a negative amount
/// and the pair is legible in a statement.
///
/// The amount is read from `credits_paid` on the member rather than recomputed
/// from the reward policy. Recomputing would be wrong the moment a policy
/// changes: the clawback would take back what the *current* policy says a dead
/// target is worth rather than what it was actually *paid*, and a policy change
/// would silently confiscate balances.
///
/// A member with `credits_paid = 0` posts nothing. That is the cap case and the
/// never-verified case, and in both the correct reversal is "nothing", which a
/// zero row would only obscure.
pub async fn reclaim_credits(db: &Database, member_id: &str) -> Result<LedgerOutcome> {
    let paid = match target_by_member(db, member_id).await? {
        Some(target) => target.credits_paid,
        None => return Ok(LedgerOutcome::Posted { credits: 0 }),
    };
    if paid <= 0 {
        return Ok(LedgerOutcome::Posted { credits: 0 });
    }
    let key = format!("preservation:reclaim:{member_id}");
    let account = account_for_paid_credits(db, member_id).await?;
    let already = ledger_entry_for(db, &key).await?;
    if let Some(amount) = already {
        return Ok(LedgerOutcome::AlreadyPosted { credits: -amount });
    }
    crate::economy::post_transaction(
        db,
        TxnType::PreservationReclaim,
        &key,
        member_id,
        &[(account, "credits".to_owned(), -paid)],
    )
    .await?;
    // `credits_paid` is set to 0 rather than left at the paid amount: it is the
    // index the clawback reads, and leaving it would make a second recheck
    // reverse the same credits again. The ledger keeps the record; this column
    // tracks what is still outstanding.
    set_credits_paid(db, member_id, 0).await?;
    Ok(LedgerOutcome::Posted { credits: -paid })
}

/// The account a paid grant went to, read back from the ledger.
///
/// **Read from the entry rather than recomputed from the work's current
/// owner**, and the reason is the same as the amount: works change hands. If a
/// work is transferred, recomputing would claw back the *new* owner's credits —
/// taking money from somebody who was never paid. The ledger entry names who
/// was, and that is the only honest answer.
///
/// Returns an error when there is no grant to read, which the caller has
/// already established by `credits_paid > 0`.
async fn account_for_paid_credits(db: &Database, member_id: &str) -> Result<String> {
    let sql = db.sql(
        "SELECT account FROM credit_entries WHERE transaction_id =
           (SELECT id FROM credit_transactions WHERE idempotency_key = ?)",
        "SELECT account::text FROM credit_entries WHERE transaction_id =
           (SELECT id FROM credit_transactions WHERE idempotency_key = $1)",
    );
    let row: Option<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(format!("preservation:grant:{member_id}"))
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(format!("preservation:grant:{member_id}"))
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    row.ok_or_else(|| {
        anyhow::anyhow!(
            "target {member_id} records credits_paid > 0 but no ledger entry \
             preservation:grant:{member_id} exists; the two are written together and \
             a disagreement between them is a bug, not a state to recover from"
        )
    })
}

/// The amount already posted under an idempotency key, or `None`.
///
/// `amount_bp` on `credit_entries` despite the name holds whole credits
/// throughout this codebase — `post_transaction` binds the caller's `i64`
/// straight into it. Reading it back as the amount is therefore correct, and the
/// column name is a historical artefact rather than a scale to divide by.
async fn ledger_entry_for(db: &Database, key: &str) -> Result<Option<i64>> {
    let sql = db.sql(
        "SELECT e.amount_bp FROM credit_entries e
           JOIN credit_transactions t ON t.id = e.transaction_id
          WHERE t.idempotency_key = ?",
        "SELECT e.amount_bp::bigint AS amount_bp FROM credit_entries e
           JOIN credit_transactions t ON t.id = e.transaction_id
          WHERE t.idempotency_key = $1",
    );
    match db.backend() {
        Backend::Sqlite => Ok(sqlx::query_scalar(&sql)
            .bind(key)
            .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
            .await?),
        Backend::Postgres => Ok(sqlx::query_scalar(&sql)
            .bind(key)
            .fetch_optional(db.postgres_pool().expect("postgres handle"))
            .await?),
    }
}
