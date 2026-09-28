//! Cross-source identity (spec §11.10, amended by §11.10b).
//!
//! §11.10 names four tables for grouping the editions of one work across
//! sources. None of them had ever existed: the plan's first draft assumed they
//! did, and a probe found otherwise. What did exist is `library_items` (0006)
//! keyed on `UNIQUE(account_id, source_key, source_work_key)`, which means
//! importing the same fic from two sites produces two unrelated rows and no
//! link between them. A reader who follows the second one to its source learns
//! nothing about the copy sitting in their own library.
//!
//! This module is the whole of Phase A0: the grouping, its members, and the
//! two empty merge tables §3 promises. It is deliberately the minimum Phase D
//! needs, and nothing more.
//!
//! **The one invariant everything else serves: a member row is only ever created
//! from an act this instance performed.** Not from title-and-author similarity,
//! not from a canonical URL, not from a shared tag set. Those are all guesses,
//! and a wrong guess is worse than no link at all — it tells a reader that two
//! different texts are one book, and no later fix removes the wrong linkage a
//! reader already believed. That is why [`EditionRelation`] has exactly one
//! variant and why there is no constructor in this module that takes a title, an
//! author or a URL. Phase D's crosspost is the only caller, and it knows because
//! it did the thing.

use anyhow::Result;
use uuid::Uuid;

use crate::{Backend, Database};

/// How a member came to be part of an identity.
///
/// One variant ships. This is not timidity — it is the only relation this
/// instance can establish *without guessing*, because it performed the act
/// itself. A second variant would be a second opportunity to be wrong, and the
/// migration's CHECK is deliberately not a closed enum over the string: a value
/// the code cannot name is a value the code cannot have produced, which is
/// exactly the property worth keeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditionRelation {
    /// This instance performed a crosspost that created this copy. The only
    /// relation A0 ships, because it is the only one witnessed.
    CrossPosted,
}

impl EditionRelation {
    /// The stored spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CrossPosted => "cross_posted",
        }
    }

    /// Parse a stored spelling, or `None` for a value this build cannot have
    /// written. A member row carrying an unknown relation is refused here rather
    /// than defaulted, because defaulting would silently relabel it.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cross_posted" => Some(Self::CrossPosted),
            _ => None,
        }
    }
}

/// An identity: one work, grouped with the other places it exists.
#[derive(Debug, Clone)]
pub struct StoryIdentity {
    pub id: String,
    /// The local work this identity is about. Always present — an identity with
    /// no local member is a purely-external grouping, and A0 does not create
    /// one, because a reader can never be shown a page for it.
    pub work_id: String,
    /// A copy of the work's title, not the authority. The work's own title is
    /// what a reader sees; this exists so a future merge can record what the
    /// group was called while it was a group, and be dissolved back.
    pub canonical_title: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

/// One edition: either a work on this instance, or a location at a site.
///
/// Note what is absent: there is no body, no excerpt, no content of any kind.
/// §11.10's "do not grant access to another edition's body" is structural —
/// there is nowhere to put one, so a reader holding this instance's copy gains
/// nothing from any member row, and an external member cannot become a way to
/// read text this instance has not fetched.
#[derive(Debug, Clone)]
pub struct IdentityMember {
    pub id: String,
    pub identity_id: String,
    /// Exactly one of `work_id` and `external_record_id` is set; the migration's
    /// CHECK enforces it and [`parse_member`] re-checks it, so a row that
    /// somehow holds both is an error here rather than a reader-facing surprise.
    pub work_id: Option<String>,
    pub external_record_id: Option<String>,
    pub edition_relation: EditionRelation,
    pub external_source_key: Option<String>,
    pub external_url: Option<String>,
    pub created_at: String,
}

impl IdentityMember {
    /// Whether this member is a copy this instance holds.
    pub fn is_local(&self) -> bool {
        self.work_id.is_some()
    }
}

/// A row read back from `story_identity_members`, before the two-halves check.
type MemberRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    String,
);

/// The identity for a work, or `None` when the work has never been crossposted.
///
/// One identity per work, so this is the lookup the work page does. `work_id` is
/// not unique in the table because a future merge may need to dissolve an
/// identity back into its members and re-form it; for now the read is
/// `ORDER BY created_at LIMIT 1` and the tests pin that there is only ever one.
pub async fn identity_for_work(db: &Database, work_id: &str) -> Result<Option<StoryIdentity>> {
    let sql = db.sql(
        "SELECT id, work_id, canonical_title, status, created_at, updated_at, version
           FROM story_identities
          WHERE work_id = ?
          ORDER BY created_at
          LIMIT 1",
        // `?::uuid` on the bind: `story_identities.work_id` is `UUID` (0085),
        // and sqlx binds a `&str` as text, which PostgreSQL will not compare
        // against a UUID column -- `42804: column "work_id" is of type uuid
        // but expression is of type text`. The read side is already cast
        // (`work_id` is read as String, so it is selected as-is and decoded
        // from a UUID column, which sqlx handles).
        "SELECT id, work_id::text, canonical_title::text, status,
                created_at::text, updated_at::text, version::bigint AS version
           FROM story_identities
          WHERE work_id = ?::uuid
          ORDER BY created_at
          LIMIT 1",
    );
    // One match rather than a helper: `Database::pool` is private and the two
    // backends have different pool types, so there is no single handle a query
    // can be handed. This is the shape the rest of this crate uses.
    let row: Option<(String, String, String, String, String, String, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(
        |(id, work_id, canonical_title, status, created_at, updated_at, version)| StoryIdentity {
            id,
            work_id,
            canonical_title,
            status,
            created_at,
            updated_at,
            version,
        },
    ))
}

/// Create the identity for a work, and its local member, in one transaction.
///
/// A work with an identity but no local member would be a group whose own copy
/// is not in it, which is exactly the inference the phase refuses. So the two
/// rows are one statement pair inside one transaction: a reader can never observe
/// the half-built state, and a crash leaves neither.
///
/// Returns the identity id. An identity for this work already existing is
/// returned as-is rather than being a second identity, because a work is one
/// grouping and two would make "the other editions of this work" ambiguous
/// before any reader ever sees it.
pub async fn ensure_identity_for_work(db: &Database, work_id: &str, title: &str) -> Result<String> {
    if let Some(existing) = identity_for_work(db, work_id).await? {
        return Ok(existing.id);
    }

    let now = crate::identity::now_rfc3339();
    let identity_id = Uuid::new_v4().to_string();
    let member_id = Uuid::new_v4().to_string();

    let insert_identity = db.sql(
        "INSERT INTO story_identities
           (id, work_id, canonical_title, status, created_at, updated_at, version)
         VALUES (?, ?, ?, 'active', ?, ?, 1)",
        // `?::uuid` because `work_id` is a UUID column, and the two timestamps
        // are bound BARE because they are `TEXT` in this schema (0085 spells
        // them TEXT to match `works` and every other table). The
        // `::timestamptz` that was here is the class of cast the handoff
        // records as a defect: it produced
        // `operator does not exist: text <= timestamp with time zone`, and
        // `fix-timestamptz-binds.py` was disabled for adding exactly these.
        "INSERT INTO story_identities
           (id, work_id, canonical_title, status, created_at, updated_at, version)
         VALUES (?, ?::uuid, ?, 'active', ?, ?, 1)",
    );
    let insert_member = db.sql(
        "INSERT INTO story_identity_members
           (id, identity_id, work_id, external_record_id, edition_relation, created_at)
         VALUES (?, ?, ?, NULL, 'cross_posted', ?)",
        // Same two corrections as the statement above.
        "INSERT INTO story_identity_members
           (id, identity_id, work_id, external_record_id, edition_relation, created_at)
         VALUES (?, ?, ?::uuid, NULL, 'cross_posted', ?)",
    );

    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            let mut tx = pool.begin().await?;
            sqlx::query(&insert_identity)
                .bind(&identity_id)
                .bind(work_id)
                .bind(title)
                .bind(&now)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            sqlx::query(&insert_member)
                .bind(&member_id)
                .bind(&identity_id)
                .bind(work_id)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            let mut tx = pool.begin().await?;
            sqlx::query(&insert_identity)
                .bind(&identity_id)
                .bind(work_id)
                .bind(title)
                .bind(&now)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            sqlx::query(&insert_member)
                .bind(&member_id)
                .bind(&identity_id)
                .bind(work_id)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
    }
    Ok(identity_id)
}

/// Record an external location this instance has itself crossposted to.
///
/// The name is the whole contract: `record_crossposted_location` says where the
/// knowledge came from, and there is no sibling that says "these look like the
/// same work". A future caller that wants one will have to add it *as a new
/// function*, in the open, rather than reach for a title and a similarity score
/// that is already here.
///
/// The migration's `UNIQUE (identity_id, external_source_key, external_url)`
/// makes a second record of the same location an error rather than a duplicate
/// edition, so this returns the error instead of swallowing it — a caller that
/// re-records an existing crosspost has a bug, and hiding it would make the
/// edition list disagree with what was actually done.
pub async fn record_crossposted_location(
    db: &Database,
    identity_id: &str,
    external_record_id: &str,
    source_key: Option<&str>,
    url: Option<&str>,
) -> Result<String> {
    let member_id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO story_identity_members
           (id, identity_id, work_id, external_record_id, edition_relation,
            external_source_key, external_url, created_at)
         VALUES (?, ?, NULL, ?, 'cross_posted', ?, ?, ?)",
        // `created_at` is a TEXT column (0085) and the bind is an RFC 3339
        // string, so the timestamp is bound bare. `?::timestamptz` here was a
        // defect of the class the handoff records: PostgreSQL rejects it with
        // `operator does not exist: text = timestamp with time zone`. The
        // `work_id` slot is NULL in this statement, so it needs no cast.
        "INSERT INTO story_identity_members
           (id, identity_id, work_id, external_record_id, edition_relation,
            external_source_key, external_url, created_at)
         VALUES (?, ?, NULL, ?, 'cross_posted', ?, ?, ?)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&member_id)
                .bind(identity_id)
                .bind(external_record_id)
                .bind(source_key)
                .bind(url)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&member_id)
                .bind(identity_id)
                .bind(external_record_id)
                .bind(source_key)
                .bind(url)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(member_id)
}

/// The edition list for an identity: this instance's copy first, then each
/// external location.
///
/// Ordered local-first because that is the order a reader needs — the copy they
/// can actually read, then the ones they cannot. `created_at` breaks the tie
/// among external members so the list is stable across requests; an unstable
/// order on a page is a page that reshuffles on every reload.
pub async fn members_of(db: &Database, identity_id: &str) -> Result<Vec<IdentityMember>> {
    let sql = db.sql(
        "SELECT id, identity_id, work_id, external_record_id, edition_relation,
                external_source_key, external_url, created_at
           FROM story_identity_members
          WHERE identity_id = ?
          ORDER BY (work_id IS NULL), created_at, id",
        "SELECT id, identity_id, work_id::text, external_record_id::text, edition_relation,
                external_source_key::text, external_url::text, created_at::text
           FROM story_identity_members
          WHERE identity_id = ?
          ORDER BY (work_id IS NULL), created_at, id",
    );
    let rows: Vec<MemberRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(identity_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(identity_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    rows.into_iter().map(parse_member).collect()
}

/// Turn a stored row into a member, refusing the two it must never be.
///
/// A row with both halves or neither is a row that means two things, and a
/// reader cannot be shown a row that means two things. The migration's CHECK
/// should have made it impossible, so reaching here means the constraint was
/// dropped or bypassed — either way the honest response is to refuse rather
/// than to pick a half and show it.
fn parse_member(
    (id, identity_id, work_id, external_record_id, relation, source_key, url, created_at): MemberRow,
) -> Result<IdentityMember> {
    let edition_relation = EditionRelation::parse(&relation).ok_or_else(|| {
        anyhow::anyhow!("identity member {id} has edition_relation {relation:?}, which this build cannot have written")
    })?;
    match (work_id.is_some(), external_record_id.is_some()) {
        (true, false) | (false, true) => {}
        (true, true) => {
            anyhow::bail!("identity member {id} names both a local work and an external record")
        }
        (false, false) => {
            anyhow::bail!("identity member {id} names neither a local work nor an external record")
        }
    }
    Ok(IdentityMember {
        id,
        identity_id,
        work_id,
        external_record_id,
        edition_relation,
        external_source_key: source_key,
        external_url: url,
        created_at,
    })
}

/// How many members an identity has, and how many are external.
///
/// Used by the work page to decide between "this work has one copy" and "this
/// work exists in N places", and by the tests that pin the no-inference
/// property: a member count that only ever changes through
/// `record_crossposted_location` is a member count that cannot be inflated by a
/// guess.
pub async fn member_counts(db: &Database, identity_id: &str) -> Result<(i64, i64)> {
    let sql = db.sql(
        "SELECT COUNT(*), COALESCE(SUM(work_id IS NULL), 0)
           FROM story_identity_members WHERE identity_id = ?",
        "SELECT COUNT(*)::bigint,
                COALESCE(SUM((work_id IS NULL)::int), 0)::bigint
           FROM story_identity_members WHERE identity_id = ?",
    );
    let (total, external): (i64, i64) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(identity_id)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(identity_id)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok((total, external))
}

/// Both merge tables are empty, and this is what proves it.
///
/// Not a convenience for a caller — an assertion. §3 promises these tables, a
/// reader browsing the data model will find them, and the moment anything writes
/// to them this returns non-zero and the test that calls it fails. Phase E is
/// what fills them, and it will have to delete this function or invert it,
/// which is the point: a table nobody writes to should be a claim someone made,
/// not a thing that quietly accumulated rows.
pub async fn merge_table_is_empty(db: &Database) -> Result<bool> {
    let sql = db.sql(
        "SELECT (SELECT COUNT(*) FROM identity_merge_proposals)
              + (SELECT COUNT(*) FROM identity_merge_history)",
        "SELECT (SELECT COUNT(*) FROM identity_merge_proposals)::bigint
              + (SELECT COUNT(*) FROM identity_merge_history)::bigint",
    );
    let (n,): (i64,) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(n == 0)
}
