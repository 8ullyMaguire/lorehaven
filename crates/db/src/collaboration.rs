//! Contributors and collaboration invitations (spec §4.3, §8).
//!
//! The rule that shapes this module: **a contributor is a pseud, not an
//! account.** An invitation names the pseud being invited and the pseud doing
//! the inviting, and an accepted invitation grants rights to exactly that
//! pseud. Nothing here ever returns an account identifier, because a
//! collaboration surface that leaked one would undo the pseud isolation the
//! rest of the platform maintains (ADR 0003).
//!
//! Inviting by pseud also means a person may be invited under one face and
//! accept under that face only; their other pseuds stay unlinked.

use anyhow::{Context, Result};
use sqlx::FromRow;

use lorehaven_domain::content::{Contributor, ContributorRole};
use lorehaven_domain::{CollaborationInviteId, PseudId, WorkId};

use crate::identity::now_rfc3339;
use crate::{sql_owned, Backend, Database};

/// A contributor row as stored.
#[derive(Debug, Clone, FromRow)]
struct ContributorRow {
    pseud_id: String,
    role: String,
    public_attribution: i64,
}

/// Contributors of a work, in role order (owner first).
///
/// An unrecognised stored role is dropped rather than defaulted: a row whose
/// role we cannot read must not silently become an owner (see
/// [`ContributorRole::parse`]).
pub async fn contributors_for_work(db: &Database, work: WorkId) -> Result<Vec<Contributor>> {
    let sql = db.sql(
        "SELECT pseud_id, role, public_attribution FROM work_contributors
          WHERE work_id = ?
          ORDER BY CASE role WHEN 'owner' THEN 0 WHEN 'coauthor' THEN 1
                             WHEN 'editor' THEN 2 ELSE 3 END, created_at ASC",
        "SELECT pseud_id::text AS pseud_id, role, public_attribution FROM work_contributors
          WHERE work_id = ?::uuid
          ORDER BY CASE role WHEN 'owner' THEN 0 WHEN 'coauthor' THEN 1
                             WHEN 'editor' THEN 2 ELSE 3 END, created_at ASC",
    );

    let rows: Vec<ContributorRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let role = ContributorRole::parse(&row.role)?;
            let pseud_id = row.pseud_id.parse().ok()?;
            Some(Contributor {
                pseud_id,
                role,
                public_attribution: row.public_attribution != 0,
            })
        })
        .collect())
}

/// The pseud that owns a work.
pub async fn owner_of(db: &Database, work: WorkId) -> Result<Option<PseudId>> {
    let sql = db.sql(
        "SELECT pseud_id FROM work_contributors WHERE work_id = ? AND role = 'owner'",
        "SELECT pseud_id::text AS pseud_id FROM work_contributors
          WHERE work_id = ?::uuid AND role = 'owner'",
    );
    let row: Option<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.and_then(|(raw,)| raw.parse().ok()))
}

/// Contributors who may be credited publicly, as a work page shows them.
///
/// Deliberately returns the *pseud's own* public fields and nothing else: no
/// account, no private co-author, no invitation state.
pub async fn public_contributors(
    db: &Database,
    work: WorkId,
) -> Result<Vec<(String, String, String)>> {
    let sql = db.sql(
        "SELECT p.handle, p.display_name, wc.role
           FROM work_contributors wc
           JOIN pseuds p ON p.id = wc.pseud_id
          WHERE wc.work_id = ? AND wc.public_attribution = 1 AND p.deleted_at IS NULL
          ORDER BY CASE wc.role WHEN 'owner' THEN 0 WHEN 'coauthor' THEN 1
                                WHEN 'editor' THEN 2 ELSE 3 END, wc.created_at ASC",
        "SELECT p.handle, p.display_name, wc.role
           FROM work_contributors wc
           JOIN pseuds p ON p.id = wc.pseud_id
          WHERE wc.work_id = ?::uuid AND wc.public_attribution = 1 AND p.deleted_at IS NULL
          ORDER BY CASE wc.role WHEN 'owner' THEN 0 WHEN 'coauthor' THEN 1
                                WHEN 'editor' THEN 2 ELSE 3 END, wc.created_at ASC",
    );

    let rows: Vec<(String, String, String)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows)
}

/// Grant a pseud a role on a work, or replace the role it already holds.
pub async fn add_contributor(
    db: &Database,
    work: WorkId,
    pseud: PseudId,
    role: ContributorRole,
    public_attribution: bool,
) -> Result<()> {
    let now = now_rfc3339();
    let sql = db.sql(
        "INSERT INTO work_contributors (work_id, pseud_id, role, public_attribution, created_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (work_id, pseud_id)
         DO UPDATE SET role = excluded.role, public_attribution = excluded.public_attribution",
        "INSERT INTO work_contributors (work_id, pseud_id, role, public_attribution, created_at)
         VALUES (?::uuid, ?::uuid, ?, ?, ?)
         ON CONFLICT (work_id, pseud_id)
         DO UPDATE SET role = excluded.role, public_attribution = excluded.public_attribution",
    );

    // The two drivers return different result types, so each arm is a
    // statement rather than the value of the match.
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(work.to_string())
                .bind(pseud.to_string())
                .bind(role.as_str())
                .bind(i64::from(public_attribution))
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(work.to_string())
                .bind(pseud.to_string())
                .bind(role.as_str())
                .bind(i64::from(public_attribution))
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

/// Change a contributor's role or attribution, or remove them (role `None`).
///
/// The owner row is protected: a work whose owner was removed would be
/// unmanageable and unpublishable.
pub async fn update_contributor(
    db: &Database,
    work: WorkId,
    pseud: PseudId,
    role: Option<ContributorRole>,
    public_attribution: Option<bool>,
) -> Result<bool> {
    let owner = owner_of(db, work).await?;
    if owner == Some(pseud) {
        return Ok(false);
    }

    let sql = db.sql(
        "UPDATE work_contributors
            SET role = COALESCE(?, role),
                public_attribution = COALESCE(?, public_attribution)
          WHERE work_id = ? AND pseud_id = ?",
        "UPDATE work_contributors
            SET role = COALESCE(?, role),
                public_attribution = COALESCE(?, public_attribution)
          WHERE work_id = ?::uuid AND pseud_id = ?::uuid",
    );
    let flags = public_attribution.map(i64::from);

    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(role.map(ContributorRole::as_str))
            .bind(flags)
            .bind(work.to_string())
            .bind(pseud.to_string())
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(role.map(ContributorRole::as_str))
            .bind(flags)
            .bind(work.to_string())
            .bind(pseud.to_string())
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };

    Ok(affected > 0)
}

/// Remove a contributor.
pub async fn remove_contributor(db: &Database, work: WorkId, pseud: PseudId) -> Result<bool> {
    let owner = owner_of(db, work).await?;
    if owner == Some(pseud) {
        return Ok(false);
    }

    let sql = db.sql(
        "DELETE FROM work_contributors WHERE work_id = ? AND pseud_id = ?",
        "DELETE FROM work_contributors WHERE work_id = ?::uuid AND pseud_id = ?::uuid",
    );

    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(work.to_string())
            .bind(pseud.to_string())
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(work.to_string())
            .bind(pseud.to_string())
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };

    Ok(affected > 0)
}

// ---------------------------------------------------------------------------
// Invitations
// ---------------------------------------------------------------------------

/// An invitation row as stored.
#[derive(Debug, Clone, FromRow)]
struct InviteRow {
    id: String,
    work_id: String,
    work_title: String,
    invited_pseud_id: String,
    invited_handle: String,
    invited_by_handle: String,
    role: String,
    status: String,
    message: Option<String>,
    created_at: String,
    version: i64,
}

/// An invitation, as both sides see it.
#[derive(Debug, Clone)]
pub struct Invite {
    /// Identifier.
    pub id: CollaborationInviteId,
    /// The work.
    pub work_id: WorkId,
    /// Its title, so a list of invitations is readable without a second query.
    pub work_title: String,
    /// The pseud invited.
    pub invited_pseud_id: PseudId,
    /// Its handle.
    pub invited_handle: String,
    /// The pseud that issued the invitation — the "exposed pseud" spec §8
    /// acceptance asks to be identified.
    pub invited_by_handle: String,
    /// The role offered.
    pub role: ContributorRole,
    /// `pending`, `accepted`, `declined`, `revoked` or `expired`.
    pub status: String,
    /// The inviter's message.
    pub message: Option<String>,
    /// Creation time, RFC 3339.
    pub created_at: String,
    /// Optimistic-concurrency version.
    pub version: i64,
}

fn decode_invite(row: InviteRow) -> Option<Invite> {
    Some(Invite {
        id: row.id.parse().ok()?,
        work_id: row.work_id.parse().ok()?,
        work_title: row.work_title,
        invited_pseud_id: row.invited_pseud_id.parse().ok()?,
        invited_handle: row.invited_handle,
        invited_by_handle: row.invited_by_handle,
        role: ContributorRole::parse(&row.role)?,
        status: row.status,
        message: row.message,
        created_at: row.created_at,
        version: row.version,
    })
}

/// Invite a pseud to a work.
pub async fn create_invite(
    db: &Database,
    work: WorkId,
    invited: PseudId,
    invited_by: PseudId,
    role: ContributorRole,
    token_hash: &str,
    message: Option<&str>,
) -> Result<CollaborationInviteId> {
    let id = CollaborationInviteId::new();
    let now = now_rfc3339();
    let sql = db.sql(
        "INSERT INTO collaboration_invites
             (id, work_id, invited_pseud_id, invited_by_pseud_id, role, status, token_hash,
              message, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, 'pending', ?, ?, ?, ?, 1)",
        "INSERT INTO collaboration_invites
             (id, work_id, invited_pseud_id, invited_by_pseud_id, role, status, token_hash,
              message, created_at, updated_at, version)
         VALUES (?::uuid, ?::uuid, ?::uuid, ?::uuid, ?, 'pending', ?, ?, ?, ?, 1)",
    );

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(id.to_string())
                .bind(work.to_string())
                .bind(invited.to_string())
                .bind(invited_by.to_string())
                .bind(role.as_str())
                .bind(token_hash)
                .bind(message)
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await
                .context("inserting collaboration invite")?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(id.to_string())
                .bind(work.to_string())
                .bind(invited.to_string())
                .bind(invited_by.to_string())
                .bind(role.as_str())
                .bind(token_hash)
                .bind(message)
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await
                .context("inserting collaboration invite")?;
        }
    }

    Ok(id)
}

const INVITE_COLUMNS: &str = "i.id AS id, i.work_id AS work_id, w.title AS work_title,
    i.invited_pseud_id AS invited_pseud_id, invited.handle AS invited_handle,
    inviter.handle AS invited_by_handle, i.role AS role, i.status AS status,
    i.message AS message, i.created_at AS created_at, i.version AS version";

const INVITE_COLUMNS_PG: &str =
    "i.id::text AS id, i.work_id::text AS work_id, w.title AS work_title,
    i.invited_pseud_id::text AS invited_pseud_id, invited.handle AS invited_handle,
    inviter.handle AS invited_by_handle, i.role, i.status,
    i.message, i.created_at, i.version";

const INVITE_JOINS: &str = "FROM collaboration_invites i
    JOIN works w ON w.id = i.work_id
    JOIN pseuds invited ON invited.id = i.invited_pseud_id
    JOIN pseuds inviter ON inviter.id = i.invited_by_pseud_id";

/// Invitations issued for a work, newest first.
pub async fn invites_for_work(db: &Database, work: WorkId) -> Result<Vec<Invite>> {
    let sql = sql_owned(
        db,
        format!("SELECT {INVITE_COLUMNS} {INVITE_JOINS} WHERE i.work_id = ? ORDER BY i.created_at DESC"),
        format!(
            "SELECT {INVITE_COLUMNS_PG} {INVITE_JOINS} WHERE i.work_id = ?::uuid ORDER BY i.created_at DESC"
        ),
    );

    let rows: Vec<InviteRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows.into_iter().filter_map(decode_invite).collect())
}

/// Invitations waiting on a pseud.
pub async fn pending_invites_for_pseud(db: &Database, pseud: PseudId) -> Result<Vec<Invite>> {
    let sql = sql_owned(
        db,
        format!(
            "SELECT {INVITE_COLUMNS} {INVITE_JOINS}
              WHERE i.invited_pseud_id = ? AND i.status = 'pending' ORDER BY i.created_at DESC"
        ),
        format!(
            "SELECT {INVITE_COLUMNS_PG} {INVITE_JOINS}
              WHERE i.invited_pseud_id = ?::uuid AND i.status = 'pending' ORDER BY i.created_at DESC"
        ),
    );

    let rows: Vec<InviteRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(pseud.to_string())
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(pseud.to_string())
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(rows.into_iter().filter_map(decode_invite).collect())
}

/// Load one invitation.
pub async fn find_invite(db: &Database, id: CollaborationInviteId) -> Result<Option<Invite>> {
    let sql = sql_owned(
        db,
        format!("SELECT {INVITE_COLUMNS} {INVITE_JOINS} WHERE i.id = ?"),
        format!("SELECT {INVITE_COLUMNS_PG} {INVITE_JOINS} WHERE i.id = ?::uuid"),
    );

    let row: Option<InviteRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(id.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(id.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    Ok(row.and_then(decode_invite))
}

/// Answer an invitation.
///
/// Accepting grants the offered role **and** marks the invitation accepted, in
/// one transaction: an accepted invitation that granted nothing, or a grant
/// with a pending invitation, would both be wrong in a way that is hard to
/// notice later.
pub async fn respond_to_invite(db: &Database, invite: &Invite, accept: bool) -> Result<bool> {
    let now = now_rfc3339();
    let status = if accept { "accepted" } else { "declined" };

    let update_sql = db.sql(
        "UPDATE collaboration_invites
            SET status = ?, responded_at = ?, updated_at = ?, version = version + 1
          WHERE id = ? AND status = 'pending'",
        "UPDATE collaboration_invites
            SET status = ?, responded_at = ?, updated_at = ?, version = version + 1
          WHERE id = ?::uuid AND status = 'pending'",
    );
    let grant_sql = db.sql(
        "INSERT INTO work_contributors (work_id, pseud_id, role, public_attribution, created_at)
         VALUES (?, ?, ?, 1, ?)
         ON CONFLICT (work_id, pseud_id) DO NOTHING",
        "INSERT INTO work_contributors (work_id, pseud_id, role, public_attribution, created_at)
         VALUES (?::uuid, ?::uuid, ?, 1, ?)
         ON CONFLICT (work_id, pseud_id) DO NOTHING",
    );

    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            let mut tx = pool.begin().await?;
            let affected = sqlx::query(&update_sql)
                .bind(status)
                .bind(&now)
                .bind(&now)
                .bind(invite.id.to_string())
                .execute(&mut *tx)
                .await?
                .rows_affected();
            if affected == 0 {
                return Ok(false);
            }
            if accept {
                sqlx::query(&grant_sql)
                    .bind(invite.work_id.to_string())
                    .bind(invite.invited_pseud_id.to_string())
                    .bind(invite.role.as_str())
                    .bind(&now)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            let mut tx = pool.begin().await?;
            let affected = sqlx::query(&update_sql)
                .bind(status)
                .bind(&now)
                .bind(&now)
                .bind(invite.id.to_string())
                .execute(&mut *tx)
                .await?
                .rows_affected();
            if affected == 0 {
                return Ok(false);
            }
            if accept {
                sqlx::query(&grant_sql)
                    .bind(invite.work_id.to_string())
                    .bind(invite.invited_pseud_id.to_string())
                    .bind(invite.role.as_str())
                    .bind(&now)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
    }

    Ok(true)
}

/// Revoke a pending invitation.
pub async fn revoke_invite(db: &Database, id: CollaborationInviteId) -> Result<bool> {
    let now = now_rfc3339();
    let sql = db.sql(
        "UPDATE collaboration_invites
            SET status = 'revoked', updated_at = ?, version = version + 1
          WHERE id = ? AND status = 'pending'",
        "UPDATE collaboration_invites
            SET status = 'revoked', updated_at = ?, version = version + 1
          WHERE id = ?::uuid AND status = 'pending'",
    );

    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(&now)
            .bind(id.to_string())
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(&now)
            .bind(id.to_string())
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };

    Ok(affected > 0)
}
