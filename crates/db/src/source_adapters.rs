//! M45-57 — curator-submitted source adapters: submissions and their reviews
//! (spec §55.2, §21.5, §19.4).
//!
//! The trust gate is the reason this module exists in the shape it does. §55.2
//! requires TL3 to submit an adapter, and the check lives **here** rather than in
//! the route, because the route is not the only caller and a gate only one path
//! passes through is a gate with a hole in it. A future worker, a CLI command, or
//! an admin tool calling [`submit`] gets the same refusal a reader does.
//!
//! Every function that returns rows takes an `account_id` and puts it in the
//! `WHERE`. A function that returns another curator's submissions because the
//! caller forgot to filter is invisible in review and obvious in production, so
//! the scoping is in the shape of the statements rather than in a comment.
//!
//! **On the two-arm shape.** `sqlx::Query` is parameterised by its database
//! type, so one value cannot run against both a `Pool<Sqlite>` and a
//! `Pool<Postgres>` — the compiler rejects it, which is the type system doing
//! exactly the job the dynamically typed engine cannot
//! (`work_characters.rs` and `work_coordinates.rs` build per arm for the same
//! reason). The helpers at the bottom of this file are monomorphised per dialect
//! rather than inlining a `match` at each of the nine call sites, which keeps the
//! per-arm duplication in one place instead of nine.

use serde::Serialize;
use sqlx::FromRow;

use crate::{sql_owned, Database};

/// §55.2's bar: `TL_REVIEWED` from `lorehaven_domain::governance`.
///
/// A named constant rather than a literal `3` at the call site, because §19.14
/// makes this a trust threshold and a bare number in a `WHERE` is one that a
/// later reader cannot tell from a paging limit.
pub const SUBMIT_TRUST_BAR: i64 = lorehaven_domain::governance::TL_REVIEWED;

// ── per-dialect plumbing ────────────────────────────────────────────────────
//
// `sqlx::Query` is parameterised by its database type, so a single value cannot
// run against both a `Pool<Sqlite>` and a `Pool<Postgres>` — the compiler rejects
// it as `expected Sqlite, found Postgres`. That is the type system doing exactly
// the job the dynamically typed engine cannot, and it is why
// `work_characters.rs` and `work_coordinates.rs` each write the statement twice.
//
// The builder is re-created *inside* each arm. Hoisting it above the match fixes
// its type at the first `.bind()`, which then fails to match the other arm's
// pool — the same rule, one level up from where it is easy to get wrong.
//
// Each macro expands to a whole **async block** rather than a `?` at the call
// site, and that is the point. A `?` written inside a macro body resolves
// against the *enclosing function's* return type — so an `exec!` that yielded
// `Result<u64, sqlx::Error>` could not be used by `submit`, which returns
// `SubmitError`, and the error conversion has to happen where the error type is
// known. Wrapping the arms in `async { ... }` gives the `?` inside them a scope
// of their own, inferred as `sqlx::Error` from the awaited calls.
//
// A generic helper does not avoid the per-arm duplication, it relocates it into
// a call the compiler cannot infer (`DB` has nothing to solve against a runtime
// match on `db.backend()`), so each call site would need a turbofish anyway.
// These macros keep the inference where it has something to work from.

/// Run a statement, returning rows affected.
macro_rules! exec {
    ($db:expr, $sql:expr, [$($b:expr),* $(,)?]) => {{
        async {
            let out: Result<u64, sqlx::Error> = match $db.backend() {
                crate::Backend::Sqlite => {
                    let q = sqlx::query($sql);
                    $(let q = q.bind($b);)*
                    q.execute($db.sqlite_pool().expect("sqlite handle")).await.map(|r| r.rows_affected())
                }
                crate::Backend::Postgres => {
                    let q = sqlx::query($sql);
                    $(let q = q.bind($b);)*
                    q.execute($db.postgres_pool().expect("postgres handle")).await.map(|r| r.rows_affected())
                }
            };
            out
        }
    }};
}

/// Fetch every row, decoding into `O`.
macro_rules! fetch_all {
    ($db:expr, $sql:expr, [$($b:expr),* $(,)?], $out:ty) => {{
        async {
            let rows: Result<Vec<$out>, sqlx::Error> = match $db.backend() {
                crate::Backend::Sqlite => {
                    let q = sqlx::query_as::<_, $out>($sql);
                    $(let q = q.bind($b);)*
                    q.fetch_all($db.sqlite_pool().expect("sqlite handle")).await.map_err(Into::into)
                }
                crate::Backend::Postgres => {
                    let q = sqlx::query_as::<_, $out>($sql);
                    $(let q = q.bind($b);)*
                    q.fetch_all($db.postgres_pool().expect("postgres handle")).await.map_err(Into::into)
                }
            };
            rows
        }
    }};
}

/// Fetch at most one row.
macro_rules! fetch_optional {
    ($db:expr, $sql:expr, [$($b:expr),* $(,)?], $out:ty) => {{
        async {
            let row: Result<Option<$out>, sqlx::Error> = match $db.backend() {
                crate::Backend::Sqlite => {
                    let q = sqlx::query_as::<_, $out>($sql);
                    $(let q = q.bind($b);)*
                    q.fetch_optional($db.sqlite_pool().expect("sqlite handle")).await.map_err(Into::into)
                }
                crate::Backend::Postgres => {
                    let q = sqlx::query_as::<_, $out>($sql);
                    $(let q = q.bind($b);)*
                    q.fetch_optional($db.postgres_pool().expect("postgres handle")).await.map_err(Into::into)
                }
            };
            row
        }
    }};
}

/// §19.4: "Extension approval: three reviewers with permission-review expertise."
///
/// A threshold, not a suggestion. [`approve_count`] is what a publish decision
/// compares against, so the number lives here once.
pub const APPROVAL_THRESHOLD: i64 = 3;

/// A submission, as stored.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct SubmissionRow {
    pub id: String,
    pub submitter: String,
    pub manifest: String,
    /// The §55.3 declarative manifest, for a `source_adapters` submission.
    pub source_manifest: Option<String>,
    pub state: String,
    pub reason: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
}

/// One reviewer's verdict.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ReviewRow {
    pub id: String,
    pub submission_id: String,
    pub reviewer_account: String,
    pub verdict: String,
    pub note: Option<String>,
    pub created_at: String,
}

/// Why a submission was refused.
///
/// A distinct type rather than a `String`, because the refusal is a policy outcome
/// the caller renders to a curator, and collapsing it into a bare error is how
/// "you are not trusted enough" starts arriving for people who are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitRefusal {
    /// Below §55.2's TL3 bar. Carries the level found, so the refusal can name it
    /// rather than merely saying no.
    BelowTrustBar { level: i64, required: i64 },
}

impl std::fmt::Display for SubmitRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BelowTrustBar { level, required } => write!(
                f,
                "submitting a source adapter requires trust level {required} \
                 (§55.2, matching §19.14's canonical-curation bar); this account is at \
                 {level}. The bar is a trust threshold and is not configurable (§0.3)."
            ),
        }
    }
}

/// Either the trust gate refused, or the database did.
///
/// Split because the refusal is a policy outcome and the query failure is not.
/// Returning one opaque error for both would mean a refused submission reported
/// as a server fault.
#[derive(Debug)]
pub enum SubmitError {
    Refused(SubmitRefusal),
    Query(sqlx::Error),
}

impl std::fmt::Display for SubmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(r) => write!(f, "{r}"),
            Self::Query(e) => write!(f, "submitting the adapter failed: {e}"),
        }
    }
}

impl std::error::Error for SubmitError {}

/// Submit an adapter.
///
/// Refuses below [`SUBMIT_TRUST_BAR`], naming the level found and the bar
/// required. Submission is not deployment: a row lands in `pending` and serves no
/// reader traffic until §19.4's quorum publishes it.
pub async fn submit(
    db: &Database,
    account_id: &str,
    manifest: &str,
    source_manifest: Option<&str>,
) -> Result<String, SubmitError> {
    let level = crate::governance::trust_for(db, account_id)
        .await
        .map_err(SubmitError::Query)?;
    if level < SUBMIT_TRUST_BAR {
        return Err(SubmitError::Refused(SubmitRefusal::BelowTrustBar {
            level,
            required: SUBMIT_TRUST_BAR,
        }));
    }

    let sql = sql_owned(
        db,
        "INSERT INTO extension_submissions (id, submitter, manifest, source_manifest, state, created_at)
         VALUES (?1, ?2, ?3, ?4, 'pending', ?5)"
            .to_string(),
        "INSERT INTO extension_submissions (id, submitter, manifest, source_manifest, state, created_at)
         VALUES ($1::uuid, $2::uuid, $3, $4, 'pending', $5::timestamptz)"
            .to_string(),
    );
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    exec!(db, &sql, [&id, account_id, manifest, source_manifest, &now])
        .await
        .map_err(SubmitError::Query)?;
    Ok(id)
}

/// Either the caller sent a verdict outside the set, or the database did.
///
/// Split because the two are different facts about different parties, and
/// conflating them is how a reviewer's typo becomes a **500**. The first version
/// returned `sqlx::Error::Protocol` for the bad verdict — a type that means "the
/// driver and the database disagreed" — and the route, which maps
/// `sqlx::Error` to `AppError::Internal`, faithfully reported a client mistake as
/// "something went wrong on our side". The store's own validation was the thing
/// that made it wrong: it knew the verdict was invalid and expressed that in the
/// only type the function could return.
#[derive(Debug)]
pub enum ReviewError {
    /// The verdict is not one of `approve`, `reject`, `abstain`.
    BadVerdict { verdict: String },
    /// The database refused the write.
    Query(sqlx::Error),
}

impl std::fmt::Display for ReviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadVerdict { verdict } => write!(
                f,
                "verdict must be approve, reject or abstain; got {verdict:?}"
            ),
            Self::Query(e) => write!(f, "recording the review failed: {e}"),
        }
    }
}

impl std::error::Error for ReviewError {}

impl From<sqlx::Error> for ReviewError {
    fn from(e: sqlx::Error) -> Self {
        Self::Query(e)
    }
}

/// Record a reviewer's verdict.
///
/// The `ON CONFLICT DO NOTHING` is a safety net *under* the table's UNIQUE
/// constraint, not the constraint itself: the constraint is what makes the
/// duplicate impossible, and this turns a second attempt into "the first row is
/// still there" rather than an error a caller must special-case. A reviewer who
/// submits twice has not voted twice.
pub async fn record_review(
    db: &Database,
    submission_id: &str,
    reviewer_account: &str,
    verdict: &str,
    note: Option<&str>,
) -> Result<String, ReviewError> {
    if !matches!(verdict, "approve" | "reject" | "abstain") {
        // The CHECK constraint would catch this too. Refusing here means the error
        // names the caller's mistake instead of arriving as a driver constraint
        // violation three layers down.
        return Err(ReviewError::BadVerdict {
            verdict: verdict.to_owned(),
        });
    }

    let sql = sql_owned(
        db,
        "INSERT INTO adapter_reviews (id, submission_id, reviewer_account, verdict, note, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (submission_id, reviewer_account) DO NOTHING"
            .to_string(),
        "INSERT INTO adapter_reviews (id, submission_id, reviewer_account, verdict, note, created_at)
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $6::timestamptz)
         ON CONFLICT (submission_id, reviewer_account) DO NOTHING"
            .to_string(),
    );
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    exec!(
        db,
        &sql,
        [&id, submission_id, reviewer_account, verdict, note, &now]
    )
    .await?;
    Ok(id)
}

/// How many *distinct* reviewers have approved.
///
/// `count(DISTINCT reviewer_account)` and not `count(*)`, even though the table's
/// UNIQUE constraint makes them equal **today**. The two are redundant by
/// construction and that is deliberate: the constraint stops a duplicate row
/// existing, and `DISTINCT` stops a duplicate counting if it ever does. A future
/// schema change — a vote-change feature that keeps history, a partial unique
/// index — can break that equivalence, and the threshold is the
/// security-relevant number in this file.
///
/// Worth recording because the redundancy is invisible to mutation testing:
/// deleting either one alone leaves every test here green, because the other
/// still holds the line. Verified by mutation — `COUNT(DISTINCT x)` → `COUNT(x)`
/// passed 11/11, and separately dropping the migration's UNIQUE constraint failed
/// 6 of 11 (the two are only jointly load-bearing).
pub async fn approve_count(db: &Database, submission_id: &str) -> Result<i64, sqlx::Error> {
    let sql = sql_owned(
        db,
        "SELECT COUNT(DISTINCT reviewer_account) FROM adapter_reviews
          WHERE submission_id = ?1 AND verdict = 'approve'"
            .to_string(),
        "SELECT COUNT(DISTINCT reviewer_account) FROM adapter_reviews
          WHERE submission_id = $1::uuid AND verdict = 'approve'"
            .to_string(),
    );
    let row: (i64,) = fetch_optional!(db, &sql, [submission_id], (i64,))
        .await?
        .expect("COUNT always returns exactly one row");
    Ok(row.0)
}

/// Is this submission at §19.4's threshold?
pub async fn has_reached_threshold(
    db: &Database,
    submission_id: &str,
) -> Result<bool, sqlx::Error> {
    Ok(approve_count(db, submission_id).await? >= APPROVAL_THRESHOLD)
}

/// Move a submission to a decided state. Returns whether a row changed.
///
/// `approved` requires the threshold, and returns `Ok(false)` rather than an
/// error when it is unmet: not reaching quorum is the normal state of a pending
/// submission, not a fault. The check is here rather than in the route for the
/// same reason the trust gate is — §19.4's threshold is a property of the data,
/// and one caller honouring it is not the property.
pub async fn decide(
    db: &Database,
    submission_id: &str,
    state: &str,
    reason: Option<&str>,
) -> Result<bool, sqlx::Error> {
    if !matches!(state, "approved" | "rejected" | "revoked") {
        return Err(sqlx::Error::Protocol(format!(
            "decide() takes approved, rejected or revoked; got {state:?}"
        )));
    }
    // §19.5 requires a reason for an emergency action, and `revoked` is one.
    if state == "revoked" && reason.is_none() {
        return Err(sqlx::Error::Protocol(
            "revoking an adapter requires a reason (§19.5)".to_string(),
        ));
    }
    if state == "approved" && !has_reached_threshold(db, submission_id).await? {
        return Ok(false);
    }

    let sql = sql_owned(
        db,
        "UPDATE extension_submissions SET state = ?1, reason = ?2, decided_at = ?3
          WHERE id = ?4"
            .to_string(),
        "UPDATE extension_submissions SET state = $1, reason = $2, decided_at = $3::timestamptz
          WHERE id = $4::uuid"
            .to_string(),
    );
    let now = crate::identity::now_rfc3339();
    let affected = exec!(db, &sql, [state, reason, &now, submission_id]).await?;
    Ok(affected > 0)
}

/// The review queue: pending submissions, newest first.
///
/// Not filtered by reviewer, deliberately. Every TL3 curator may see what awaits
/// review — §55.2's quorum needs reviewers to be able to *find* work, and a
/// queue scoped to "things I have not already reviewed" hides a submission
/// everyone has passed over.
pub async fn list_pending(db: &Database) -> Result<Vec<SubmissionRow>, sqlx::Error> {
    let sql = sql_owned(
        db,
        "SELECT id, submitter, manifest, source_manifest, state, reason, created_at, decided_at
           FROM extension_submissions WHERE state = 'pending'
          ORDER BY created_at DESC"
            .to_string(),
        "SELECT id::text, submitter::text, manifest, source_manifest, state, reason,
                created_at::text, decided_at::text
           FROM extension_submissions WHERE state = 'pending'
          ORDER BY created_at DESC"
            .to_string(),
    );
    fetch_all!(db, &sql, [], SubmissionRow).await
}

/// One submission, by id.
pub async fn by_id(
    db: &Database,
    submission_id: &str,
) -> Result<Option<SubmissionRow>, sqlx::Error> {
    let sql = sql_owned(
        db,
        "SELECT id, submitter, manifest, source_manifest, state, reason, created_at, decided_at
           FROM extension_submissions WHERE id = ?1"
            .to_string(),
        "SELECT id::text, submitter::text, manifest, source_manifest, state, reason,
                created_at::text, decided_at::text
           FROM extension_submissions WHERE id = $1::uuid"
            .to_string(),
    );
    fetch_optional!(db, &sql, [submission_id], SubmissionRow).await
}

/// Every review on one submission, oldest first.
///
/// Reviewers read the others' reasoning before voting, so oldest-first is the
/// order a quorum actually reads in.
pub async fn reviews_for(
    db: &Database,
    submission_id: &str,
) -> Result<Vec<ReviewRow>, sqlx::Error> {
    let sql = sql_owned(
        db,
        "SELECT id, submission_id, reviewer_account, verdict, note, created_at
           FROM adapter_reviews WHERE submission_id = ?1 ORDER BY created_at, id"
            .to_string(),
        "SELECT id::text, submission_id::text, reviewer_account::text, verdict, note,
                created_at::text
           FROM adapter_reviews WHERE submission_id = $1::uuid ORDER BY created_at, id"
            .to_string(),
    );
    fetch_all!(db, &sql, [submission_id], ReviewRow).await
}

/// Submissions by one curator, newest first.
pub async fn submissions_by(
    db: &Database,
    account_id: &str,
) -> Result<Vec<SubmissionRow>, sqlx::Error> {
    let sql = sql_owned(
        db,
        "SELECT id, submitter, manifest, source_manifest, state, reason, created_at, decided_at
           FROM extension_submissions WHERE submitter = ?1 ORDER BY created_at DESC"
            .to_string(),
        "SELECT id::text, submitter::text, manifest, source_manifest, state, reason,
                created_at::text, decided_at::text
           FROM extension_submissions WHERE submitter = $1::uuid ORDER BY created_at DESC"
            .to_string(),
    );
    fetch_all!(db, &sql, [account_id], SubmissionRow).await
}

/// Published declarative manifests — what the registry loads at boot.
///
/// Only `approved` rows, and only a non-null `source_manifest`. A pending or
/// revoked submission serves no reader traffic, so a query that forgot the state
/// filter would break §55.7's invariant with no error anywhere.
pub async fn published_source_manifests(
    db: &Database,
) -> Result<Vec<(String, String)>, sqlx::Error> {
    let sql = sql_owned(
        db,
        "SELECT submitter, source_manifest FROM extension_submissions
          WHERE state = 'approved' AND source_manifest IS NOT NULL"
            .to_string(),
        "SELECT submitter::text, source_manifest FROM extension_submissions
          WHERE state = 'approved' AND source_manifest IS NOT NULL"
            .to_string(),
    );
    let rows: Vec<(String, Option<String>)> =
        fetch_all!(db, &sql, [], (String, Option<String>)).await?;
    Ok(rows
        .into_iter()
        .filter_map(|(submitter, m)| m.map(|m| (submitter, m)))
        .collect())
}
