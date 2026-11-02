//! §58.6 — the welcome rota (M45-26).
//!
//! ## The problem
//!
//! §58.2 measures conversation debt and refuses to name anyone until the corpus has
//! some conversation. That is the right refusal, and it leaves a gap: on an instance
//! whose first comment has not been written, nothing asks anybody to write one.
//! The welcome rota is that ask. A reader takes **one** work — finished or already
//! endorsed, and with no conversation — and owes it a first comment within 72 hours.
//!
//! ## What makes it a promise rather than a suggestion
//!
//! Three properties, each of which is a way the feature could be built and be
//! useless:
//!
//! 1. **One open welcome per reader.** `claim` returns the reader's open row instead
//!    of taking a second, so a reader cannot bank five assignments and keep none of
//!    them. The 72 hours is a deadline on one thing.
//! 2. **One welcome per author.** The unit of the promise is an author hearing from
//!    someone. Three comments on three works by the same author is the same author
//!    being welcomed once, so a candidate whose author already has a welcome is not
//!    offered again.
//! 3. **Completion is evidenced, not pressed.** `complete` looks for a comment by
//!    that pseud on that work, created at or after the assignment. With none it
//!    answers [`Complete::NoCommentYet`] and writes nothing. A door that closes the
//!    rota without the comment shows a green count for work that did not happen —
//!    the same defect `supply::complete` carries today (M45-60), which is why this
//!    module does the check that one does not.
//!
//! ## Measured supply, 2026-11-02, on the live catalogue
//!
//! `status = 'complete'` holds 397 rows and `likes > 0` holds 1,247, and the two sets
//! are **disjoint** — the estate import carries publication status with no
//! engagement, the boards carry engagement with no clean status. A rule of
//! "complete AND endorsed" therefore routes nothing on this instance and never
//! would, so the predicate is `likes > 0 OR status = 'complete'`. Of those 1,644
//! rows, **290** carry a `work_id`, and a comment cannot attach to a work without
//! one: that is the real supply until M63-32/M63-33 backfill the ids.

use anyhow::Result;
use serde::Serialize;
use sqlx::Row;

use crate::identity::{in_seconds, now_rfc3339};
use crate::{Backend, Database};

/// The promise: a welcome comes due three days after it is taken.
///
/// Stored in the row rather than derived at read time, so changing the promise
/// cannot silently move a deadline already given.
pub const DUE_HOURS: i64 = 72;

/// One assignment, with the work it names.
#[derive(Debug, Clone, Serialize)]
pub struct Assignment {
    pub id: String,
    pub work_id: String,
    pub title: String,
    pub source_key: String,
    pub author_key: String,
    pub assigned_at: String,
    pub due_at: String,
    pub completed_at: Option<String>,
    /// Computed at read time, never stored: a deadline that passed is a fact about
    /// the clock, not about the row.
    pub overdue: bool,
}

/// What `claim` did.
#[derive(Debug)]
pub enum Claim {
    /// A new welcome was taken.
    Assigned(Assignment),
    /// This reader already owes one; the open row is returned and nothing was taken.
    AlreadyOpen(Assignment),
    /// Nothing on this catalogue needs a welcome right now.
    NothingToWelcome { eligible: i64 },
}

/// What `complete` did.
#[derive(Debug)]
pub enum Complete {
    Done(Assignment),
    /// The work has no comment by this pseud since the assignment. Nothing was written.
    NoCommentYet { work_id: String },
    /// The assignment exists and belongs to someone else.
    NotYours,
    NoSuchAssignment,
}

/// The operator's view. `measured` is false when the catalogue itself is empty, which
/// is different from "every work already has a comment".
#[derive(Debug, Serialize)]
pub struct RotaReport {
    pub measured: bool,
    pub reason: Option<String>,
    /// Works that could take a welcome: finished or endorsed, no comment, with a work id.
    pub eligible: i64,
    pub assigned: i64,
    pub completed: i64,
    pub open: i64,
    pub overdue: i64,
}

/// The content predicate, shared by the candidate pick and the report so the two
/// cannot disagree about what "eligible" means.
///
/// `work_id IS NOT NULL` is not tidiness: `comments.subject_id` holds a work id, so a
/// library row without one can never be given a comment and offering it would be a
/// promise nobody could keep.
const ELIGIBLE: &str = "li.work_id IS NOT NULL
        AND (li.likes > 0 OR li.status = 'complete')
        AND NOT EXISTS (SELECT 1 FROM comments c
                         WHERE c.subject_type = 'work'
                           AND c.subject_id = CAST(li.work_id AS TEXT))";

/// Build an [`Assignment`] from whichever driver's row type arrived.
///
/// The two drivers return different row types and `sqlx::any` is not enabled in this
/// workspace, so each `match db.backend()` arm maps inside itself through this. A
/// shared generic mapper would need `AnyRow`, which this crate deliberately does not
/// use: the engines are not interchangeable (§4), and the mapper is the one place
/// that would pretend they are.
macro_rules! assignment_of {
    ($r:ident) => {{
        let completed_at: Option<String> = $r.get::<Option<String>, _>("completed_at");
        let due_at: String = $r.get::<String, _>("due_at");
        let overdue = completed_at.is_none() && due_at.as_str() < now_rfc3339().as_str();
        Assignment {
            id: $r.get::<String, _>("id"),
            work_id: $r.get::<String, _>("work_id"),
            title: $r.get::<String, _>("title"),
            source_key: $r.get::<String, _>("source_key"),
            author_key: $r.get::<String, _>("author_key"),
            assigned_at: $r.get::<String, _>("assigned_at"),
            due_at,
            completed_at,
            overdue,
        }
    }};
}

const SELECT_ASSIGNMENT: &str = "SELECT r.id, r.work_id, li.title, li.source_key, r.author_key,
        r.assigned_at, r.due_at, r.completed_at
   FROM welcome_rota r
   LEFT JOIN library_items li ON CAST(li.work_id AS TEXT) = r.work_id";

async fn fetch_assignment(db: &Database, id: &str) -> Result<Option<Assignment>> {
    let sql = format!("{SELECT_ASSIGNMENT} WHERE r.id = ?");
    // `?` is SQLite's; PostgreSQL needs `$n`, and sqlx does not rewrite it here.
    // Without this the PG arm answers "syntax error at or near AND" for a correct
    // query (standing trap 11).
    let sql = db.sql(&sql, &sql);
    Ok(match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(&sql)
                .bind(id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            rows.first().map(|r| assignment_of!(r))
        }
        Backend::Postgres => {
            let rows = sqlx::query(&sql)
                .bind(id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            rows.first().map(|r| assignment_of!(r))
        }
    })
}

/// The reader's open welcome, if any.
///
/// Open means `completed_at IS NULL`; an abandoned welcome stays in the table as
/// evidence that it was owed, for the same reason a routing row is never deleted.
pub async fn open_for(db: &Database, pseud_id: &str) -> Result<Option<Assignment>> {
    let sql = format!(
        "{SELECT_ASSIGNMENT} WHERE r.pseud_id = ? AND r.completed_at IS NULL
         ORDER BY r.assigned_at ASC LIMIT 1"
    );
    // `?` is SQLite's; PostgreSQL needs `$n`, and sqlx does not rewrite it here.
    // Without this the PG arm answers "syntax error at or near AND" for a correct
    // query (standing trap 11).
    let sql = db.sql(&sql, &sql);
    Ok(match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(&sql)
                .bind(pseud_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            rows.first().map(|r| assignment_of!(r))
        }
        Backend::Postgres => {
            let rows = sqlx::query(&sql)
                .bind(pseud_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            rows.first().map(|r| assignment_of!(r))
        }
    })
}

/// Everything this reader has been asked to welcome, newest first.
pub async fn mine(db: &Database, pseud_id: &str) -> Result<Vec<Assignment>> {
    let sql = format!(
        "{SELECT_ASSIGNMENT} WHERE r.pseud_id = ?
         ORDER BY (r.completed_at IS NULL) DESC, r.assigned_at DESC LIMIT 50"
    );
    // `?` is SQLite's; PostgreSQL needs `$n`, and sqlx does not rewrite it here.
    // Without this the PG arm answers "syntax error at or near AND" for a correct
    // query (standing trap 11).
    let sql = db.sql(&sql, &sql);
    Ok(match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(&sql)
                .bind(pseud_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            rows.iter().map(|r| assignment_of!(r)).collect()
        }
        Backend::Postgres => {
            let rows = sqlx::query(&sql)
                .bind(pseud_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            rows.iter().map(|r| assignment_of!(r)).collect()
        }
    })
}

/// How many works could take a welcome at all.
async fn eligible_count(db: &Database) -> Result<i64> {
    let sql = format!("SELECT count(*) AS n FROM library_items li WHERE {ELIGIBLE}");
    // `?` is SQLite's; PostgreSQL needs `$n`, and sqlx does not rewrite it here.
    // Without this the PG arm answers "syntax error at or near AND" for a correct
    // query (standing trap 11).
    let sql = db.sql(&sql, &sql);
    Ok(match db.backend() {
        Backend::Sqlite => {
            let r = sqlx::query(&sql)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            r.get::<i64, _>("n")
        }
        Backend::Postgres => {
            let r = sqlx::query(&sql)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?;
            r.get::<i64, _>("n")
        }
    })
}

/// The candidate row, before it becomes an assignment.
struct Candidate {
    work_id: String,
    title: String,
    source_key: String,
    author_key: String,
}

/// Take one work to welcome, or hand back the one already owed.
///
/// The order is deliberate: an endorsed work before a merely finished one (an
/// endorsement is a person who read it, a status flag is a scraped field), then the
/// most endorsed, then the title so two calls in a row agree.
pub async fn claim(db: &Database, pseud_id: &str) -> Result<Claim> {
    if let Some(open) = open_for(db, pseud_id).await? {
        return Ok(Claim::AlreadyOpen(open));
    }

    let candidate_sql = format!(
        "SELECT CAST(li.work_id AS TEXT) AS work_id, li.title, li.source_key,
                lower(trim(li.author_text)) AS author_key
           FROM library_items li
          WHERE {ELIGIBLE}
            AND NOT EXISTS (SELECT 1 FROM welcome_rota r
                             WHERE r.work_id = CAST(li.work_id AS TEXT)
                               AND r.completed_at IS NULL)
            AND (lower(trim(li.author_text)) = ''
                 OR lower(trim(li.author_text)) NOT IN
                    (SELECT author_key FROM welcome_rota WHERE author_key <> ''))
          ORDER BY (li.likes > 0) DESC, CAST(li.likes AS BIGINT) DESC, li.title
          LIMIT 1"
    );
    // `?` is SQLite's; PostgreSQL needs `$n`, and sqlx does not rewrite it here.
    // Without this the PG arm answers "syntax error at or near AND" for a correct
    // query (standing trap 11).
    let candidate_sql = db.sql(&candidate_sql, &candidate_sql);
    let candidate = match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(&candidate_sql)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            rows.first().map(|r| Candidate {
                work_id: r.get::<String, _>("work_id"),
                title: r.get::<String, _>("title"),
                source_key: r.get::<String, _>("source_key"),
                author_key: r.get::<String, _>("author_key"),
            })
        }
        Backend::Postgres => {
            let rows = sqlx::query(&candidate_sql)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            rows.first().map(|r| Candidate {
                work_id: r.get::<String, _>("work_id"),
                title: r.get::<String, _>("title"),
                source_key: r.get::<String, _>("source_key"),
                author_key: r.get::<String, _>("author_key"),
            })
        }
    };
    let Some(candidate) = candidate else {
        return Ok(Claim::NothingToWelcome {
            eligible: eligible_count(db).await?,
        });
    };

    let id = uuid::Uuid::new_v4().to_string();
    let assigned_at = now_rfc3339();
    let due_at = in_seconds(DUE_HOURS * 3600);
    let insert_sql = "INSERT INTO welcome_rota
            (id, work_id, pseud_id, author_key, assigned_at, due_at)
            VALUES (?, ?, ?, ?, ?, ?)";
    // `?` is SQLite's; PostgreSQL needs `$n`, and sqlx does not rewrite it here.
    // Without this the PG arm answers "syntax error at or near AND" for a correct
    // query (standing trap 11).
    let insert_sql = db.sql(insert_sql, insert_sql);
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&insert_sql)
                .bind(&id)
                .bind(&candidate.work_id)
                .bind(pseud_id)
                .bind(&candidate.author_key)
                .bind(&assigned_at)
                .bind(&due_at)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&insert_sql)
                .bind(&id)
                .bind(&candidate.work_id)
                .bind(pseud_id)
                .bind(&candidate.author_key)
                .bind(&assigned_at)
                .bind(&due_at)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }

    Ok(Claim::Assigned(Assignment {
        id,
        work_id: candidate.work_id,
        title: candidate.title,
        source_key: candidate.source_key,
        author_key: candidate.author_key,
        assigned_at,
        due_at,
        completed_at: None,
        overdue: false,
    }))
}

/// Close a welcome, but only on a comment that evidences it.
pub async fn complete(db: &Database, pseud_id: &str, assignment_id: &str) -> Result<Complete> {
    let sql = "SELECT pseud_id, work_id, assigned_at, completed_at
                 FROM welcome_rota WHERE id = ?";
    // `?` is SQLite's; PostgreSQL needs `$n`, and sqlx does not rewrite it here.
    // Without this the PG arm answers "syntax error at or near AND" for a correct
    // query (standing trap 11).
    let sql = db.sql(sql, sql);
    let row = match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(&sql)
                .bind(assignment_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            rows.first().map(|r| {
                (
                    r.get::<String, _>("pseud_id"),
                    r.get::<String, _>("work_id"),
                    r.get::<String, _>("assigned_at"),
                    r.get::<Option<String>, _>("completed_at"),
                )
            })
        }
        Backend::Postgres => {
            let rows = sqlx::query(&sql)
                .bind(assignment_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            rows.first().map(|r| {
                (
                    r.get::<String, _>("pseud_id"),
                    r.get::<String, _>("work_id"),
                    r.get::<String, _>("assigned_at"),
                    r.get::<Option<String>, _>("completed_at"),
                )
            })
        }
    };
    let Some((owner, work_id, assigned_at, already)) = row else {
        return Ok(Complete::NoSuchAssignment);
    };
    if owner != pseud_id {
        return Ok(Complete::NotYours);
    }
    if already.is_some() {
        // Idempotent: the row already carries the evidence, so a second call is a read.
        return Ok(match fetch_assignment(db, assignment_id).await? {
            Some(a) => Complete::Done(a),
            None => Complete::NoSuchAssignment,
        });
    }

    // The evidence: a comment by this pseud on this work, written since the
    // assignment. A comment from before it does not count — the work had no comment
    // when it was offered, which is why it was offered.
    let comment_sql = "SELECT id FROM comments
            WHERE subject_type = 'work' AND subject_id = ? AND author_pseud = ?
              AND created_at >= ?
            ORDER BY created_at ASC LIMIT 1";
    // `?` is SQLite's; PostgreSQL needs `$n`, and sqlx does not rewrite it here.
    // Without this the PG arm answers "syntax error at or near AND" for a correct
    // query (standing trap 11).
    let comment_sql = db.sql(comment_sql, comment_sql);
    let comment_id = match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(&comment_sql)
                .bind(&work_id)
                .bind(pseud_id)
                .bind(&assigned_at)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            rows.first().map(|r| r.get::<String, _>("id"))
        }
        Backend::Postgres => {
            let rows = sqlx::query(&comment_sql)
                .bind(&work_id)
                .bind(pseud_id)
                .bind(&assigned_at)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            rows.first().map(|r| r.get::<String, _>("id"))
        }
    };
    let Some(comment_id) = comment_id else {
        return Ok(Complete::NoCommentYet { work_id });
    };

    let completed_at = now_rfc3339();
    let update_sql = "UPDATE welcome_rota SET completed_at = ?, comment_id = ? WHERE id = ?";
    // `?` is SQLite's; PostgreSQL needs `$n`, and sqlx does not rewrite it here.
    // Without this the PG arm answers "syntax error at or near AND" for a correct
    // query (standing trap 11).
    let update_sql = db.sql(update_sql, update_sql);
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&update_sql)
                .bind(&completed_at)
                .bind(&comment_id)
                .bind(assignment_id)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&update_sql)
                .bind(&completed_at)
                .bind(&comment_id)
                .bind(assignment_id)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }

    Ok(match fetch_assignment(db, assignment_id).await? {
        Some(a) => Complete::Done(a),
        None => Complete::NoSuchAssignment,
    })
}

/// The operator's report: how much need there is, and how the promise is kept.
pub async fn report(db: &Database) -> Result<RotaReport> {
    let corpus_sql = "SELECT count(*) AS n FROM library_items";
    let corpus = match db.backend() {
        Backend::Sqlite => {
            let r = sqlx::query(corpus_sql)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            r.get::<i64, _>("n")
        }
        Backend::Postgres => {
            let r = sqlx::query(corpus_sql)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?;
            r.get::<i64, _>("n")
        }
    };
    if corpus == 0 {
        return Ok(RotaReport {
            measured: false,
            reason: Some(
                "the catalogue holds no library rows, so there is nothing to welcome".to_string(),
            ),
            eligible: 0,
            assigned: 0,
            completed: 0,
            open: 0,
            overdue: 0,
        });
    }

    // SQLite has no FILTER, so the two engines get different statements rather than
    // one statement that is correct on neither.
    let pg_counts = "SELECT CAST(count(*) AS BIGINT) AS assigned,
            CAST(count(*) FILTER (WHERE completed_at IS NOT NULL) AS BIGINT) AS completed,
            CAST(count(*) FILTER (WHERE completed_at IS NULL) AS BIGINT) AS open_rows
       FROM welcome_rota";
    let lite_counts = "SELECT CAST(count(*) AS BIGINT) AS assigned,
            CAST(sum(CASE WHEN completed_at IS NOT NULL THEN 1 ELSE 0 END) AS BIGINT) AS completed,
            CAST(sum(CASE WHEN completed_at IS NULL THEN 1 ELSE 0 END) AS BIGINT) AS open_rows
       FROM welcome_rota";
    let (assigned, completed, open) = match db.backend() {
        Backend::Sqlite => {
            let r = sqlx::query(lite_counts)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            (
                r.get::<i64, _>("assigned"),
                r.get::<i64, _>("completed"),
                r.get::<i64, _>("open_rows"),
            )
        }
        Backend::Postgres => {
            let r = sqlx::query(pg_counts)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?;
            (
                r.get::<i64, _>("assigned"),
                r.get::<i64, _>("completed"),
                r.get::<i64, _>("open_rows"),
            )
        }
    };

    // Overdue is a comparison against the clock in the storage format, so it is a
    // plain indexed range rather than a normalising function in a predicate.
    let now = now_rfc3339();
    let overdue_sql = "SELECT CAST(count(*) AS BIGINT) AS n FROM welcome_rota
            WHERE completed_at IS NULL AND due_at < ?";
    // `?` is SQLite's; PostgreSQL needs `$n`, and sqlx does not rewrite it here.
    // Without this the PG arm answers "syntax error at or near AND" for a correct
    // query (standing trap 11).
    let overdue_sql = db.sql(overdue_sql, overdue_sql);
    let overdue = match db.backend() {
        Backend::Sqlite => {
            let r = sqlx::query(&overdue_sql)
                .bind(&now)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            r.get::<i64, _>("n")
        }
        Backend::Postgres => {
            let r = sqlx::query(&overdue_sql)
                .bind(&now)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?;
            r.get::<i64, _>("n")
        }
    };

    Ok(RotaReport {
        measured: true,
        reason: None,
        eligible: eligible_count(db).await?,
        assigned,
        completed,
        open,
        overdue,
    })
}
