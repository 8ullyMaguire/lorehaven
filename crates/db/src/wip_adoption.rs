//! §56 — WIP adoption, closure notes and lineage. The records.
//!
//! The rules are [`darkhold_domain::wip_adoption`]; this module is the SQL. There is one thing to
//! know before reading it: **no function here writes `works`**. §56.1 says ownership never moves,
//! and the way that is kept is that no statement in this file mentions `owner_pseud_id` — grep is
//! the proof, and the acceptance test asserts the column's value before and after an adoption
//! rather than trusting this comment.
//!
//! ## Lineage can be unavailable, and that is an answer
//!
//! §56.3 derives lineage from `chapter_revisions.created_by_pseud_id`. On the live instance that
//! table **does not exist** (M63-33 — the deployed schema came from another lineage), so a plain
//! join would answer a `500` about a missing relation. `contributors` therefore asks
//! [`has_table`] first and answers `available: false` with a reason instead — the same shape §58.2
//! uses for a drought that cannot be measured. Nothing in this tree writes a chapter revision yet,
//! so on a fresh database the list is empty rather than unavailable.

use anyhow::Result;
use sqlx::Row;

use darkhold_domain::wip_adoption::{
    may_adopt, next_revision, order_contributors, AdoptionRefusal, Contributor, OfferStatus,
    OpenRefusal, WithdrawalRefusal,
};

use crate::{identity::now_rfc3339, sql_owned, Backend, Database};

/// One adoption offer, as stored.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Offer {
    pub id: String,
    pub work_id: String,
    pub offered_by_pseud_id: String,
    pub author_pseud_id: String,
    pub adopted_by_pseud_id: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

/// The contributor list, and whether it could be read at all.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Lineage {
    pub contributors: Vec<Contributor>,
    pub available: bool,
    /// Present only when `available` is false: why the list cannot be derived here.
    pub reason: Option<String>,
}

/// One closure note revision.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ClosureNote {
    pub work_id: String,
    pub revision_number: i64,
    pub body: String,
    pub author_pseud_id: String,
    pub created_at: String,
}

macro_rules! offer_of {
    ($row:expr) => {
        Offer {
            id: $row.get::<String, _>("id"),
            work_id: $row.get::<String, _>("work_id"),
            offered_by_pseud_id: $row.get::<String, _>("offered_by_pseud_id"),
            author_pseud_id: $row.get::<String, _>("author_pseud_id"),
            adopted_by_pseud_id: $row.get::<Option<String>, _>("adopted_by_pseud_id"),
            status: $row.get::<String, _>("status"),
            created_at: $row.get::<String, _>("created_at"),
            updated_at: $row.get::<String, _>("updated_at"),
        }
    };
}

macro_rules! closure_note_of {
    ($row:expr) => {
        ClosureNote {
            work_id: $row.get::<String, _>("work_id"),
            revision_number: $row.get::<i64, _>("revision_number"),
            body: $row.get::<String, _>("body"),
            author_pseud_id: $row.get::<String, _>("author_pseud_id"),
            created_at: $row.get::<String, _>("created_at"),
        }
    };
}

const OFFER_COLUMNS: &str =
    "id, work_id, offered_by_pseud_id, author_pseud_id, adopted_by_pseud_id, status, created_at, updated_at";

/// Whether this database has `table`. One catalog query; used by [`contributors`] so a missing
/// table is a named answer instead of a `500`.
pub async fn has_table(db: &Database, table: &str) -> Result<bool> {
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query("SELECT COUNT(*) AS n FROM sqlite_master WHERE type = 'table' AND name = ?")
                .bind(table)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(row.get::<i64, _>("n") > 0)
        }
        Backend::Postgres => {
            let row = sqlx::query("SELECT to_regclass($1) IS NOT NULL AS present")
                .bind(format!("public.{table}"))
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(row.get::<bool, _>("present"))
        }
    }
}

/// The work's owner and completion state, or `None` when the work does not exist.
async fn work_facts(db: &Database, work_id: &str) -> Result<Option<(String, String)>> {
    let sql = db.sql(
        "SELECT owner_pseud_id, completion FROM works WHERE id = ?",
        "SELECT owner_pseud_id::text AS owner_pseud_id, completion FROM works WHERE id::text = $1",
    );
    Ok(match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(work_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
            .map(|r| {
                (
                    r.get::<String, _>("owner_pseud_id"),
                    r.get::<String, _>("completion"),
                )
            }),
        Backend::Postgres => sqlx::query(&sql)
            .bind(work_id)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?
            .map(|r| {
                (
                    r.get::<String, _>("owner_pseud_id"),
                    r.get::<String, _>("completion"),
                )
            }),
    })
}

/// The open offer for a work, if any.
pub async fn open_offer_for(db: &Database, work_id: &str) -> Result<Option<Offer>> {
    let sql = sql_owned(
        db,
        format!("SELECT {OFFER_COLUMNS} FROM adoption_offers WHERE work_id = ? AND status = 'open'"),
        format!("SELECT {OFFER_COLUMNS} FROM adoption_offers WHERE work_id = $1 AND status = 'open'"),
    );
    Ok(match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(work_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
            .as_ref()
            .map(|r| offer_of!(r)),
        Backend::Postgres => sqlx::query(&sql)
            .bind(work_id)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?
            .as_ref()
            .map(|r| offer_of!(r)),
    })
}

/// Offer an abandoned work for adoption. The caller must own it.
pub async fn open_offer(
    db: &Database,
    work_id: &str,
    pseud_id: &str,
) -> Result<std::result::Result<Offer, OpenRefusal>> {
    let Some((owner, completion)) = work_facts(db, work_id).await? else {
        return Ok(Err(OpenRefusal::NotOwner));
    };
    if owner != pseud_id {
        return Ok(Err(OpenRefusal::NotOwner));
    }
    // §56.1: `hiatus` is an author who means to come back, and `complete` has nothing to
    // continue. Only `abandoned` is offerable.
    if completion != "abandoned" {
        return Ok(Err(OpenRefusal::NotAbandoned));
    }
    if open_offer_for(db, work_id).await?.is_some() {
        return Ok(Err(OpenRefusal::AlreadyOpen));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = now_rfc3339();
    let sql = db.sql(
        "INSERT INTO adoption_offers (id, work_id, offered_by_pseud_id, author_pseud_id, adopted_by_pseud_id, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, NULL, 'open', ?, ?)",
        "INSERT INTO adoption_offers (id, work_id, offered_by_pseud_id, author_pseud_id, adopted_by_pseud_id, status, created_at, updated_at)
         VALUES ($1, $2, $3, $4, NULL, 'open', $5, $6)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(pseud_id)
                .bind(pseud_id)
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(pseud_id)
                .bind(pseud_id)
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    let offer = open_offer_for(db, work_id)
        .await?
        .expect("the row just written");
    Ok(Ok(offer))
}

/// Withdraw the author's own open offer. §56.1: no notice period, and only the author.
pub async fn withdraw(
    db: &Database,
    offer_id: &str,
    pseud_id: &str,
) -> Result<std::result::Result<Offer, WithdrawalRefusal>> {
    let Some(offer) = get_offer(db, offer_id).await? else {
        return Ok(Err(WithdrawalRefusal::NotOpen));
    };
    if offer.author_pseud_id != pseud_id {
        return Ok(Err(WithdrawalRefusal::NotTheAuthor));
    }
    if offer.status != OfferStatus::Open.as_str() {
        return Ok(Err(WithdrawalRefusal::NotOpen));
    }
    let now = now_rfc3339();
    let sql = db.sql(
        "UPDATE adoption_offers SET status = 'withdrawn', updated_at = ?
         WHERE id = ? AND status = 'open'",
        "UPDATE adoption_offers SET status = 'withdrawn', updated_at = $1
         WHERE id = $2 AND status = 'open'",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&now)
                .bind(offer_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&now)
                .bind(offer_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(Ok(get_offer(db, offer_id)
        .await?
        .expect("the row still exists")))
}

pub async fn get_offer(db: &Database, offer_id: &str) -> Result<Option<Offer>> {
    let sql = sql_owned(
        db,
        format!("SELECT {OFFER_COLUMNS} FROM adoption_offers WHERE id = ?"),
        format!("SELECT {OFFER_COLUMNS} FROM adoption_offers WHERE id = $1"),
    );
    Ok(match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(offer_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
            .as_ref()
            .map(|r| offer_of!(r)),
        Backend::Postgres => sqlx::query(&sql)
            .bind(offer_id)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?
            .as_ref()
            .map(|r| offer_of!(r)),
    })
}

/// Open offers, newest first.
pub async fn list_open(db: &Database, limit: i64) -> Result<Vec<Offer>> {
    let sql = sql_owned(
        db,
        format!(
            "SELECT {OFFER_COLUMNS} FROM adoption_offers WHERE status = 'open'
             ORDER BY created_at DESC, id LIMIT ?"
        ),
        format!(
            "SELECT {OFFER_COLUMNS} FROM adoption_offers WHERE status = 'open'
             ORDER BY created_at DESC, id LIMIT $1"
        ),
    );
    let rows: Vec<Offer> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
                .iter()
                .map(|r| offer_of!(r))
                .collect()
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
                .iter()
                .map(|r| offer_of!(r))
                .collect()
        }
    };
    Ok(rows)
}

/// Adopt an open offer. **Writes nothing to `works`.**
pub async fn adopt(
    db: &Database,
    offer_id: &str,
    pseud_id: &str,
    min_trust_level: i64,
    caller_trust: i64,
) -> Result<std::result::Result<Offer, AdoptionRefusal>> {
    let Some(offer) = get_offer(db, offer_id).await? else {
        return Ok(Err(AdoptionRefusal::NotOpen));
    };
    if offer.status != OfferStatus::Open.as_str() {
        return Ok(Err(AdoptionRefusal::NotOpen));
    }
    let Some((_owner, completion)) = work_facts(db, &offer.work_id).await? else {
        return Ok(Err(AdoptionRefusal::NotAbandoned));
    };
    if completion != "abandoned" {
        return Ok(Err(AdoptionRefusal::NotAbandoned));
    }
    if !may_adopt(min_trust_level, caller_trust) {
        return Ok(Err(AdoptionRefusal::BelowBar {
            bar: min_trust_level,
            caller: caller_trust,
        }));
    }

    let now = now_rfc3339();
    // The whole of §56.1 in one statement: `adopted_by_pseud_id` and `status` change, and
    // `works` is not mentioned.
    let sql = db.sql(
        "UPDATE adoption_offers SET status = 'adopted', adopted_by_pseud_id = ?, updated_at = ?
         WHERE id = ? AND status = 'open'",
        "UPDATE adoption_offers SET status = 'adopted', adopted_by_pseud_id = $1, updated_at = $2
         WHERE id = $3 AND status = 'open'",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(pseud_id)
                .bind(&now)
                .bind(offer_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(pseud_id)
                .bind(&now)
                .bind(offer_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(Ok(get_offer(db, offer_id)
        .await?
        .expect("the row still exists")))
}

/// §56.3: who wrote what, derived from revisions and ordered by first contribution.
pub async fn contributors(db: &Database, work_id: &str) -> Result<Lineage> {
    // 0122 renamed the model: `chapters` is `units` and `chapter_revisions` is `revisions`
    // (§5 — a unit is a part of a work, a revision an append-only version of its text). The
    // lineage source is those two tables; §56.3's wording still says "chapter" because the rule
    // is about the act, not the table.
    if !has_table(db, "revisions").await? {
        return Ok(Lineage {
            contributors: Vec::new(),
            available: false,
            reason: Some(
                "revisions is absent from this instance's schema, so lineage cannot be derived \
                 here (M63-33 reconciles the live schema with the migrations)"
                    .to_string(),
            ),
        });
    }

    // `units` is the join path: a revision belongs to a unit, and a unit to a work. Both are
    // checked, because a revision table without its parent is the same silence.
    if !has_table(db, "units").await? {
        return Ok(Lineage {
            contributors: Vec::new(),
            available: false,
            reason: Some("units is absent from this instance's schema (M63-33)".to_string()),
        });
    }

    let sql = db.sql(
        "SELECT r.created_by_pseud_id AS pseud_id,
                p.handle AS handle,
                MIN(r.created_at) AS first_revision_at,
                COUNT(*) AS revisions
           FROM revisions r
           JOIN units u ON u.id = r.unit_id
           LEFT JOIN pseuds p ON p.id = r.created_by_pseud_id
          WHERE u.work_id = ?
          GROUP BY r.created_by_pseud_id, p.handle",
        "SELECT r.created_by_pseud_id::text AS pseud_id,
                p.handle AS handle,
                MIN(r.created_at) AS first_revision_at,
                COUNT(*) AS revisions
           FROM revisions r
           JOIN units u ON u.id = r.unit_id
           LEFT JOIN pseuds p ON p.id = r.created_by_pseud_id
          WHERE u.work_id::text = $1
          GROUP BY r.created_by_pseud_id, p.handle",
    );
    let rows: Vec<Contributor> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
                .iter()
                .map(|r| Contributor {
                    pseud_id: r.get::<String, _>("pseud_id"),
                    handle: r.get::<Option<String>, _>("handle").unwrap_or_default(),
                    first_revision_at: r.get::<String, _>("first_revision_at"),
                    revisions: r.get::<i64, _>("revisions"),
                })
                .collect()
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
                .iter()
                .map(|r| Contributor {
                    pseud_id: r.get::<String, _>("pseud_id"),
                    handle: r.get::<Option<String>, _>("handle").unwrap_or_default(),
                    first_revision_at: r.get::<String, _>("first_revision_at"),
                    revisions: r.get::<i64, _>("revisions"),
                })
                .collect()
        }
    };

    Ok(Lineage {
        contributors: order_contributors(rows),
        available: true,
        reason: None,
    })
}

/// §56.2: append a closure note revision. Absence of a note is never an error anywhere.
pub async fn put_closure_note(
    db: &Database,
    work_id: &str,
    pseud_id: &str,
    body: &str,
) -> Result<ClosureNote> {
    let current = latest_closure_note(db, work_id)
        .await?
        .map(|n| n.revision_number);
    let revision = next_revision(current);
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_rfc3339();
    let sql = db.sql(
        "INSERT INTO closure_notes (id, work_id, revision_number, body, author_pseud_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
        "INSERT INTO closure_notes (id, work_id, revision_number, body, author_pseud_id, created_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(revision)
                .bind(body)
                .bind(pseud_id)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(work_id)
                .bind(revision)
                .bind(body)
                .bind(pseud_id)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(ClosureNote {
        work_id: work_id.to_string(),
        revision_number: revision,
        body: body.to_string(),
        author_pseud_id: pseud_id.to_string(),
        created_at: now,
    })
}

/// The highest-numbered note, or `None` when the work has none.
pub async fn latest_closure_note(db: &Database, work_id: &str) -> Result<Option<ClosureNote>> {
    let sql = db.sql(
        "SELECT work_id, revision_number, body, author_pseud_id, created_at
           FROM closure_notes WHERE work_id = ?
          ORDER BY revision_number DESC LIMIT 1",
        "SELECT work_id, revision_number, body, author_pseud_id, created_at
           FROM closure_notes WHERE work_id = $1
          ORDER BY revision_number DESC LIMIT 1",
    );
    let row = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(work_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?
            .map(|r| closure_note_of!(r)),
        Backend::Postgres => sqlx::query(&sql)
            .bind(work_id)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?
            .map(|r| closure_note_of!(r)),
    };
    Ok(row)
}
