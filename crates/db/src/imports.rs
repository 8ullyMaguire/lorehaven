//! Import repositories: the source catalogue, per-pseud credentials, library
//! items, import jobs and imported chapters (spec §11).
//!
//! Conventions are the same as every other module in this crate: DML is written
//! once with `?` placeholders and the PostgreSQL half carries its casts, reads
//! alias native UUID columns to text, and booleans are `INTEGER` columns decoded
//! as `i64` and compared to zero. See `crate::Database::sql` and ADR 0004.
//!
//! # Two things this module will not do
//!
//! * **It never reads or writes a secret.** `source_credentials` holds a
//!   `secret_id` pointing into `crate::storage`'s sibling `secrets` table, and
//!   the ciphertext is only ever opened by `crates/app/src/secrets.rs` for the
//!   single operation that needs it. There is no function here that returns
//!   credential material, which is the point.
//! * **It never deletes a blob.** A blob is removed only through
//!   [`crate::storage::BlobStore::delete_if_unreferenced`], after the reference
//!   count reaches zero. Nothing in this file cascades a file away.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::identity::now_rfc3339;
use crate::{sql_owned, Backend, Database};

/// Run one statement against whichever driver this handle speaks.
///
/// A macro rather than a function because `sqlx::query`'s type is parameterised
/// by the backend: the SQLite arm and the PostgreSQL arm return different query
/// and result types, so a `match` whose arms both end in `?` will not unify and a
/// shared closure cannot be typed. The body is expanded once per arm, which
/// means the bind chain is written once in the source and cannot drift between
/// the two dialects — the failure ADR 0004's rule exists to prevent.
///
/// It expands to an `async` block so that the caller keeps ownership of the
/// await: `.await` and `.with_context(...)` read the same as they would if this
/// were an ordinary future.
macro_rules! run {
    ($db:expr, $sql:expr, |$query:ident| $body:expr) => {
        async {
            match $db.backend() {
                Backend::Sqlite => {
                    let $query = sqlx::query(&$sql);
                    let affected = ($body)
                        .execute($db.sqlite_pool().expect("sqlite handle"))
                        .await?
                        .rows_affected();
                    Ok::<u64, sqlx::Error>(affected)
                }
                Backend::Postgres => {
                    let $query = sqlx::query(&$sql);
                    let affected = ($body)
                        .execute($db.postgres_pool().expect("postgres handle"))
                        .await?
                        .rows_affected();
                    Ok::<u64, sqlx::Error>(affected)
                }
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// One source, as the operator's catalogue holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    /// The registry key (`ao3`).
    pub key: String,
    /// What a reader sees in the import page's list.
    pub display_name: String,
    /// The adapter version that last parsed this source, for attribution.
    pub adapter_version: String,
    /// Whether imports from this source are allowed.
    pub enabled: bool,
    /// Why not, when it is not. Shown to the reader.
    pub disabled_reason: Option<String>,
    /// The adapter's declared capabilities, as JSON.
    pub capability_json: String,
    /// The rolling health state (spec §11.8).
    pub health: String,
    /// When the source was last probed.
    pub last_checked_at: Option<String>,
    /// Where this build could verify the source from, if it could (M53-04).
    ///
    /// `Some("blocked-here")` is a claim about **this host**, not about the
    /// source: the site may be serving every other reader perfectly while this
    /// machine cannot get past its front door. `None` means the default --
    /// verified, or never claimed either way -- deliberately, because 92
    /// migrations write `sources` rows and a NOT NULL sentinel would either
    /// rewrite all of them or leave every reader handling two spellings of "fine".
    pub verification_status: Option<String>,
    /// Why, in an operator's words. Empty is not an option: a `blocked-here` an
    /// operator cannot act on is a claim nobody can check.
    pub verification_note: Option<String>,
}

impl Source {
    /// The capability document, parsed. Returns an empty document rather than an
    /// error: a source whose capabilities cannot be read is a source with no
    /// advertised capabilities, which is the honest reading and not a fault
    /// worth failing an import over.
    #[must_use]
    pub fn capabilities(&self) -> serde_json::Value {
        serde_json::from_str(&self.capability_json).unwrap_or_else(|_| serde_json::json!({}))
    }
}

/// A reader's private copy of a work from another site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryItem {
    /// Identifier.
    pub id: String,
    /// The account that owns this copy. Never disclosed to anyone else.
    pub account_id: String,
    /// The work this was materialised into, when it was.
    pub work_id: Option<String>,
    /// The source's key.
    pub source_key: String,
    /// The source's own identifier for the work.
    pub source_work_key: String,
    /// Title as the source shows it.
    pub title: String,
    /// Author as the source shows it.
    pub author_text: String,
    /// The author's profile URL, when the source's page carried one.
    pub author_url: Option<String>,
    /// Summary as plain text.
    pub summary: String,
    /// The source's language tag.
    pub language: Option<String>,
    /// The source's word count.
    pub word_count: Option<i64>,
    /// `ongoing`, `complete`, `hiatus`, `cancelled` or `unknown`.
    pub status: String,
    /// The canonical URL at the source.
    pub source_url: String,
    /// When the *source* says the work last changed.
    pub source_updated_at: Option<String>,
    /// When *we* last looked. Kept apart from the line above, because "the
    /// author updated it" and "we checked it" are different facts.
    pub last_synced_at: Option<String>,
    /// How many of this work's chapters are stored and readable.
    ///
    /// Counted, not copied from the source: the source's number is a claim about
    /// the work and this is a fact about the copy. They differ whenever an
    /// import is partial, which is the state a reader most needs to be told.
    pub chapter_count: i64,
    /// Which import produced this and from where.
    pub provenance_json: String,
    /// When this copy was created.
    pub created_at: String,
    /// When this copy was last changed.
    pub updated_at: String,
    /// Optimistic-concurrency version.
    pub version: i64,
}

/// What an import is doing, or did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportJob {
    /// Identifier.
    pub id: String,
    /// The queue row this import rides on.
    pub job_id: String,
    /// The account that asked.
    pub account_id: String,
    /// The pseud that asked, which scopes its credential.
    pub pseud_id: String,
    /// The source's key.
    pub source_key: String,
    /// The URL as pasted.
    pub source_url: String,
    /// `library` or `draft`.
    pub destination_type: String,
    /// Whether this was a rehearsal.
    pub dry_run: bool,
    /// `queued`, `running`, `paused`, `completed`, `failed` or `cancelled`.
    pub state: String,
    /// The library item this produced or updated.
    pub library_item_id: Option<String>,
    /// The per-chapter report, as JSON.
    pub report_json: Option<String>,
    /// When it was asked for.
    pub created_at: String,
    /// When it last changed.
    pub updated_at: String,
}

impl ImportJob {
    /// The report, parsed. `None` when the import has not produced one.
    #[must_use]
    pub fn report(&self) -> Option<serde_json::Value> {
        self.report_json
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok())
    }

    /// Whether the import is still doing something.
    #[must_use]
    pub fn is_live(&self) -> bool {
        matches!(self.state.as_str(), "queued" | "running" | "paused")
    }
}

/// One chapter's import state.
///
/// The `state` vocabulary is deliberately four-valued. `skipped` is the one that
/// matters: a re-import that did not need to re-read a chapter is not a chapter
/// that failed, and a report that conflates them tells a reader the wrong thing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportChapter {
    /// Identifier.
    pub id: String,
    /// The import this chapter belongs to.
    pub import_job_id: String,
    /// The library item it landed in.
    pub library_item_id: Option<String>,
    /// The source's own chapter identifier, or the ordinal when it has none.
    pub source_chapter_key: String,
    /// 1-based position.
    pub ordinal: i64,
    /// Title as the source shows it.
    pub title: String,
    /// `pending`, `stored`, `skipped` or `failed`.
    pub state: String,
    /// The chapter body, by content checksum.
    pub content_blob_checksum: Option<String>,
    /// Set when the chapter was materialised into a work's chapter.
    pub chapter_id: Option<String>,
    /// Why a chapter failed, or what a skip meant.
    pub note: Option<String>,
}

/// A stored connection to a source.
///
/// Carries no secret. `secret_id` names the row in `secrets` that holds the
/// ciphertext, and that row is readable only through the application's secret
/// store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCredential {
    /// Identifier.
    pub id: String,
    /// The pseud this belongs to. Never the account: spec §11.6.
    pub pseud_id: String,
    /// The source's key.
    pub source_key: String,
    /// The row in `secrets` holding the ciphertext.
    pub secret_id: String,
    /// The reader's label for this connection.
    pub label: String,
    /// `active`, `expired`, `rejected` or `revoked`.
    pub status: String,
    /// When it stops being usable.
    pub expires_at: Option<String>,
    /// When we last used it successfully.
    pub last_checked_at: Option<String>,
    /// When it was stored.
    pub created_at: String,
    /// When it last changed.
    pub updated_at: String,
    /// Optimistic-concurrency version.
    pub version: i64,
    /// Which kind of secret this is: `token`, `password` or `session_cookie`.
    ///
    /// `None` means "not recorded", which for a password or cookie means
    /// *un-consented* rather than *consented by omission* — spec §11.6 requires
    /// explicit consent, and a row written before this column existed has no
    /// consent moment to record. [`SourceCredential::is_consented`] is the
    /// reader of this; nothing treats `None` as consent.
    pub kind: Option<String>,
    /// The host this credential is bound to (spec §11.6, "origin-bound
    /// credential use").
    ///
    /// `None` for a row stored before the column existed. We cannot reconstruct
    /// which host it was created for, so such a row is refused for use rather
    /// than tidied into a bound one — see [`SourceCredential::usable_for`].
    pub origin_host: Option<String>,
    /// When the reader consented to this credential being stored.
    ///
    /// Rewritten on every store rather than carried forward, because §11.6's
    /// "re-consent for renewal" is only satisfiable if a renewal is a new act.
    pub consent_at: Option<String>,
}

impl SourceCredential {
    /// Whether this credential may be used right now.
    ///
    /// Both halves are checked: a credential an operator revoked is unusable
    /// however far off its expiry is, and one whose date has passed is unusable
    /// however `active` its status still reads.
    #[must_use]
    pub fn is_usable(&self, now: &str) -> bool {
        if self.status != "active" {
            return false;
        }
        match self.expires_at.as_deref() {
            // RFC 3339 UTC sorts lexicographically, which is why it is stored in
            // this form rather than as a formatted display date.
            Some(expires_at) => expires_at > now,
            None => true,
        }
    }

    /// Whether a consent record exists for this credential.
    ///
    /// Kept separate from [`SourceCredential::usable_for`] because the two answer
    /// different questions, and because consent is the one that is *recorded*
    /// rather than derived: a token needs no consent under §11.6 ("prefer
    /// source-issued tokens"), so a token row may legitimately have none and
    /// still be usable, while a password or cookie row without one is not.
    #[must_use]
    pub fn is_consented(&self) -> bool {
        self.consent_at.is_some()
    }

    /// Why this credential may not be used against `host`, or `None` if it may.
    ///
    /// This is spec §11.6's "origin-bound credential use", and the return type is
    /// the load-bearing part. Returning a bare `bool` would leave every call site
    /// to invent its own refusal text, and the text is the requirement: §11.6
    /// asks for an *actionable* status, and "no" is not one.
    ///
    /// Three ways this can fail, each with a different fix, which is why they are
    /// three cases rather than one:
    ///
    /// - **unbound** — the row predates origin binding, so there is no host to
    ///   check against. The fix is to store it again, which records the origin.
    /// - **wrong host** — a real cookie for a lookalike domain. The fix is to
    ///   store a credential for the source actually being fetched.
    /// - **unconsented** — a password or cookie with no consent record.
    // No `#[must_use]`: `Result` already carries it, and clippy's
    // `double_must_use` is right that the attribute adds nothing here.
    pub fn usable_for(&self, host: &str, now: &str) -> Result<(), CredentialRefusal> {
        if !self.is_usable(now) {
            return Err(CredentialRefusal::Unusable {
                status: self.status.clone(),
            });
        }
        match self.origin_host.as_deref() {
            None => Err(CredentialRefusal::Unbound {
                label: self.label.clone(),
            }),
            Some(bound) if !same_host(bound, host) => Err(CredentialRefusal::WrongHost {
                bound: bound.to_owned(),
                asked: host.to_owned(),
            }),
            Some(_) => {
                // A password or a cookie is the two kinds §11.6 gates behind
                // explicit consent. A token is not: the same section prefers
                // source-issued tokens, so requiring consent for one would mean
                // gating the method the spec recommends.
                if consent_required(self.kind.as_deref()) && !self.is_consented() {
                    return Err(CredentialRefusal::Unconsented {
                        kind: self.kind.clone().unwrap_or_else(|| "unknown".to_owned()),
                    });
                }
                Ok(())
            }
        }
    }
}

/// Why a credential may not be used, in terms the reader can act on.
///
/// Spec §11.6: "Expired credentials pause affected jobs with actionable status."
/// Actionable is doing the work — see [`CredentialRefusal::message`], which is
/// where these become prose, so that no call site has to invent wording that
/// drifts from what the spec asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialRefusal {
    /// Revoked, expired, or otherwise not in a usable state.
    Unusable {
        /// The status the row carries.
        status: String,
    },
    /// Stored before origin binding existed, so no host can be checked.
    Unbound {
        /// The reader's label, so the message can say which one to re-store.
        label: String,
    },
    /// Bound to a different host than the one being fetched.
    WrongHost {
        /// The host it is bound to.
        bound: String,
        /// The host the caller asked for.
        asked: String,
    },
    /// A password or cookie with no consent record.
    Unconsented {
        /// The kind of secret, named so the reader knows what is being asked.
        kind: String,
    },
}

impl CredentialRefusal {
    /// The operator-facing explanation.
    ///
    /// Every arm names the fix. The two this most easily gets wrong are the ones
    /// with no obvious remedy: an unbound credential and an un-consented one
    /// both read as "this credential exists and works" from the outside, so
    /// without a named fix the reader has no idea the credential is being held
    /// back from use.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Unusable { status } => format!(
                "this credential is {status}; revoke it at \
                 DELETE /api/v1/source-credentials/:id and store a new one"
            ),
            Self::Unbound { label } => format!(
                "{label} was stored before this instance recorded which host a \
                 credential belongs to, so it cannot be used against any site. \
                 Store it again at POST /api/v1/source-credentials and it will \
                 be bound to {label}'s source."
            ),
            Self::WrongHost { bound, asked } => format!(
                "this credential is bound to {bound} and will not be sent to \
                 {asked}. Origin binding is why a credential for one site cannot \
                 be replayed against another; store a credential for {asked} \
                 instead."
            ),
            Self::Unconsented { kind } => format!(
                "this {kind} was stored without a recorded consent. §11.6 \
                 requires explicit consent for a password or a session cookie, \
                 so it will not be used until you store it again and confirm."
            ),
        }
    }
}

/// Whether §11.6 requires a consent record for this kind of secret.
///
/// "Password or session-cookie storage requires explicit consent" — and §11.6's
/// first choice is a source-issued token, so gating one would gate the method
/// the spec recommends. An unknown kind is treated as requiring consent, because
/// that is the direction a new kind must earn: the alternative is that adding a
/// fourth `AuthKind` silently starts serving secrets nobody consented to.
#[must_use]
fn consent_required(kind: Option<&str>) -> bool {
    !matches!(kind, Some("token"))
}

/// Whether two host strings are the same origin.
///
/// Lowercased, because a host is case-insensitive by definition and `WWW.Example.COM`
/// is the same site as `www.example.com` — a comparison that called those two
/// different would refuse a legitimate credential on a spelling difference. A
/// port is *not* ignored, because a credential for `example.com:8080` is not
/// automatically a credential for `example.com:443`.
#[must_use]
fn same_host(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

/// Record a source the build knows about, or update its adapter version.
///
/// Called at startup for every registered adapter, so the catalogue a reader
/// sees cannot drift from the adapters the binary actually has. The operator's
/// `enabled` flag and reason are *not* overwritten: a paused source must stay
/// paused across a deploy.
pub async fn upsert_source(
    db: &Database,
    key: &str,
    display_name: &str,
    adapter_version: &str,
    capability_json: &str,
) -> Result<()> {
    let now = now_rfc3339();
    let sql = db.sql(
        "INSERT INTO sources (id, key, display_name, adapter_version, enabled, capability_json,
                              health, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, 1, ?, 'unknown', ?, ?, 1)
         ON CONFLICT (key) DO UPDATE SET
             display_name = excluded.display_name,
             adapter_version = excluded.adapter_version,
             capability_json = excluded.capability_json,
             updated_at = excluded.updated_at,
             version = sources.version + 1",
        "INSERT INTO sources (id, key, display_name, adapter_version, enabled, capability_json,
                              health, created_at, updated_at, version)
         VALUES (?::uuid, ?, ?, ?, 1, ?, 'unknown', ?, ?, 1)
         ON CONFLICT (key) DO UPDATE SET
             display_name = excluded.display_name,
             adapter_version = excluded.adapter_version,
             capability_json = excluded.capability_json,
             updated_at = excluded.updated_at,
             version = sources.version + 1",
    );
    run!(db, &sql, |query| {
        query
            .bind(lorehaven_domain::SourceId::new().to_string())
            .bind(key)
            .bind(display_name)
            .bind(adapter_version)
            .bind(capability_json)
            .bind(&now)
            .bind(&now)
    })
    .await
    .with_context(|| format!("recording the {key} source"))?;
    Ok(())
}

/// Switch a source on or off, with the reader-visible reason.
pub async fn set_source_enabled(
    db: &Database,
    key: &str,
    enabled: bool,
    reason: Option<&str>,
) -> Result<bool> {
    let now = now_rfc3339();
    let sql = db.sql(
        "UPDATE sources SET enabled = ?, disabled_reason = ?, health = ?,
                            updated_at = ?, version = version + 1
         WHERE key = ?",
        "UPDATE sources SET enabled = ?, disabled_reason = ?, health = ?,
                            updated_at = ?, version = version + 1
         WHERE key = ?",
    );
    let health = if enabled { "unknown" } else { "paused" };
    let affected = run!(db, &sql, |query| {
        query
            .bind(i64::from(enabled))
            .bind(reason.map(str::to_owned))
            .bind(health)
            .bind(&now)
            .bind(key)
    })
    .await
    .with_context(|| format!("switching the {key} source"))?;
    Ok(affected > 0)
}

/// Every source in the catalogue, by key.
pub async fn list_sources(db: &Database) -> Result<Vec<Source>> {
    // Named bindings rather than `&format!(...)` inline: `db.sql` returns a
    // borrowed `&str`, and a temporary built inside the call is freed before the
    // borrow is used.
    let sqlite_sql = format!("SELECT {SOURCE_COLUMNS} FROM sources ORDER BY key");
    let postgres_sql = format!("SELECT {SOURCE_COLUMNS_PG} FROM sources ORDER BY key");
    let sql = db.sql(&sqlite_sql, &postgres_sql);
    let rows: Vec<SourceRow> = match db.backend() {
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
    rows.into_iter().map(decode_source).collect()
}

/// Record a source's health after an attempt (spec §11.8).
pub async fn set_source_health(db: &Database, key: &str, health: &str) -> Result<()> {
    let now = now_rfc3339();
    let sql = db.sql(
        "UPDATE sources SET health = ?, last_checked_at = ?, updated_at = ?,
                            version = version + 1
         WHERE key = ?",
        "UPDATE sources SET health = ?, last_checked_at = ?, updated_at = ?,
                            version = version + 1
         WHERE key = ?",
    );
    run!(db, &sql, |query| {
        query.bind(health).bind(&now).bind(&now).bind(key)
    })
    .await
    .with_context(|| format!("recording the {key} source's health"))?;
    Ok(())
}

/// What one source's recent imports say about it, and what that changed.
///
/// Returned so an operator (and a test) can see *why* a source moved rather
/// than only that it moved. A sweep that silently rewrites health is a sweep
/// nobody can debug.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceHealthChange {
    /// The source's key.
    pub key: String,
    /// What the row said before.
    pub previous: String,
    /// What it says now. Equal to `previous` when nothing changed.
    pub current: String,
    /// Finished imports that succeeded inside the window.
    pub completed: i64,
    /// Finished imports that failed inside the window.
    pub failed: i64,
}

/// How far back the sweep looks.
///
/// Seven days, because spec §11.8's question is "is this source working *now*".
/// A failure a month ago is history, and a source that has been broken for a
/// month has been failing recently too.
pub const HEALTH_WINDOW_DAYS: i64 = 7;

/// How many failures, with no success at all, make a source unavailable.
///
/// Three rather than one, because one failure is a page that moved and a reader
/// who pasted a bad URL — the source's health is a claim about the *source*, and
/// a single attempt is evidence about one attempt.
pub const FAILURES_TO_UNAVAILABLE: i64 = 3;

/// The `sources.verification_status` spelling for "this build cannot reach it".
///
/// A constant rather than a bare string in two places, because the value is
/// compared in the health sweep and written by the setter, and a typo in one of
/// them would make a `blocked-here` source quietly eligible for `unavailable`
/// again — which is the exact failure M53-04 exists to prevent, and the one no
/// test in this file would notice.
pub const BLOCKED_HERE: &str = "blocked-here";

/// Record that this build cannot reach a source from where it runs (M53-04).
///
/// Spec §11.8 defines health as a claim about the *source*, and warns against
/// overstating it. This is the other claim: about **this build's relationship
/// with** the source. Keeping them apart is the whole point of the row — a
/// `blocked-here` source is not unhealthy, and saying it were would publish "the
/// site is down" on a public status endpoint when the truth is "we could not
/// check".
///
/// `reason` is required whenever `status` is `blocked-here`, and refused if
/// blank. A `blocked-here` with no explanation is indistinguishable from a source
/// nobody ever checked, and an operator cannot act on either.
pub async fn set_source_verification(
    db: &Database,
    key: &str,
    status: Option<&str>,
    reason: Option<&str>,
) -> Result<()> {
    if status == Some(BLOCKED_HERE) && reason.is_none_or(|note| note.trim().is_empty()) {
        anyhow::bail!(
            "a source cannot be marked blocked-here without saying why: \
             set_source_verification was called with an empty note for {key}"
        );
    }
    let now = now_rfc3339();
    let sql = db.sql(
        "UPDATE sources SET verification_status = ?, verification_note = ?,
                            updated_at = ?, version = version + 1
         WHERE key = ?",
        "UPDATE sources SET verification_status = ?, verification_note = ?,
                            updated_at = ?, version = version + 1
         WHERE key = ?",
    );
    run!(db, &sql, |query| {
        query.bind(status).bind(reason).bind(&now).bind(key)
    })
    .await
    .with_context(|| format!("recording how {key} could be verified"))?;
    Ok(())
}

/// What `recompute_source_health` gives up when it sees a `blocked-here` source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedSource {
    /// The source's key.
    pub key: String,
    /// The operator-facing note that says why this host cannot reach it.
    pub reason: String,
}

/// Recompute every source's health, and say which sources it deliberately did
/// not touch (M53-04).
///
/// # Why the returned list is part of the result
///
/// A sweep that silently skipped three sources would look identical to one that
/// had no reason to skip them. §11.7's "adapter counts are an outcome of verified
/// implementation" is only auditable if the excluded set is visible, and the same
/// argument applies to health: an operator reading "14 sources, all healthy" has
/// to be able to see that one was never examined.
///
/// [`recompute_source_health`] is this function with the list discarded, kept
/// because every existing caller wants exactly that.
pub async fn recompute_source_health_with_skips(
    db: &Database,
    window_days: i64,
) -> Result<(Vec<SourceHealthChange>, Vec<SkippedSource>)> {
    let sources = list_sources(db).await?;
    let since = window_start(window_days);
    let mut changes = Vec::new();
    let mut skipped = Vec::new();

    for source in sources {
        // A pause is a decision, not an observation.
        if source.health == "paused" {
            continue;
        }

        // §11.8: "Do not label a source unavailable because one user's
        // credentials expired." The same reasoning applies harder here, and this
        // is where it has to live: a host that cannot get past a source's front
        // door fails every import it attempts, so without this the very first
        // attempt would rewrite a perfectly healthy source to `unavailable` — on
        // a permanent, non-actionable basis, and about a site that is serving
        // every other reader fine. `verification_status` is checked before any
        // evidence is counted, not after, so the skip cannot be reached by any
        // combination of import outcomes.
        if source.verification_status.as_deref() == Some(BLOCKED_HERE) {
            skipped.push(SkippedSource {
                key: source.key.clone(),
                reason: source
                    .verification_note
                    .clone()
                    .unwrap_or_else(|| "unreachable from the build host".to_owned()),
            });
            continue;
        }

        let (completed, failed) = finished_import_counts(db, &source.key, &since).await?;
        if completed == 0 && failed == 0 {
            // Nothing finished: no evidence, so no change. `last_checked_at` is
            // deliberately not touched either — it records when the source was
            // actually read, and this sweep read nothing.
            continue;
        }
        let current = if failed == 0 {
            "healthy".to_owned()
        } else if completed == 0 && failed >= FAILURES_TO_UNAVAILABLE {
            "unavailable".to_owned()
        } else {
            "degraded".to_owned()
        };
        if current != source.health {
            set_source_health(db, &source.key, &current).await?;
        }
        changes.push(SourceHealthChange {
            key: source.key,
            previous: source.health,
            current,
            completed,
            failed,
        });
    }

    Ok((changes, skipped))
}

/// Recompute every source's health from the outcome of its recent imports
/// (spec §11.8).
///
/// # The rules, and why each is what it is
///
/// * **`paused` is never derived and never overwritten.** A pause is an
///   operator's decision, and a sweep that could clear one would be a sweep
///   that silently un-pauses a source somebody deliberately switched off.
/// * **A source with no finished imports in the window keeps what it had.**
///   Silence is not evidence, and `unknown` is the honest state for a source
///   nobody has tried.
/// * **One success with no failures is `healthy`.** One failure alongside
///   successes is `degraded`: the source works and sometimes does not, which is
///   exactly what a reader should be told.
/// * **Three failures with no success is `unavailable`**, and an import into an
///   unavailable source is refused before it is queued rather than queued and
///   failed.
/// * **A cancelled import counts as neither.** The reader changed their mind,
///   which says nothing about the source.
pub async fn recompute_source_health(
    db: &Database,
    window_days: i64,
) -> Result<Vec<SourceHealthChange>> {
    // Delegates rather than repeating the loop. The M53-04 `blocked-here` skip
    // lives in `recompute_source_health_with_skips`, and a second copy of the
    // body would be a second place to forget it — which is exactly how a
    // `blocked-here` source's health gets rewritten to `unavailable` by someone
    // tidying this function up.
    let (changes, _skipped) = recompute_source_health_with_skips(db, window_days).await?;
    Ok(changes)
}

/// The RFC 3339 instant `window_days` ago, in the same shape the columns hold.
fn window_start(window_days: i64) -> String {
    use time::{Duration, OffsetDateTime};
    let now = OffsetDateTime::now_utc();
    let then = now - Duration::days(window_days);
    then.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| now_rfc3339())
}

/// How many of a source's imports finished, by outcome, since an instant.
///
/// `cancelled` is excluded rather than counted as a failure: a reader who
/// stopped an import has said something about the import, not about the source.
async fn finished_import_counts(
    db: &Database,
    source_key: &str,
    since: &str,
) -> Result<(i64, i64)> {
    let sql = db.sql(
        "SELECT
             SUM(CASE WHEN state = 'completed' THEN 1 ELSE 0 END) AS completed,
             SUM(CASE WHEN state = 'failed'    THEN 1 ELSE 0 END) AS failed
         FROM import_jobs
         WHERE source_key = ? AND updated_at >= ?
           AND state IN ('completed', 'failed')",
        "SELECT
             COALESCE(SUM(CASE WHEN state = 'completed' THEN 1 ELSE 0 END), 0)::BIGINT AS completed,
             COALESCE(SUM(CASE WHEN state = 'failed'    THEN 1 ELSE 0 END), 0)::BIGINT AS failed
         FROM import_jobs
         WHERE source_key = $1 AND updated_at >= $2
           AND state IN ('completed', 'failed')",
    );

    #[derive(FromRow)]
    struct Counts {
        completed: Option<i64>,
        failed: Option<i64>,
    }

    // A parameter is bound once per dialect, so the binds are written per arm
    // rather than through `run!`.
    let row: Counts = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(source_key)
                .bind(since)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(source_key)
                .bind(since)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok((row.completed.unwrap_or(0), row.failed.unwrap_or(0)))
}

/// One source by key, or `None` when this instance has never heard of it.
///
/// A key that is not a row is not an error: the catalogue is written by the
/// build, and a source added by a newer build of the software has no row until
/// the instance has synced its catalogue once.
pub async fn find_source(db: &Database, key: &str) -> Result<Option<Source>> {
    let sqlite_sql = format!("SELECT {SOURCE_COLUMNS} FROM sources WHERE key = ?");
    let postgres_sql = format!("SELECT {SOURCE_COLUMNS_PG} FROM sources WHERE key = ?");
    let sql = db.sql(&sqlite_sql, &postgres_sql);
    fetch_optional_source(db, &sql, key).await
}

/// The `Option`-returning half of [`find_source`], split out so the two driver
/// arms are written once.
async fn fetch_optional_source(db: &Database, sql: &str, key: &str) -> Result<Option<Source>> {
    let row: Option<SourceRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(sql)
                .bind(key)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(sql)
                .bind(key)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    row.map(decode_source).transpose()
}

// ---------------------------------------------------------------------------
// Library items
// ---------------------------------------------------------------------------

/// The fields an import writes about a work, before it knows anything else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryItemInput {
    /// Title as the source shows it.
    pub title: String,
    /// Author as the source shows it.
    pub author_text: String,
    /// The author's profile URL.
    pub author_url: Option<String>,
    /// Summary, as plain text.
    pub summary: String,
    /// The source's language tag.
    pub language: Option<String>,
    /// The source's word count.
    pub word_count: Option<i64>,
    /// `ongoing`, `complete`, `hiatus`, `cancelled` or `unknown`.
    pub status: String,
    /// The canonical URL at the source.
    pub source_url: String,
    /// When the source says the work last changed, in RFC 3339.
    pub source_updated_at: Option<String>,
    /// Which import produced this and from where.
    pub provenance_json: String,
}

/// Create or update the reader's copy of a work from a source.
///
/// The unique key is `(account_id, source_key, source_work_key)`, so importing
/// the same URL twice updates one row rather than creating two. This is the
/// mechanism behind spec §11's "repeat imports avoid accidental duplicates":
/// the database refuses the duplicate rather than a check that a concurrent
/// request could race.
///
/// Returns the item as it now stands.
pub async fn upsert_library_item(
    db: &Database,
    account_id: &str,
    source_key: &str,
    source_work_key: &str,
    input: &LibraryItemInput,
) -> Result<LibraryItem> {
    let now = now_rfc3339();
    let sql = db.sql(
        "INSERT INTO library_items (id, account_id, work_id, source_key, source_work_key,
                                    title, author_text, author_url, summary, language, word_count,
                                    status, source_url, source_updated_at, last_synced_at,
                                    provenance_json, created_at, updated_at, version)
         VALUES (?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)
         ON CONFLICT (account_id, source_key, source_work_key) DO UPDATE SET
             title = excluded.title,
             author_text = excluded.author_text,
             author_url = excluded.author_url,
             summary = excluded.summary,
             language = excluded.language,
             word_count = excluded.word_count,
             status = excluded.status,
             source_url = excluded.source_url,
             source_updated_at = excluded.source_updated_at,
             last_synced_at = excluded.last_synced_at,
             provenance_json = excluded.provenance_json,
             updated_at = excluded.updated_at,
             version = library_items.version + 1",
        "INSERT INTO library_items (id, account_id, work_id, source_key, source_work_key,
                                    title, author_text, author_url, summary, language, word_count,
                                    status, source_url, source_updated_at, last_synced_at,
                                    provenance_json, created_at, updated_at, version)
         VALUES (?::uuid, ?::uuid, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)
         ON CONFLICT (account_id, source_key, source_work_key) DO UPDATE SET
             title = excluded.title,
             author_text = excluded.author_text,
             author_url = excluded.author_url,
             summary = excluded.summary,
             language = excluded.language,
             word_count = excluded.word_count,
             status = excluded.status,
             source_url = excluded.source_url,
             source_updated_at = excluded.source_updated_at,
             last_synced_at = excluded.last_synced_at,
             provenance_json = excluded.provenance_json,
             updated_at = excluded.updated_at,
             version = library_items.version + 1",
    );
    let id = lorehaven_domain::LibraryItemId::new().to_string();
    let input = input.clone();
    let now_for_bind = now.clone();
    run!(db, &sql, |query| {
        query
            .bind(&id)
            .bind(account_id)
            .bind(source_key)
            .bind(source_work_key)
            .bind(&input.title)
            .bind(&input.author_text)
            .bind(input.author_url.clone())
            .bind(&input.summary)
            .bind(input.language.clone())
            .bind(input.word_count)
            .bind(&input.status)
            .bind(&input.source_url)
            .bind(input.source_updated_at.clone())
            .bind(&now_for_bind)
            .bind(&input.provenance_json)
            .bind(&now_for_bind)
            .bind(&now_for_bind)
    })
    .await
    .with_context(|| format!("recording the library item for {source_key}/{source_work_key}"))?;

    find_library_item(db, account_id, source_key, source_work_key)
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "the library item for {source_key}/{source_work_key} vanished after upsert"
            )
        })
}

/// Look up one reader's copy of a work.
pub async fn find_library_item(
    db: &Database,
    account_id: &str,
    source_key: &str,
    source_work_key: &str,
) -> Result<Option<LibraryItem>> {
    let sql = sql_owned(
        db,
        format!(
            "SELECT {LIBRARY_COLUMNS}, {LIBRARY_CHAPTER_COUNT} FROM library_items
             WHERE account_id = ? AND source_key = ? AND source_work_key = ?"
        ),
        format!(
            "SELECT {LIBRARY_COLUMNS_PG}, {LIBRARY_CHAPTER_COUNT} FROM library_items
             WHERE account_id::text = ? AND source_key = ? AND source_work_key = ?"
        ),
    );
    fetch_optional_library_item(db, &sql, account_id, source_key, source_work_key).await
}

/// One reader's copy, by identifier. Scoped to the account: a library item is
/// private, and a lookup that did not scope by account would be an enumeration.
pub async fn get_library_item(
    db: &Database,
    item_id: &str,
    account_id: &str,
) -> Result<Option<LibraryItem>> {
    let sql = sql_owned(
        db,
        format!(
            "SELECT {LIBRARY_COLUMNS}, {LIBRARY_CHAPTER_COUNT} FROM library_items \
             WHERE id = ? AND account_id = ?"
        ),
        format!(
            "SELECT {LIBRARY_COLUMNS_PG}, {LIBRARY_CHAPTER_COUNT} FROM library_items \
             WHERE id::text = ? AND account_id::text = ?"
        ),
    );
    let row: Option<LibraryItemRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(item_id)
                .bind(account_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(item_id)
                .bind(account_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    row.map(decode_library_item).transpose()
}

/// A page of one reader's library, newest first.
///
/// Keyset pagination on `(updated_at DESC, id DESC)` rather than `OFFSET`: an
/// import that lands while a reader is paging must not shift the page under
/// them, and the pair is unique so no row can be skipped or repeated.
pub async fn list_library_items(
    db: &Database,
    account_id: &str,
    limit: i64,
    after: Option<(&str, &str)>,
) -> Result<Vec<LibraryItem>> {
    let limit = limit.clamp(1, 200);
    let sql = sql_owned(
        db,
        format!(
            "SELECT {LIBRARY_COLUMNS}, {LIBRARY_CHAPTER_COUNT} FROM library_items
             WHERE account_id = ?
               AND (? IS NULL OR (updated_at, id) < (?, ?))
             ORDER BY updated_at DESC, id DESC
             LIMIT ?"
        ),
        format!(
            "SELECT {LIBRARY_COLUMNS_PG}, {LIBRARY_CHAPTER_COUNT} FROM library_items
             WHERE account_id::text = ?
               AND (?::text IS NULL OR (updated_at, id::text) < (?::text, ?::text))
             ORDER BY updated_at DESC, id DESC
             LIMIT ?"
        ),
    );
    let (after_updated, after_id) = match after {
        Some((updated, id)) => (Some(updated.to_owned()), Some(id.to_owned())),
        None => (None, None),
    };
    let after_updated_bind = after_updated.clone();
    let after_id_bind = after_id.clone();
    let rows: Vec<LibraryItemRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .bind(after_updated)
                .bind(after_id)
                .bind(after_id_bind)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .bind(after_updated_bind)
                .bind(after_id_bind.clone())
                .bind(after_id_bind)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    rows.into_iter().map(decode_library_item).collect()
}

/// Mark a library item as just checked, without changing anything else.
pub async fn touch_library_item_synced(db: &Database, item_id: &str) -> Result<()> {
    let now = now_rfc3339();
    let sql = db.sql(
        "UPDATE library_items SET last_synced_at = ?, updated_at = ? WHERE id = ?",
        "UPDATE library_items SET last_synced_at = ?, updated_at = ? WHERE id::text = ?",
    );
    run!(db, &sql, |query| {
        query.bind(&now).bind(&now).bind(item_id)
    })
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Import jobs
// ---------------------------------------------------------------------------

/// Record an import, bound to the queue row that carries it.
///
/// The id is the caller's to choose so that the queue row can name the import in
/// its payload before the import exists. The other way round leaves a claimable
/// job that points at nothing, which the worker would fail for a reason that is
/// nobody's fault.
#[allow(clippy::too_many_arguments)]
pub async fn create_import_job(
    db: &Database,
    id: &str,
    queue_job_id: &str,
    account_id: &str,
    pseud_id: &str,
    source_key: &str,
    source_url: &str,
    destination_type: &str,
    dry_run: bool,
) -> Result<ImportJob> {
    let now = now_rfc3339();
    let sql = db.sql(
        "INSERT INTO import_jobs (id, job_id, account_id, pseud_id, source_key, source_url,
                                  destination_type, destination_id, dry_run, state, created_at,
                                  updated_at, version)
         VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?, 'queued', ?, ?, 1)",
        "INSERT INTO import_jobs (id, job_id, account_id, pseud_id, source_key, source_url,
                                  destination_type, destination_id, dry_run, state, created_at,
                                  updated_at, version)
         VALUES (?::uuid, ?::uuid, ?::uuid, ?::uuid, ?, ?, ?, NULL, ?, 'queued', ?, ?, 1)",
    );
    run!(db, &sql, |query| {
        query
            .bind(id)
            .bind(queue_job_id)
            .bind(account_id)
            .bind(pseud_id)
            .bind(source_key)
            .bind(source_url)
            .bind(destination_type)
            .bind(i64::from(dry_run))
            .bind(&now)
            .bind(&now)
    })
    .await
    .with_context(|| format!("recording an import of {source_key}"))?;

    get_import_job(db, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the import job vanished after insert"))
}

/// One import, by identifier. Not account-scoped: the caller is the worker, and
/// the routes scope their own reads.
pub async fn get_import_job(db: &Database, id: &str) -> Result<Option<ImportJob>> {
    let sql = sql_owned(
        db,
        format!("SELECT {IMPORT_JOB_COLUMNS} FROM import_jobs WHERE id = ?"),
        format!("SELECT {IMPORT_JOB_COLUMNS_PG} FROM import_jobs WHERE id::text = ?"),
    );
    let row: Option<ImportJobRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    row.map(decode_import_job).transpose()
}

/// One import, scoped to the reader who asked for it.
pub async fn get_import_job_for(
    db: &Database,
    id: &str,
    account_id: &str,
) -> Result<Option<ImportJob>> {
    let sql = sql_owned(
        db,

        format!(
            "SELECT {IMPORT_JOB_COLUMNS} FROM import_jobs WHERE id = ? AND account_id = ?"
        ),
        format!(
            "SELECT {IMPORT_JOB_COLUMNS_PG} FROM import_jobs WHERE id::text = ? AND account_id::text = ?"
        ),
    );
    let row: Option<ImportJobRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(id)
                .bind(account_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(id)
                .bind(account_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    row.map(decode_import_job).transpose()
}

/// A page of one reader's imports, newest first.
///
/// The state filter and the cursor are both applied in SQL rather than in Rust,
/// so a page is still one page: filtering after the limit would return fewer
/// rows than asked for and make a client believe it had reached the end.
pub async fn list_import_jobs(
    db: &Database,
    account_id: &str,
    state: Option<&str>,
    limit: i64,
    after: Option<(&str, &str)>,
) -> Result<Vec<ImportJob>> {
    let limit = limit.clamp(1, 200);
    let (after_at, after_id) = after.map_or((None::<&str>, None::<&str>), |(at, id)| {
        (Some(at), Some(id))
    });
    let sql = sql_owned(
        db,
        format!(
            "SELECT {IMPORT_JOB_COLUMNS} FROM import_jobs
             WHERE account_id = ?
               AND (? IS NULL OR state = ?)
               AND (? IS NULL OR (created_at, id) < (?, ?))
             ORDER BY created_at DESC, id DESC LIMIT ?"
        ),
        format!(
            "SELECT {IMPORT_JOB_COLUMNS_PG} FROM import_jobs
             WHERE account_id::text = ?
               AND (?::text IS NULL OR state = ?::text)
               AND (?::text IS NULL OR (created_at, id) < (?::text, ?::uuid))
             ORDER BY created_at DESC, id DESC LIMIT ?"
        ),
    );
    let rows: Vec<ImportJobRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .bind(state)
                .bind(state)
                .bind(after_at)
                .bind(after_at)
                .bind(after_id)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account_id)
                .bind(state)
                .bind(state)
                .bind(after_at)
                .bind(after_at)
                .bind(after_id)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    rows.into_iter().map(decode_import_job).collect()
}

/// The queue row that runs this import, so cancelling one cancels both.
pub async fn job_for_import(db: &Database, import_id: &str) -> Result<Option<String>> {
    let sql = db.sql(
        "SELECT job_id FROM import_jobs WHERE id = ?",
        "SELECT job_id::text FROM import_jobs WHERE id::text = ?",
    );
    let row: Option<(Option<String>,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(import_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(import_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.and_then(|(id,)| id))
}

/// One credential, by id. Metadata only; the value lives in `secrets`.
pub async fn get_source_credential(db: &Database, id: &str) -> Result<Option<SourceCredential>> {
    let sql = sql_owned(
        db,
        format!("SELECT {CREDENTIAL_COLUMNS} FROM source_credentials WHERE id = ?"),
        format!("SELECT {CREDENTIAL_COLUMNS_PG} FROM source_credentials WHERE id::text = ?"),
    );
    let row: Option<SourceCredentialRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    row.map(decode_credential).transpose()
}

/// Move an import to a new state, optionally recording its report and item.
pub async fn set_import_state(
    db: &Database,
    id: &str,
    state: &str,
    library_item_id: Option<&str>,
    report_json: Option<&str>,
) -> Result<()> {
    let now = now_rfc3339();
    // COALESCE per field: a caller that only wants to change the state must not
    // have to re-send the report it is not touching.
    let sql = db.sql(
        "UPDATE import_jobs SET state = ?,
                                library_item_id = COALESCE(?, library_item_id),
                                report_json = COALESCE(?, report_json),
                                updated_at = ?, version = version + 1
         WHERE id = ?",
        "UPDATE import_jobs SET state = ?,
                                library_item_id = COALESCE(?::uuid, library_item_id),
                                report_json = COALESCE(?, report_json),
                                updated_at = ?, version = version + 1
         WHERE id::text = ?",
    );
    run!(db, &sql, |query| {
        query
            .bind(state)
            .bind(library_item_id.map(str::to_owned))
            .bind(report_json.map(str::to_owned))
            .bind(&now)
            .bind(id)
    })
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Imported chapters
// ---------------------------------------------------------------------------

/// What is known about a chapter at the moment its row is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportChapterInput {
    /// The source's own identifier.
    pub source_chapter_key: String,
    /// 1-based position.
    pub ordinal: i64,
    /// Title as the source shows it.
    pub title: String,
    /// `pending`, `stored`, `skipped` or `failed`.
    pub state: String,
    /// The body's content checksum, when it has been stored.
    pub content_blob_checksum: Option<String>,
    /// Why a chapter failed, or what a skip meant.
    pub note: Option<String>,
}

/// Record a chapter's outcome, replacing any earlier row for the same chapter.
///
/// `(import_job_id, source_chapter_key)` is unique, so this is the write that
/// makes a retry idempotent: running the same import twice cannot store a
/// chapter twice, and a chapter already `stored` can be skipped rather than
/// re-read.
pub async fn upsert_import_chapter(
    db: &Database,
    import_job_id: &str,
    library_item_id: Option<&str>,
    input: &ImportChapterInput,
) -> Result<()> {
    let id = lorehaven_domain::ImportChapterId::new().to_string();
    let now = now_rfc3339();
    let input = input.clone();
    let item = library_item_id.map(str::to_owned);
    let sql = db.sql(
        "INSERT INTO import_chapters (id, import_job_id, library_item_id, source_chapter_key,
                                      ordinal, title, state, content_blob_checksum, note,
                                      created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (import_job_id, source_chapter_key) DO UPDATE SET
             library_item_id = COALESCE(excluded.library_item_id, import_chapters.library_item_id),
             ordinal = excluded.ordinal,
             title = excluded.title,
             state = excluded.state,
             content_blob_checksum = COALESCE(excluded.content_blob_checksum,
                                              import_chapters.content_blob_checksum),
             note = excluded.note,
             updated_at = excluded.updated_at",
        "INSERT INTO import_chapters (id, import_job_id, library_item_id, source_chapter_key,
                                      ordinal, title, state, content_blob_checksum, note,
                                      created_at, updated_at)
         VALUES (?::uuid, ?::uuid, ?::uuid, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (import_job_id, source_chapter_key) DO UPDATE SET
             library_item_id = COALESCE(excluded.library_item_id, import_chapters.library_item_id),
             ordinal = excluded.ordinal,
             title = excluded.title,
             state = excluded.state,
             content_blob_checksum = COALESCE(excluded.content_blob_checksum,
                                              import_chapters.content_blob_checksum),
             note = excluded.note,
             updated_at = excluded.updated_at",
    );
    let now_for_bind = now.clone();
    run!(db, &sql, |query| {
        query
            .bind(&id)
            .bind(import_job_id)
            .bind(item.clone())
            .bind(&input.source_chapter_key)
            .bind(input.ordinal)
            .bind(&input.title)
            .bind(&input.state)
            .bind(input.content_blob_checksum.clone())
            .bind(input.note.clone())
            .bind(&now_for_bind)
            .bind(&now_for_bind)
    })
    .await
    .with_context(|| {
        format!(
            "recording chapter {} of import {import_job_id}",
            input.source_chapter_key
        )
    })?;
    Ok(())
}

/// The chapters of one import, in order.
pub async fn list_import_chapters(
    db: &Database,
    import_job_id: &str,
) -> Result<Vec<ImportChapter>> {
    let sql = sql_owned(
        db,
        format!(
            "SELECT {IMPORT_CHAPTER_COLUMNS} FROM import_chapters
             WHERE import_job_id = ? ORDER BY ordinal"
        ),
        format!(
            "SELECT {IMPORT_CHAPTER_COLUMNS_PG} FROM import_chapters
             WHERE import_job_id::text = ? ORDER BY ordinal"
        ),
    );
    let rows: Vec<ImportChapterRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(import_job_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(import_job_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    rows.into_iter().map(decode_import_chapter).collect()
}

/// The most recent chapters recorded for a library item.
///
/// This is what a re-import compares against: the checksum each chapter had the
/// last time it was read is the only way to answer "did the text actually
/// change", a question a preview cannot answer.
/// What the previous import of this work stored, or `None` if there is no
/// previous import.
///
/// This is the input the planner needs: a re-import is compared against what is
/// already held, and "nothing held yet" is a different answer from "held, with
/// these chapters". Returning an empty vector for both would make a first
/// import look like an update that removed everything.
pub async fn previous_chapters_for(
    db: &Database,
    account_id: &str,
    source_key: &str,
    source_work_key: &str,
) -> Result<Option<Vec<ImportChapter>>> {
    let item = find_library_item(db, account_id, source_key, source_work_key).await?;
    let Some(item) = item else {
        return Ok(None);
    };
    let sql = sql_owned(
        db,
        format!(
            "SELECT {IMPORT_CHAPTER_COLUMNS} FROM import_chapters
             WHERE import_job_id = (
                 SELECT id FROM import_jobs
                 WHERE library_item_id = ? AND account_id = ?
                 ORDER BY created_at DESC, id DESC LIMIT 1
             )
             ORDER BY ordinal"
        ),
        format!(
            "SELECT {IMPORT_CHAPTER_COLUMNS} FROM import_chapters
             WHERE import_job_id = (
                 SELECT id FROM import_jobs
                 WHERE library_item_id::text = ? AND account_id::text = ?
                 ORDER BY created_at DESC, id DESC LIMIT 1
             )
             ORDER BY ordinal"
        ),
    );
    let rows: Vec<ImportChapterRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(&item.id)
                .bind(account_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(&item.id)
                .bind(account_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    rows.into_iter()
        .map(decode_import_chapter)
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

pub async fn latest_chapters_for_item(
    db: &Database,
    library_item_id: &str,
) -> Result<Vec<ImportChapter>> {
    let sql = sql_owned(
        db,
        format!(
            "SELECT {IMPORT_CHAPTER_COLUMNS} FROM import_chapters
             WHERE library_item_id = ?
               AND import_job_id = (
                   SELECT import_job_id FROM import_chapters
                   WHERE library_item_id = ?
                   ORDER BY updated_at DESC, id DESC LIMIT 1
               )
             ORDER BY ordinal"
        ),
        format!(
            "SELECT {IMPORT_CHAPTER_COLUMNS_PG} FROM import_chapters
             WHERE library_item_id::text = ?
               AND import_job_id = (
                   SELECT import_job_id FROM import_chapters
                   WHERE library_item_id::text = ?
                   ORDER BY updated_at DESC, id DESC LIMIT 1
               )
             ORDER BY ordinal"
        ),
    );
    let rows: Vec<ImportChapterRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(library_item_id)
                .bind(library_item_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(library_item_id)
                .bind(library_item_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    rows.into_iter().map(decode_import_chapter).collect()
}

// ---------------------------------------------------------------------------
// Source credentials
// ---------------------------------------------------------------------------

/// Store a connection to a source, replacing one with the same label.
///
/// `secret_id` names the row in `secrets` that holds the ciphertext; this
/// function never sees the credential itself. A replacement revokes the previous
/// ciphertext by leaving it to cascade when its row is deleted by the caller —
/// which is why the caller must delete the old secret, and why this function
/// returns the *previous* secret id so that it can.
///
/// # Why the three M53-03 fields are arguments and not derived
///
/// `kind`, `origin_host` and `consent_at` are all things the *caller* knows and
/// this function cannot. Deriving them would be the mistake: an adapter's current
/// `AuthKind` is what the adapter says today, not what the reader was told when
/// they consented, and writing today's value into a consent record is inventing
/// one. The origin comes from the adapter's declared host list, and consent is
/// an act by a person, so all three are passed in or not recorded at all.
///
/// `renewed` is recorded by the audit layer rather than here, because this
/// function is the only place that knows a replacement happened — and §11.6's
/// "re-consent for renewal" only means something if the renewal is visible.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_source_credential(
    db: &Database,
    pseud_id: &str,
    source_key: &str,
    secret_id: &str,
    label: &str,
    expires_at: Option<&str>,
    kind: Option<&str>,
    origin_host: Option<&str>,
    consent_at: Option<&str>,
) -> Result<(SourceCredential, Option<String>)> {
    let previous = find_credential_by_label(db, pseud_id, source_key, label).await?;
    let id = match &previous {
        Some(existing) => existing.id.clone(),
        None => lorehaven_domain::SourceCredentialId::new().to_string(),
    };
    let previous_secret = previous.as_ref().map(|row| row.secret_id.clone());
    let now = now_rfc3339();
    // Every M53-03 column is overwritten on conflict rather than carried
    // forward. For `consent_at` that is the point: §11.6 requires re-consent for
    // renewal, so a rotation that kept the old timestamp would satisfy the word
    // "consent" and none of its intent.
    let sql = db.sql(
        "INSERT INTO source_credentials (id, pseud_id, source_key, secret_id, label, status,
                                         expires_at, created_at, updated_at, version,
                                         kind, origin_host, consent_at)
         VALUES (?, ?, ?, ?, ?, 'active', ?, ?, ?, 1, ?, ?, ?)
         ON CONFLICT (pseud_id, source_key, label) DO UPDATE SET
             secret_id = excluded.secret_id,
             status = 'active',
             expires_at = excluded.expires_at,
             last_checked_at = NULL,
             updated_at = excluded.updated_at,
             kind = excluded.kind,
             origin_host = excluded.origin_host,
             consent_at = excluded.consent_at,
             version = source_credentials.version + 1",
        "INSERT INTO source_credentials (id, pseud_id, source_key, secret_id, label, status,
                                         expires_at, created_at, updated_at, version,
                                         kind, origin_host, consent_at)
         VALUES (?::uuid, ?::uuid, ?, ?::uuid, ?, 'active', ?, ?, ?, 1, ?, ?, ?)
         ON CONFLICT (pseud_id, source_key, label) DO UPDATE SET
             secret_id = excluded.secret_id,
             status = 'active',
             expires_at = excluded.expires_at,
             last_checked_at = NULL,
             updated_at = excluded.updated_at,
             kind = excluded.kind,
             origin_host = excluded.origin_host,
             consent_at = excluded.consent_at,
             version = source_credentials.version + 1",
    );
    let expires = expires_at.map(str::to_owned);
    let kind = kind.map(str::to_owned);
    let origin_host = origin_host.map(str::to_owned);
    let consent_at = consent_at.map(str::to_owned);
    let now_for_bind = now.clone();
    run!(db, &sql, |query| {
        query
            .bind(&id)
            .bind(pseud_id)
            .bind(source_key)
            .bind(secret_id)
            .bind(label)
            .bind(expires.clone())
            .bind(&now_for_bind)
            .bind(&now_for_bind)
            .bind(kind.clone())
            .bind(origin_host.clone())
            .bind(consent_at.clone())
    })
    .await
    .with_context(|| format!("storing a {source_key} credential"))?;

    let stored = get_source_credential(db, &id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the credential vanished after upsert"))?;
    Ok((stored, previous_secret))
}

/// Find a credential by its label within a pseud's connections.
pub async fn find_credential_by_label(
    db: &Database,
    pseud_id: &str,
    source_key: &str,
    label: &str,
) -> Result<Option<SourceCredential>> {
    let sql = sql_owned(
        db,
        format!(
            "SELECT {CREDENTIAL_COLUMNS} FROM source_credentials
             WHERE pseud_id = ? AND source_key = ? AND label = ?"
        ),
        format!(
            "SELECT {CREDENTIAL_COLUMNS_PG} FROM source_credentials
             WHERE pseud_id::text = ? AND source_key = ? AND label = ?"
        ),
    );
    let row: Option<SourceCredentialRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(pseud_id)
                .bind(source_key)
                .bind(label)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(pseud_id)
                .bind(source_key)
                .bind(label)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    row.map(decode_credential).transpose()
}

/// A pseud's connections, optionally for one source.
///
/// Scoped to the pseud, never the account: spec §11.6 requires that one pseud's
/// credential is not usable as another's, and the scoping has to be in the query
/// rather than in a check the caller might skip.
pub async fn list_source_credentials(
    db: &Database,
    pseud_id: &str,
    source_key: Option<&str>,
) -> Result<Vec<SourceCredential>> {
    let sql = sql_owned(
        db,
        format!(
            "SELECT {CREDENTIAL_COLUMNS} FROM source_credentials
             WHERE pseud_id = ? AND (? IS NULL OR source_key = ?)
             ORDER BY source_key, label"
        ),
        format!(
            "SELECT {CREDENTIAL_COLUMNS_PG} FROM source_credentials
             WHERE pseud_id::text = ? AND (?::text IS NULL OR source_key = ?)
             ORDER BY source_key, label"
        ),
    );
    let key = source_key.map(str::to_owned);
    let key_bind = key.clone();
    let rows: Vec<SourceCredentialRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(pseud_id)
                .bind(key)
                .bind(key_bind)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(pseud_id)
                .bind(key_bind.clone())
                .bind(key_bind)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    rows.into_iter().map(decode_credential).collect()
}

/// Change a credential's operational status.
pub async fn set_credential_status(
    db: &Database,
    id: &str,
    status: &str,
    mark_checked: bool,
) -> Result<()> {
    let now = now_rfc3339();
    let sql = db.sql(
        "UPDATE source_credentials
         SET status = ?, last_checked_at = CASE WHEN ? = 1 THEN ? ELSE last_checked_at END,
             updated_at = ?, version = version + 1
         WHERE id = ?",
        "UPDATE source_credentials
         SET status = ?, last_checked_at = CASE WHEN ? = 1 THEN ? ELSE last_checked_at END,
             updated_at = ?, version = version + 1
         WHERE id::text = ?",
    );
    run!(db, &sql, |query| {
        query
            .bind(status)
            .bind(i64::from(mark_checked))
            .bind(&now)
            .bind(&now)
            .bind(id)
    })
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Credential audit trail (M53-03, spec §11.6)
// ---------------------------------------------------------------------------

/// The events §11.6's "audit events without secret contents" admits.
///
/// A closed set rather than a free string, and the closure is the point: an audit
/// table with a free-text `event` column is an audit table that will eventually
/// carry whatever a caller put in it, including a snippet of a rejected password
/// pasted into an error message. The `CHECK` in migration 0096 enforces the same
/// set at the schema, and
/// `every_credential_event_the_store_accepts_is_accepted_by_the_schema` asserts
/// the two agree — a store that accepts an event the schema rejects is a reader
/// who gets an audit error on a legitimate credential action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialEvent {
    /// A credential was stored for the first time.
    Created,
    /// A credential was used for a fetch.
    Used,
    /// A credential was tested against its source.
    Tested,
    /// A use or test failed.
    Failed,
    /// A credential was revoked, and that stops it being used.
    Revoked,
    /// An existing credential was replaced, with fresh consent.
    Renewed,
    /// A credential passed its expiry.
    Expired,
}

impl CredentialEvent {
    /// The stored spelling, matching the `CHECK` constraint in migration 0096.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Used => "used",
            Self::Tested => "tested",
            Self::Failed => "failed",
            Self::Revoked => "revoked",
            Self::Renewed => "renewed",
            Self::Expired => "expired",
        }
    }

    /// Every event, for tests that assert the store and the schema agree.
    pub const ALL: [Self; 7] = [
        Self::Created,
        Self::Used,
        Self::Tested,
        Self::Failed,
        Self::Revoked,
        Self::Renewed,
        Self::Expired,
    ];
}

/// One audit row: which credential did what, when, and under which pseud.
///
/// There is deliberately no free-text field and no column for anything derived
/// from the secret. §11.6 says "audit events without secret contents", and the
/// only reliable way to keep a secret out of an audit trail is for the audit
/// trail to have nowhere to put one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialAuditEvent {
    /// Identifier.
    pub id: String,
    /// The credential this happened to.
    pub credential_id: String,
    /// The pseud that owns the credential.
    pub pseud_id: String,
    /// What happened.
    pub event: String,
    /// When.
    pub occurred_at: String,
}

#[derive(FromRow)]
struct CredentialAuditRow {
    id: String,
    credential_id: String,
    pseud_id: String,
    event: String,
    occurred_at: String,
}

fn decode_audit_event(row: CredentialAuditRow) -> Result<CredentialAuditEvent> {
    Ok(CredentialAuditEvent {
        id: row.id,
        credential_id: row.credential_id,
        pseud_id: row.pseud_id,
        event: row.event,
        occurred_at: row.occurred_at,
    })
}

const AUDIT_COLUMNS: &str = "id, credential_id, pseud_id, event, occurred_at";

const AUDIT_COLUMNS_PG: &str = "id::text AS id, credential_id::text AS credential_id, \
    pseud_id::text AS pseud_id, event, occurred_at";

/// Record that something happened to a credential.
///
/// Pseud-scoped on write, deliberately. The caller passes both the credential id
/// and the pseud, and the insert refuses if they disagree — because an audit row
/// that names a credential belonging to a *different* pseud is exactly the leak
/// §11.6's audit clause exists to prevent: it would let one reader see that
/// another reader has a connection to a login-gated source, and when they last
/// used it.
pub async fn record_credential_event(
    db: &Database,
    credential_id: &str,
    pseud_id: &str,
    event: CredentialEvent,
) -> Result<()> {
    let owner_sql = db.sql(
        "SELECT pseud_id FROM source_credentials WHERE id = ?",
        "SELECT pseud_id::text FROM source_credentials WHERE id::text = ?",
    );
    let owner: Option<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&owner_sql)
                .bind(credential_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&owner_sql)
                .bind(credential_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    // A missing credential is refused rather than recorded. `credential_id` is not
    // a foreign key (migration 0096), so an insert naming a credential that never
    // existed would succeed and leave a permanent audit row for something that
    // never happened — and the one caller that can produce that is a request for
    // a credential id the reader guessed.
    let Some((owner_pseud,)) = owner else {
        anyhow::bail!("no such credential");
    };
    if !same_host(&owner_pseud, pseud_id) {
        // The mismatch means the caller passed a credential id that is not theirs.
        // The message names no id and no secret, so it is safe to surface; the fix
        // is the same as for any other cross-pseud access: do not write.
        anyhow::bail!("the credential does not belong to this pseud");
    }

    let sql = db.sql(
        "INSERT INTO credential_audit_events (id, credential_id, pseud_id, event, occurred_at)
         VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO credential_audit_events (id, credential_id, pseud_id, event, occurred_at)
         VALUES (?::uuid, ?::uuid, ?::uuid, ?, ?)",
    );
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_rfc3339();
    run!(db, &sql, |query| {
        query
            .bind(&id)
            .bind(credential_id)
            .bind(pseud_id)
            .bind(event.as_str())
            .bind(&now)
    })
    .await
    .with_context(|| format!("recording a credential {} event", event.as_str()))?;
    Ok(())
}

/// Every audit row for one credential, newest first.
pub async fn list_credential_events(
    db: &Database,
    credential_id: &str,
) -> Result<Vec<CredentialAuditEvent>> {
    let sql = sql_owned(
        db,
        format!(
            "SELECT {AUDIT_COLUMNS} FROM credential_audit_events
             WHERE credential_id = ? ORDER BY occurred_at DESC, id DESC"
        ),
        format!(
            "SELECT {AUDIT_COLUMNS_PG} FROM credential_audit_events
             WHERE credential_id::text = ? ORDER BY occurred_at DESC, id DESC"
        ),
    );
    let rows: Vec<CredentialAuditRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(credential_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(credential_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    rows.into_iter().map(decode_audit_event).collect()
}

/// Pseudonyms holding at least one credential with no consent record.
///
/// §11.6's "re-consent for renewal" is only enforceable if something asks the
/// question, and this is the sweep that does. Partial-index-backed: it reads only
/// the rows where `consent_at IS NULL`, so a healthy instance pays nothing.
pub async fn pseuds_with_unconsented_credentials(db: &Database) -> Result<Vec<String>> {
    let sql = db.sql(
        "SELECT DISTINCT pseud_id FROM source_credentials WHERE consent_at IS NULL",
        "SELECT DISTINCT pseud_id::text FROM source_credentials WHERE consent_at IS NULL",
    );
    let rows: Vec<(String,)> = match db.backend() {
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
    Ok(rows.into_iter().map(|(pseud,)| pseud).collect())
}

/// Remove a connection, returning the secret it pointed at.
///
/// The caller deletes that secret; the ciphertext is not this module's to hold.
/// Deleting a credential never deletes an already imported copy (spec §11.6).
pub async fn delete_source_credential(
    db: &Database,
    id: &str,
    pseud_id: &str,
) -> Result<Option<String>> {
    let existing = get_source_credential(db, id).await?;
    let Some(existing) = existing else {
        return Ok(None);
    };
    if existing.pseud_id != pseud_id {
        // Not this pseud's to delete. Reported as absent rather than refused, so
        // the route can answer 404 without disclosing that the row exists.
        return Ok(None);
    }
    // The secret is what is deleted, and the credential goes with it: the schema
    // declares `source_credentials.secret_id` as `REFERENCES secrets(id) ON
    // DELETE CASCADE`, so removing the ciphertext removes the row that names it.
    //
    // That direction is deliberate. A revocation that left the ciphertext behind
    // would leave a reader's source password sitting in the database after they
    // asked for it to be gone, and "the caller also has to remember to delete
    // the secret" is exactly the kind of instruction that gets forgotten by the
    // second caller. The foreign key is the guarantee; this function only has to
    // not fight it.
    let sql = db.sql(
        "DELETE FROM secrets WHERE id = ?",
        "DELETE FROM secrets WHERE id::text = ?",
    );
    run!(db, &sql, |query| query.bind(&existing.secret_id)).await?;
    Ok(Some(existing.secret_id))
}

// ---------------------------------------------------------------------------
// Row shapes
// ---------------------------------------------------------------------------

/// How many chapters of this copy are actually stored.
///
/// Counted from `import_chapters` rather than read off the source's own count,
/// because the two answer different questions and the difference is the point:
/// "the source lists 109 chapters" and "we hold 109 chapters" diverge the moment
/// an import is partial, and a library that showed the source's number would
/// report a complete copy of a work it holds a third of. `state = 'stored'` is
/// the same rule the planner uses, so a chapter that is known and not yet
/// fetched does not count as one the reader can read.
pub(crate) const LIBRARY_CHAPTER_COUNT: &str = "(SELECT COUNT(*) FROM import_chapters \
    WHERE import_chapters.library_item_id = library_items.id AND state = 'stored') \
    AS chapter_count";

pub(crate) const LIBRARY_COLUMNS: &str =
    "id, account_id, work_id, source_key, source_work_key, title, \
    author_text, author_url, summary, language, word_count, status, source_url, \
    source_updated_at, last_synced_at, provenance_json, created_at, updated_at, version";

pub(crate) const LIBRARY_COLUMNS_PG: &str = "id::text AS id, account_id::text AS account_id, \
    work_id::text AS work_id, source_key, source_work_key, title, author_text, author_url, \
    summary, language, word_count, status, source_url, source_updated_at, last_synced_at, \
    provenance_json, created_at, updated_at, version";

const IMPORT_JOB_COLUMNS: &str = "id, job_id, account_id, pseud_id, source_key, source_url, \
    destination_type, dry_run, state, library_item_id, report_json, created_at, updated_at";

const IMPORT_JOB_COLUMNS_PG: &str = "id::text AS id, job_id::text AS job_id, \
    account_id::text AS account_id, pseud_id::text AS pseud_id, source_key, source_url, \
    destination_type, dry_run, state, library_item_id::text AS library_item_id, report_json, \
    created_at, updated_at";

const IMPORT_CHAPTER_COLUMNS: &str = "id, import_job_id, library_item_id, source_chapter_key, \
    ordinal, title, state, content_blob_checksum, chapter_id, note";

const IMPORT_CHAPTER_COLUMNS_PG: &str = "id::text AS id, import_job_id::text AS import_job_id, \
    library_item_id::text AS library_item_id, source_chapter_key, ordinal, title, state, \
    content_blob_checksum, chapter_id::text AS chapter_id, note";

/// The columns every read of `sources` selects, in one place.
///
/// Two readers that drift apart is how a column ends up bound to the wrong
/// field, and `verification_status` is the one whose absence is silent: a
/// `blocked-here` source that a query does not select looks exactly like one
/// nobody has classified.
const SOURCE_COLUMNS: &str = "key, display_name, adapter_version, enabled, disabled_reason, \
    capability_json, health, last_checked_at, verification_status, verification_note";

/// The PostgreSQL twin of [`SOURCE_COLUMNS`]: `enabled` is a smallint there and
/// an integer here, so only that column differs in spelling.
const SOURCE_COLUMNS_PG: &str = "key, display_name, adapter_version, enabled, disabled_reason, \
    capability_json, health, last_checked_at, verification_status, verification_note";

const CREDENTIAL_COLUMNS: &str = "id, pseud_id, source_key, secret_id, label, status, expires_at, \
    last_checked_at, created_at, updated_at, version, kind, origin_host, consent_at";

const CREDENTIAL_COLUMNS_PG: &str = "id::text AS id, pseud_id::text AS pseud_id, source_key, \
    secret_id::text AS secret_id, label, status, expires_at, last_checked_at, created_at, \
    updated_at, version, kind, origin_host, consent_at";

#[derive(FromRow)]
struct SourceRow {
    key: String,
    display_name: String,
    adapter_version: String,
    enabled: i64,
    disabled_reason: Option<String>,
    capability_json: String,
    health: String,
    last_checked_at: Option<String>,
    verification_status: Option<String>,
    verification_note: Option<String>,
}

#[derive(FromRow)]
pub(crate) struct LibraryItemRow {
    pub(crate) id: String,
    pub(crate) account_id: String,
    pub(crate) work_id: Option<String>,
    pub(crate) chapter_count: i64,
    pub(crate) source_key: String,
    pub(crate) source_work_key: String,
    pub(crate) title: String,
    pub(crate) author_text: String,
    pub(crate) author_url: Option<String>,
    pub(crate) summary: String,
    pub(crate) language: Option<String>,
    pub(crate) word_count: Option<i64>,
    pub(crate) status: String,
    pub(crate) source_url: String,
    pub(crate) source_updated_at: Option<String>,
    pub(crate) last_synced_at: Option<String>,
    pub(crate) provenance_json: String,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) version: i64,
}

#[derive(FromRow)]
struct ImportJobRow {
    id: String,
    job_id: String,
    account_id: String,
    pseud_id: String,
    source_key: String,
    source_url: String,
    destination_type: String,
    dry_run: i64,
    state: String,
    library_item_id: Option<String>,
    report_json: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(FromRow)]
struct ImportChapterRow {
    id: String,
    import_job_id: String,
    library_item_id: Option<String>,
    source_chapter_key: String,
    ordinal: i64,
    title: String,
    state: String,
    content_blob_checksum: Option<String>,
    chapter_id: Option<String>,
    note: Option<String>,
}

#[derive(FromRow)]
struct SourceCredentialRow {
    id: String,
    pseud_id: String,
    source_key: String,
    secret_id: String,
    label: String,
    status: String,
    expires_at: Option<String>,
    last_checked_at: Option<String>,
    created_at: String,
    updated_at: String,
    version: i64,
    kind: Option<String>,
    origin_host: Option<String>,
    consent_at: Option<String>,
}

fn decode_source(row: SourceRow) -> Result<Source> {
    Ok(Source {
        key: row.key,
        display_name: row.display_name,
        adapter_version: row.adapter_version,
        enabled: row.enabled != 0,
        disabled_reason: row.disabled_reason,
        capability_json: row.capability_json,
        health: row.health,
        last_checked_at: row.last_checked_at,
        verification_status: row.verification_status,
        verification_note: row.verification_note,
    })
}

pub(crate) fn decode_library_item(row: LibraryItemRow) -> Result<LibraryItem> {
    Ok(LibraryItem {
        id: row.id,
        account_id: row.account_id,
        work_id: row.work_id,
        source_key: row.source_key,
        source_work_key: row.source_work_key,
        title: row.title,
        author_text: row.author_text,
        author_url: row.author_url,
        summary: row.summary,
        language: row.language,
        word_count: row.word_count,
        status: row.status,
        source_url: row.source_url,
        source_updated_at: row.source_updated_at,
        last_synced_at: row.last_synced_at,
        chapter_count: row.chapter_count,
        provenance_json: row.provenance_json,
        created_at: row.created_at,
        updated_at: row.updated_at,
        version: row.version,
    })
}

fn decode_import_job(row: ImportJobRow) -> Result<ImportJob> {
    Ok(ImportJob {
        id: row.id,
        job_id: row.job_id,
        account_id: row.account_id,
        pseud_id: row.pseud_id,
        source_key: row.source_key,
        source_url: row.source_url,
        destination_type: row.destination_type,
        dry_run: row.dry_run != 0,
        state: row.state,
        library_item_id: row.library_item_id,
        report_json: row.report_json,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn decode_import_chapter(row: ImportChapterRow) -> Result<ImportChapter> {
    Ok(ImportChapter {
        id: row.id,
        import_job_id: row.import_job_id,
        library_item_id: row.library_item_id,
        source_chapter_key: row.source_chapter_key,
        ordinal: row.ordinal,
        title: row.title,
        state: row.state,
        content_blob_checksum: row.content_blob_checksum,
        chapter_id: row.chapter_id,
        note: row.note,
    })
}

fn decode_credential(row: SourceCredentialRow) -> Result<SourceCredential> {
    Ok(SourceCredential {
        id: row.id,
        pseud_id: row.pseud_id,
        source_key: row.source_key,
        secret_id: row.secret_id,
        label: row.label,
        status: row.status,
        expires_at: row.expires_at,
        last_checked_at: row.last_checked_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
        version: row.version,
        kind: row.kind,
        origin_host: row.origin_host,
        consent_at: row.consent_at,
    })
}

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

async fn fetch_optional_library_item(
    db: &Database,
    sql: &str,
    account_id: &str,
    source_key: &str,
    source_work_key: &str,
) -> Result<Option<LibraryItem>> {
    let row: Option<LibraryItemRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(sql)
                .bind(account_id)
                .bind(source_key)
                .bind(source_work_key)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(sql)
                .bind(account_id)
                .bind(source_key)
                .bind(source_work_key)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    row.map(decode_library_item).transpose()
}
