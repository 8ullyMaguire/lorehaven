//! The import service: one queued import, carried out (spec §11.3, §11.4).
//!
//! This is where a reader's request becomes a library item. It is deliberately
//! the only place that knows the whole sequence, because the sequence is a
//! policy decision and policies belong in one place:
//!
//! 1. the source must exist and be switched on, checked *before* anything is
//!    fetched, so a paused source costs no requests at all;
//! 2. the credential must exist and not be expired, also checked before any
//!    fetch, so a stale login is a message rather than a half-written item;
//! 3. preview, so the plan can be reported before a single chapter is stored;
//! 4. fetch and store chapter by chapter, checking for cancellation between
//!    units and skipping what a previous attempt already stored;
//! 5. upsert the library item and write the report.
//!
//! Every step that can fail returns [`HandlerError`], so the queue's own retry
//! policy decides what happens next: a source that timed out is transient, a
//! work that no longer exists is fatal, and neither needs a second mechanism.
//!
//! # What is deliberately not here
//!
//! Nothing in this module parses HTML. The adapter does that and returns
//! sanitised chapters, so a change to a site's markup never reaches this file,
//! and the retry, resume and reporting rules never depend on a site's shape
//! (spec §11.1).

use anyhow::Result;
use lorehaven_db::storage::BlobStore;
use lorehaven_db::{imports, jobs};
use lorehaven_domain::imports::{
    plan_import, ChapterChange, ChapterIdentity, ImportPlan, ImportedWork,
};
use lorehaven_domain::JobId;
use lorehaven_scrapers::registry::Registry;
use lorehaven_scrapers::{
    Credentials, FetchPolicy, Fetcher, SafeFetcher, SourceAdapter, SourceKey, SourceWork,
};

use crate::config::{Config, RunScope};

/// The fetch policy for one source, as this instance builds it.
///
/// The adapter's capabilities set the politeness floor; the instance's import
/// settings add what it is willing to run when the source refuses a plain
/// request. Built here rather than at each call site so that a fetch path added
/// later cannot arrive without the escalation settings attached — a preview that
/// quietly did not escalate would fail on exactly the sources the import could
/// read, which is the kind of difference nobody notices until a reader reports
/// it.
pub fn policy_for(adapter: &dyn SourceAdapter, config: &Config) -> FetchPolicy {
    policy_for_in(adapter, config, None)
}

/// The fetch policy for a run of `source_key`, with that run's overrides applied.
///
/// The one way a run builds a policy. `run()` calls this rather than pairing
/// `policy_for_in` with a scope it built itself, because that pairing is
/// exactly where a dropped scope hides: both halves are correct, the call site
/// is wrong, and nothing fails. Taking the source key here means a run cannot
/// fetch under a posture that its own overrides do not describe.
pub fn run_policy(adapter: &dyn SourceAdapter, config: &Config, source_key: &str) -> FetchPolicy {
    let scope = run_scope_for(config, source_key);
    policy_for_in(adapter, config, Some(&scope))
}

/// The temporary overrides in force for a run of `source_key`.
///
/// Extracted from `run()` because the selection has rules worth testing and
/// `run()` is not testable at that level: which overrides are granted, to
/// which source, and which are not granted at all. While this was inline in
/// `run()`, two mutations survived — dropping the scope entirely, and inverting
/// the `persistent` check — because nothing outside `run()` could reach the
/// decision. A rule that only the largest function in the file can exercise is
/// a rule that is not tested.
///
/// A `persistent: false` override is the operator saying "this source, this
/// time". It reaches the run through the FILE — `posture_for_source` already
/// returns it for every lookup, inside a run or not — and the scope exists for
/// grants that come from somewhere ELSE: a caller that has to unblock one fetch
/// part-way through a run it has already started.
///
/// An earlier version had this function re-grant the file's `persistent: false`
/// entries. That was dead weight, and provably so: the file's answer and the
/// grant's answer were the same posture for the same source, so
/// `narrowest(run, file)` returned the same value either way. Mutation F4 —
/// deleting the scope from `run_policy` — stayed green, because the scope
/// genuinely could not change any outcome. A second path to an answer that the
/// first path already gives is not redundancy, it is a second answer, and it
/// costs a test to try to pin down.
///
/// So the split is: the file says what a source runs under (permanently or for
/// the next run, per `persistent`), and a `RunScope` carries only what a caller
/// decided mid-run. Both are narrowed to the file's posture on read, so neither
/// can widen anything.
pub fn run_scope_for(config: &Config, source_key: &str) -> RunScope {
    // Nothing is granted from the file: `posture_for_source` already returns a
    // `persistent: false` override for every lookup, so a grant here would be a
    // second copy of an answer that is already correct. The scope is the channel
    // for grants that come from somewhere else, and today no caller in this
    // crate needs one — so the ordinary run's scope is empty and a caller that
    // does need to unblock a fetch mid-run builds its own `RunScope` and passes
    // it to `policy_for_in`.
    let _ = (config, source_key);
    RunScope::none()
}

/// The fetch policy for one source, given a run that may be holding temporary
/// overrides of its own.
///
/// The two-argument form is what an ad-hoc request (a preview, a probe, a
/// connectivity check) should call: there is no run, so there is nothing a
/// run-scoped override could legitimately apply to. `run()` passes
/// `Some(&scope)`. Keeping both, rather than defaulting the parameter, is
/// deliberate — a default argument would let a future call site pick up run
/// overrides by accident, and an ad-hoc fetch silently inheriting one run's
/// grant is precisely the leak `RunScope` exists to prevent.
pub fn policy_for_in(
    adapter: &dyn SourceAdapter,
    config: &Config,
    run: Option<&RunScope>,
) -> FetchPolicy {
    let mut policy = FetchPolicy::for_source(adapter.capabilities());
    policy.unblock = config.imports.unblock_for(adapter);
    // The instance's answer to the source's own `robots.txt`, applied here so
    // every import path — preview, chapter fetch, update check — gets the same
    // one. Defaults to compliance; see `ImportsConfig::resolved_robots_posture`.
    //
    // The posture is the field the fetcher reads, so it is set here rather than
    // leaving `honour_robots` to be consulted at the point of use. Two fields
    // that both look authoritative and disagree is the shape this change
    // exists to remove.
    // Per source, not per instance: an override for this adapter's key narrows
    // what this fetch may do, and the narrowing was already checked against the
    // instance posture at config load. Read through `posture_for_source` rather
    // than the instance posture directly, so an adapter cannot be exempt from an
    // operator's setting by being on a code path that forgot to ask.
    policy.robots_posture = config
        .imports
        .posture_for_source_in(adapter.key().as_str(), run);
    // Kept in step with the posture so a caller that still reads the
    // compatibility field sees the same answer rather than the raw config
    // value, which may have been overridden by a deliberate posture.
    //
    // The question this answers is "does this policy override the rules?", so
    // the bool is true for exactly the permissive posture. An earlier version
    // asked "is the posture permissive?" and assigned that instead, which set
    // the field to the answer to its own question — so the compatibility field
    // read `false` on a strict instance and `true` on a permissive one, and
    // the existing test that asserts it caught the inversion. The posture, not
    // the bool, is what the fetcher reads, so nothing was crawling the wrong
    // way; a reader of the deprecated field was simply being told the opposite
    // of the truth.
    policy.honour_robots = !matches!(
        policy.robots_posture,
        lorehaven_scrapers::robots::RobotsPosture::Permissive
    );
    policy
}
use serde_json::{json, Value};
use time::OffsetDateTime;

use crate::state::AppState;
use crate::worker::HandlerError;

/// Where the import job's id lives in the queue payload.
pub const PAYLOAD_IMPORT_JOB_ID: &str = "import_job_id";

/// The owner type recorded in `content_references` for a stored chapter.
///
/// A constant because it is a foreign key in all but name: the blob collector
/// asks `content_references` whether a checksum is still wanted, and a typo here
/// would make every imported chapter look unreferenced and quietly deletable.
pub const CHAPTER_OWNER_TYPE: &str = "library_item_chapter";

/// The header a credential is carried in, absent a source-specific choice.
///
/// A header rather than a query parameter, because query strings end up in
/// access logs, in `Referer` and in error messages, and a credential that leaks
/// into any of those has to be replaced rather than apologised for.
pub const CREDENTIAL_HEADER: &str = "Authorization";

/// How much of the progress bar the preview owns, in permille.
const PREVIEW_PERMILLE: i64 = 50;

/// Turn any error that is not a decision into a retryable one.
///
/// The distinction that matters for the queue is not "database or network" but
/// "would trying again help". A database that is briefly unavailable would; a
/// work that does not exist would not, and says so itself.
fn transient(error: impl std::fmt::Display) -> HandlerError {
    HandlerError::Transient(error.to_string())
}

/// A failure the import has already explained, ready to be recorded.
fn fatal(message: impl Into<String>) -> HandlerError {
    HandlerError::Fatal(message.into())
}

/// Recompute the source catalogue's health from the import history (spec §11.8).
///
/// Called by the worker after every import attempt and by an operator route on
/// demand. It sweeps *every* source rather than the one just touched, for two
/// reasons: the answer for a source nobody has tried is "no change", so the
/// extra work is one aggregate query, and a sweep that only looked at the source
/// that happened to run would never notice a source that has stopped being used
/// at all — which is precisely the source an operator wants to hear about.
///
/// The job id is taken rather than the source key because it is what the queue
/// has, and because a job whose import row has vanished has nothing to
/// attribute: that case returns empty rather than sweeping on a phantom.
///
/// # Errors
/// A database failure. The caller decides what that means: the worker logs it
/// and keeps the import's own outcome, because a health row is not worth
/// retrying a work that was fetched correctly.
pub async fn settle_source_health(
    state: &AppState,
    import_job_id: &str,
) -> Result<Vec<imports::SourceHealthChange>> {
    if imports::get_import_job(state.db(), import_job_id)
        .await?
        .is_none()
    {
        return Ok(Vec::new());
    }
    imports::recompute_source_health(state.db(), imports::HEALTH_WINDOW_DAYS).await
}

/// Carry out one queued import.
///
/// # Errors
/// Returns the failure the queue should act on: transient for anything that
/// might work next time, fatal for the decisions a retry cannot change.
pub async fn run(
    state: &AppState,
    registry: &Registry,
    job: JobId,
    payload: &Value,
) -> Result<(), HandlerError> {
    let import_job_id = payload
        .get(PAYLOAD_IMPORT_JOB_ID)
        .and_then(Value::as_str)
        .ok_or_else(|| {
            fatal(format!(
                "an import payload must carry {PAYLOAD_IMPORT_JOB_ID}"
            ))
        })?;

    let row = imports::get_import_job(state.db(), import_job_id)
        .await
        .map_err(transient)?
        .ok_or_else(|| fatal(format!("import {import_job_id} no longer exists")))?;

    let source_key = SourceKey::new(row.source_key.clone());
    let adapter = registry.by_key(&source_key).map_err(|error| {
        fatal(format!(
            "no adapter for source {:?}: {error}",
            row.source_key
        ))
    })?;

    // Rule 1: a source that is switched off costs no requests. The check is
    // first, and the message names the reason, because "the import failed" with
    // no explanation is what makes an operator look in the wrong place.
    let source = imports::find_source(state.db(), &row.source_key)
        .await
        .map_err(transient)?;
    if let Some(source) = &source {
        if !source.enabled {
            let reason = source
                .disabled_reason
                .clone()
                .unwrap_or_else(|| "no reason was recorded".to_owned());
            imports::set_import_state(
                state.db(),
                import_job_id,
                "failed",
                None,
                Some(&json!({ "error": "source_disabled", "source": row.source_key, "reason": reason }).to_string()),
            )
            .await
            .map_err(transient)?;
            return Err(fatal(format!(
                "the {:?} source is paused: {reason}",
                row.source_key
            )));
        }
    }

    // Rule 2: a credential that cannot be used is reported before anything is
    // fetched, so the reader finds out by reading a message and not by finding
    // half an item in their library.
    let credentials = match resolve_credentials(state, &row, adapter).await? {
        CredentialOutcome::NotNeeded => None,
        CredentialOutcome::Ready(credentials) => Some(credentials),
        CredentialOutcome::Missing => {
            let message = format!(
                "the {:?} source needs a saved credential; add one and start the import again",
                row.source_key
            );
            fail_import(state, import_job_id, "credential_missing", &message).await?;
            return Err(fatal(message));
        }
        CredentialOutcome::Expired(at) => {
            let message = format!(
                "the saved {:?} credential expired on {at}; replace it and start the import again",
                row.source_key
            );
            fail_import(state, import_job_id, "credential_expired", &message).await?;
            return Err(fatal(message));
        }
        CredentialOutcome::Refused(message) => {
            // `credential_refused`, not `credential_expired`: the three causes
            // have three different fixes and an operator reading the import row
            // has to be able to tell them apart. `fatal` and not a retry,
            // because retrying sends the same credential the policy just refused.
            fail_import(state, import_job_id, "credential_refused", &message).await?;
            return Err(fatal(message));
        }
    };

    // The run's temporary overrides, gathered once and dropped when this
    // function returns. Nothing to clean up: the value goes out of scope with
    // the run, which is the whole of the expiry mechanism (see `RunScope`).
    // `run_policy`, not `policy_for` + a scope built by hand. This call site was
    // the last place that could forget the scope, and a mutation that dropped it
    // (`Some(&run_scope)` -> `None`) survived the whole suite, because no test
    // runs `run()`. Collapsing the pair into one function that cannot be called
    // without a scope means the omission is no longer expressible here: a
    // mutation can only break the function's body, which the unit tests reach.
    let policy = run_policy(adapter, state.config(), &row.source_key);
    let mut fetcher = SafeFetcher::new(adapter.hosts(), policy);
    let mut credentialed = false;
    if let Some(credentials) = &credentials {
        if !credentials.secret.is_empty() {
            credentialed = true;
            // The header goes to the source's own host and to nothing a
            // redirect leads to; the fetcher enforces both halves.
            let host = adapter.hosts().into_iter().next().unwrap_or_default();
            fetcher = fetcher
                .with_credential_header(host, CREDENTIAL_HEADER, &credentials.secret)
                .map_err(|error| fatal(format!("the saved credential cannot be sent: {error}")))?;
        }
    }

    // Reads are filed under the source and under *who* they were made as. A
    // page fetched with a reader's credential is a different resource from the
    // same URL fetched anonymously, and serving one for the other would hand
    // somebody gated content — so the scope is part of the cache key rather
    // than a detail of the fetch (spec §10.4).
    let scope = if credentialed && !row.pseud_id.is_empty() {
        row.pseud_id.clone()
    } else {
        crate::revisions::PUBLIC_SCOPE.to_owned()
    };
    let adapter_version = source
        .as_ref()
        .map_or("0", |record| record.adapter_version.as_str())
        .to_owned();
    let fetcher = crate::revisions::CachingFetcher::new(
        fetcher,
        state.db(),
        state.config().storage.root.clone(),
        &row.source_key,
        &adapter_version,
        &scope,
        state.config().revisions.ttl_secs,
    );

    // Rule 3: preview first, so the plan is a fact before any body is stored.
    let url = row
        .source_url
        .parse::<url::Url>()
        .map_err(|error| fatal(format!("the stored source url is not a url: {error}")))?;
    imports::set_import_state(state.db(), import_job_id, "running", None, None)
        .await
        .map_err(transient)?;

    // A preview that fails is where a vanished origin is DETECTED: the source
    // answered, and its answer is that the work is not there or may not be
    // served. `classify` has already decided whether that is fatal (it is, for
    // both) and has produced the human sentence; what was missing is the record
    // that lets anything later ask "which works is this instance past saving?".
    //
    // Before this, a `not_found` or `withheld` propagated out of `run` and left
    // `import_jobs.state` at `running` — an import that is neither queued for a
    // retry nor recorded as failed, which is the one state the schema's
    // vocabulary has no word for. §11.15's `works_past_saving` needs a recorded
    // answer here, and a work whose origin has vanished is precisely the case
    // the count is about.
    //
    // The code is the *class*, not the message. `classify`'s message is prose
    // that may be reworded; the code is what the count reads, so it has to be a
    // stable word rather than a substring of a sentence. `not_found` and
    // `withheld` are the two that mean the work is GONE; a rate limit or a
    // network fault is transient and is left for the queue to retry.
    let work = match adapter
        .preview(&fetcher, &url, credentials.as_deref())
        .await
    {
        Ok(work) => work,
        Err(error) => {
            // Matched on the TYPED error, not on the rendered sentence. A
            // substring test over `classify`'s message would break the day
            // somebody rewords the prose, and the reworded build would
            // silently stop recording vanished origins — the count would go to
            // zero and look like good news.
            if let Some(code) = origin_gone_code(&error) {
                let classified = classify(error, &row.source_key);
                if let Err(record_error) =
                    fail_import(state, import_job_id, code, &classified.message()).await
                {
                    tracing::warn!(
                        %record_error,
                        "the source reported the work as gone and the import could not \
                         be marked failed"
                    );
                }
                return Err(classified);
            }
            return Err(classify(error, &row.source_key));
        }
    };

    // The planner compares the source's view against what is already held, so a
    // re-import reports what changed rather than importing blind. A missing item
    // is `None`, which is a creation — not an empty item, which would read as an
    // update that dropped every chapter.
    let existing_item = imports::find_library_item(
        state.db(),
        &row.account_id,
        &row.source_key,
        &work.source_work_key,
    )
    .await
    .map_err(transient)?;
    let existing_chapters = imports::previous_chapters_for(
        state.db(),
        &row.account_id,
        &row.source_key,
        &work.source_work_key,
    )
    .await
    .map_err(transient)?;

    let held = existing_item.as_ref().map(|item| ImportedWork {
        title: item.title.clone(),
        author_text: item.author_text.clone(),
        chapters: existing_chapters
            .clone()
            .unwrap_or_default()
            .iter()
            .map(to_identity)
            .collect(),
        word_count: None,
    });
    let plan = plan_import(
        held.as_ref(),
        &ImportedWork {
            title: work.title.clone(),
            author_text: work.author_text.clone(),
            chapters: work.chapters.iter().map(to_identity_ref).collect(),
            word_count: None,
        },
    );

    progress(
        state,
        job,
        PREVIEW_PERMILLE,
        Some(work.source_work_key.as_str()),
    )
    .await?;

    if row.dry_run {
        // A dry run reports and stops. Nothing has been written but the report,
        // which is the promise `POST /imports/preview` makes.
        finish(
            state,
            &row,
            import_job_id,
            "completed",
            &report(&plan, &[], true),
        )
        .await?;
        return Ok(());
    }

    // The item exists before its chapters so that the blobs have an owner to be
    // referenced by, and so a cancelled import leaves a readable stub rather
    // than orphaned content.
    let item_id = row
        .library_item_id
        .clone()
        .unwrap_or_else(|| lorehaven_domain::LibraryItemId::new().to_string());
    let item = imports::upsert_library_item(
        state.db(),
        &row.account_id,
        &row.source_key,
        &work.source_work_key,
        &library_input(&work, &row),
    )
    .await
    .map_err(transient)?;

    let source = SourceFetch {
        registry,
        fetcher: &fetcher,
        credentials: credentials.as_deref(),
        work: &work,
    };
    // §11.15 non-degradation, in one direction: "On a caching instance, a body
    // that fails to fetch is a failed import that retries and eventually
    // reports — never a work quietly reclassified as a link, because that turns
    // a temporary source failure into a permanent loss of something the reader
    // asked this instance to keep."
    //
    // The library item was written at the line above, before the chapters, so
    // that a cancelled import leaves a readable stub rather than orphaned
    // content. That is the right order for a *cancel*, and it is exactly the
    // shape the spec forbids for a *failure*: the item now has a title, a
    // summary and a `source_url`, and a reader looking at it sees a work that
    // looks imported and is not. So the failure is recorded on the import row —
    // `state = 'failed'` with a machine-readable code — and the error still
    // propagates so the queue retries. The item is left in place deliberately:
    // deleting it would be the deletion workflow of §10.4, not an import's
    // decision to make.
    //
    // The other direction needs no code here and that is worth stating: an
    // aggregating instance returns `Ok(vec![])` from `store_chapters` and is
    // marked `completed`, because storing no body is what it was configured to
    // do. Only a *failure* on a *caching* instance is the degradation §11.15
    // names.
    let stored = match store_chapters(state, &source, job, &row, &item).await {
        Ok(stored) => stored,
        Err(error) => {
            let message = error.message();
            // The write is best-effort in the sense that a failure to record
            // must not replace the real error with a different one: the reader
            // needs to know the body fetch failed, and "could not write the
            // failure" would be a second, worse problem. So this logs and the
            // original error is what propagates.
            if let Err(record_error) =
                fail_import(state, import_job_id, "body_fetch_failed", &message).await
            {
                tracing::warn!(
                    %record_error,
                    "a body fetch failed and the import could not be marked failed"
                );
            }
            return Err(error);
        }
    };

    // §32.7.9: rescue media. Every image URL found in the chapters is
    // deduplicated and queued for archival. A URL already held by any work
    // returns its existing reference; a new URL creates a reference with a
    // placeholder hash and an availability link. The health monitor fills in
    // the hash on its first check.
    let media_summary = rescue_import_media(state.db(), &item, &stored).await;

    let report_json = report_with_media(&plan, &stored, false, &media_summary);

    finish(state, &row, import_job_id, "completed", &report_json).await?;
    let _ = item_id;
    Ok(())
}

/// What resolving a source's credential ended in.
enum CredentialOutcome {
    /// The source does not need one.
    NotNeeded,
    /// One was found, and it has not expired.
    Ready(Box<Credentials>),
    /// The source needs one and this account has none for it.
    Missing,
    /// One exists and its expiry has passed.
    Expired(String),
    /// One exists but §11.6 does not permit sending it: unbound, bound to a
    /// different host, or stored without a consent record.
    ///
    /// Carries the store's own message rather than an enum, because the refusal
    /// text is a requirement of §11.6's "actionable status" and rebuilding it at
    /// the call site is how two call sites end up saying different things about
    /// the same requirement.
    Refused(String),
}

/// Find and decrypt the credential this import should use.
///
/// A credential is pseud-scoped (spec §11.6), so the search is by the import's
/// own pseud rather than by account: one reader's login for a source must not
/// become usable by another of their identities, and an import started as one
/// pseud must not silently authenticate as another.
async fn resolve_credentials(
    state: &AppState,
    row: &imports::ImportJob,
    adapter: &dyn SourceAdapter,
) -> Result<CredentialOutcome, HandlerError> {
    let capabilities = adapter.capabilities();
    let stored = imports::list_source_credentials(state.db(), &row.pseud_id, Some(&row.source_key))
        .await
        .map_err(transient)?;

    if stored.is_empty() {
        return Ok(match capabilities.authentication {
            lorehaven_scrapers::AuthKind::None => CredentialOutcome::NotNeeded,
            _ => CredentialOutcome::Missing,
        });
    }

    // Most recently used first is the order the repository returns; a reader who
    // saved two logins gets the one they last used, not an arbitrary one.
    let Some(chosen) = stored
        .into_iter()
        .find(|credential| credential.status != "revoked")
    else {
        return Ok(CredentialOutcome::Missing);
    };

    if let Some(expires_at) = &chosen.expires_at {
        if expired(expires_at, OffsetDateTime::now_utc()) {
            imports::set_credential_status(state.db(), &chosen.id, "expired", true)
                .await
                .map_err(transient)?;
            // §11.6's audit trail, and `expired` is an event of its own rather
            // than a use: it is the moment the credential stopped being usable,
            // and a trail that only recorded successes would date the end of a
            // login from its next successful use, which can be never.
            imports::record_credential_event(
                state.db(),
                &chosen.id,
                &chosen.pseud_id,
                imports::CredentialEvent::Expired,
            )
            .await
            .map_err(transient)?;
            return Ok(CredentialOutcome::Expired(expires_at.clone()));
        }
    }

    if chosen.secret_id.is_empty() {
        return Ok(CredentialOutcome::Missing);
    }

    // §11.6 origin binding, enforced on the path that actually decrypts. The
    // expiry check above reads only a date; this is the first point at which the
    // secret would leave the process, so this is where the host it is bound to
    // has to match. Refusing here means the import pauses with the store's own
    // message rather than a fetch that a lookalike domain receives.
    //
    // The host comes from the adapter rather than the URL being fetched: the
    // adapter's declared host is what §11.6 means by "origin", and a fetcher
    // could legitimately be pointed at a path on that host, but never at another
    // host with the credential attached.
    if let Some(host) = adapter.hosts().into_iter().next() {
        if let Err(refusal) = chosen.usable_for(&host, &rfc3339_now()) {
            return Ok(CredentialOutcome::Refused(refusal.message()));
        }
    }

    let cipher = crate::secrets::load_cipher(
        &state.config().storage.root,
        state.config().security.secret_key_file.as_deref(),
        !state.config().environment.is_development(),
    )
    .map_err(|error| fatal(format!("the instance's secret key is unusable: {error}")))?;

    let secret = crate::secrets::open_secret(state.db(), &cipher, &chosen.secret_id)
        .await
        .map_err(transient)?
        .ok_or_else(|| fatal("the saved credential's secret row is gone"))?;

    imports::set_credential_status(state.db(), &chosen.id, "active", true)
        .await
        .map_err(transient)?;

    // `used`, recorded at the moment of use and not at the moment of storage, so
    // "when did we last send this login to the source" has an answer that is not
    // "when did the reader save it".
    imports::record_credential_event(
        state.db(),
        &chosen.id,
        &chosen.pseud_id,
        imports::CredentialEvent::Used,
    )
    .await
    .map_err(transient)?;

    Ok(CredentialOutcome::Ready(Box::new(Credentials {
        source_key: SourceKey::new(row.source_key.clone()),
        username: chosen.label.clone(),
        secret,
        adult_allowed: false,
    })))
}

/// The current instant in the shape the credential columns hold.
///
/// One function rather than a second call to `now_utc` formatted inline, because
/// `usable_for` compares against an RFC 3339 string and the two formats have to
/// agree: a `usable_for` call that formatted the instant differently would refuse
/// every credential, which is a failure that looks like a feature working.
fn rfc3339_now() -> String {
    crate::routes::imports::format_time(OffsetDateTime::now_utc())
}

/// Whether an RFC 3339 instant is in the past.
///
/// Unparseable counts as expired: a date we cannot read is not a promise that
/// the credential is still good, and treating it as valid would mean sending a
/// credential that the source has already rejected.
fn expired(raw: &str, now: OffsetDateTime) -> bool {
    match OffsetDateTime::parse(raw, &time::format_description::well_known::Rfc3339) {
        Ok(at) => at <= now,
        Err(_) => true,
    }
}

/// Turn a source failure into the queue's decision.
///
/// The mapping is the whole point of the error categories: `NotFound` is fatal
/// because the work will not come back, `RateLimited` and `Blocked` are
/// transient because the site is asking for patience, and a credential the
/// source rejected is fatal because retrying will send the same rejected
/// credential five more times.
fn classify(error: lorehaven_scrapers::SourceError, source_key: &str) -> HandlerError {
    use lorehaven_scrapers::SourceError as E;
    match error {
        E::NotFound => fatal(format!(
            "the {source_key} source has no work at that address; check the link"
        )),
        E::Unsupported(message) => fatal(format!("{source_key} cannot do this: {message}")),
        E::AuthRequired(message) => fatal(format!(
            "the {source_key} source refused the credential: {message}"
        )),
        E::Parse(message) => fatal(format!(
            "{source_key} sent a page this build cannot read: {message}"
        )),
        E::Refused(message) => fatal(format!("{source_key}: {message}")),
        // A moderation hold, a withdrawn work, a takedown in progress: the source
        // answered, and its answer is no. Fatal rather than transient because a
        // hold is a state the *source* is in rather than a refusal aimed at us —
        // retrying asks the same question and gets the same answer, five times,
        // while a reader waits for an import that will never arrive.
        E::Withheld(message) => fatal(format!(
            "the {source_key} source holds that work but will not serve it: {message}"
        )),
        // Blocked and rate-limited are the same instruction to us: wait. A
        // retry budget is the queue's job, not this function's.
        E::RateLimited(message) => {
            HandlerError::Transient(format!("{source_key} is rate limiting us: {message}"))
        }
        E::Blocked => HandlerError::Transient(format!(
            "{source_key} refused the request as automated; it may pass on a later attempt"
        )),
        E::Network(message) => HandlerError::Transient(format!("{source_key}: {message}")),
        E::Internal(message) => HandlerError::Transient(format!("{source_key}: {message}")),
    }
}

/// The code to record when the source says the work is gone, or `None`.
///
/// Two variants and no more, because the two mean different things and only one
/// of them is a loss. `NotFound` is a dead link; `Withheld` is a work the source
/// holds and will not serve — a moderation hold, a takedown in progress. Both are
/// states the *source* is in, both are fatal rather than transient, and both mean
/// this instance cannot get the text again without the source changing its mind.
///
/// Everything else is deliberately excluded, and the exclusions are the point:
///
/// * `RateLimited`, `Blocked`, `Network` — transient. The work is not gone, the
///   fetch was unlucky, and the queue is already retrying. Counting these would
///   make a busy afternoon look like a preservation debt.
/// * `AuthRequired` — a credential problem. An operator who logs in again fixes
///   it, and the work is still there.
/// * `Parse`, `Unsupported`, `Internal`, `Refused` — this instance's problem, not
///   the origin's. A parser that cannot read a page says nothing about whether
///   the work exists.
fn origin_gone_code(error: &lorehaven_scrapers::SourceError) -> Option<&'static str> {
    use lorehaven_scrapers::SourceError as E;
    match error {
        E::NotFound => Some("not_found"),
        E::Withheld(_) => Some("withheld"),
        _ => None,
    }
}

/// Record progress, ignoring a lost lease.
///
/// A progress row is a courtesy to whoever is watching; failing a running import
/// because its progress could not be written would be the tail wagging the dog.
async fn progress(
    state: &AppState,
    job: JobId,
    permille: i64,
    checkpoint: Option<&str>,
) -> Result<(), HandlerError> {
    jobs::progress(state.db(), job, permille.clamp(0, 1000), checkpoint)
        .await
        .map_err(transient)
}

/// Mark an import failed with a machine-readable code and a human message.
async fn fail_import(
    state: &AppState,
    id: &str,
    code: &str,
    message: &str,
) -> Result<(), HandlerError> {
    imports::set_import_state(
        state.db(),
        id,
        "failed",
        None,
        Some(&json!({ "error": code, "message": message }).to_string()),
    )
    .await
    .map_err(transient)
}

/// The library row for a previewed work.
fn library_input(work: &SourceWork, row: &imports::ImportJob) -> imports::LibraryItemInput {
    imports::LibraryItemInput {
        title: work.title.clone(),
        author_text: work.author_text.clone(),
        author_url: work.author_url.clone(),
        summary: work.summary.clone(),
        language: work.language.clone(),
        word_count: work.word_count,
        status: work.status.as_str().to_owned(),
        source_url: work.source_url.clone(),
        source_updated_at: work.updated_at.map(|at| {
            at.format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| at.to_string())
        }),
        provenance_json: json!({
            "source": row.source_key,
            "source_url": row.source_url,
            "import_job_id": row.id,
        })
        .to_string(),
    }
}

/// The planner's view of one stored chapter.
fn to_identity(chapter: &imports::ImportChapter) -> ChapterIdentity {
    ChapterIdentity {
        source_chapter_key: chapter.source_chapter_key.clone(),
        ordinal: u32::try_from(chapter.ordinal).unwrap_or(u32::MAX),
        title: chapter.title.clone(),
        word_count: None,
    }
}

/// The planner's view of one chapter a source just reported.
fn to_identity_ref(chapter: &lorehaven_scrapers::ChapterRef) -> ChapterIdentity {
    ChapterIdentity {
        source_chapter_key: chapter.source_chapter_key.clone(),
        ordinal: chapter.ordinal,
        title: chapter.title.clone(),
        word_count: None, // Preview doesn't count words; grading will be Held until import
    }
}

/// What one chapter's storage ended in.
#[derive(Debug, Clone)]
struct StoredChapter {
    ordinal: u32,
    source_chapter_key: String,
    title: String,
    checksum: Option<String>,
    note: Option<String>,
    stored: bool,
    /// Image URLs found in this chapter's source body, for media rescue (§32.7.9).
    image_urls: Vec<String>,
}

/// Everything a chapter-store needs from the source side.
///
/// Grouped into one struct because the alternative is a seven-argument function
/// whose arguments are all references to things that must agree with each other
/// — the fetcher's allow-list came from the adapter, and the work came from that
/// adapter's preview. Passing them separately invites a caller to mix two
/// sources' halves.
struct SourceFetch<'a> {
    registry: &'a Registry,
    fetcher: &'a dyn Fetcher,
    credentials: Option<&'a Credentials>,
    work: &'a SourceWork,
}

/// Fetch and store a work's chapters, resuming rather than starting over.
///
/// The rule this function exists to hold: **a retry does not re-read what is
/// already stored.** On the first attempt every chapter is fetched in one bulk
/// request, because one request per work is what a source would rather receive.
/// On a retry, only the chapters that failed are fetched again, and only where
/// the adapter can address a single chapter — which it advertises, so a source
/// that cannot is not asked to pretend (§11.7).
async fn store_chapters(
    state: &AppState,
    source: &SourceFetch<'_>,
    job: JobId,
    row: &imports::ImportJob,
    item: &imports::LibraryItem,
) -> Result<Vec<StoredChapter>, HandlerError> {
    let registry = source.registry;
    let fetcher = source.fetcher;
    let work = source.work;
    let credentials = source.credentials;
    let adapter = registry
        .by_key(&SourceKey::new(row.source_key.clone()))
        .map_err(|error| {
            fatal(format!(
                "no adapter for source {:?}: {error}",
                row.source_key
            ))
        })?;

    // §11.15: an aggregating instance never fetches a body into its storage. The
    // check is HERE, at the one place a body is about to be stored, rather than
    // at the fetch site — because the property is about *storage*, and a
    // `Metadata` fetch that happened to return prose is still prose this
    // instance must not keep. Checking earlier would put the rule in two places
    // (the fetch and the write) and the write is the one that cannot be bypassed.
    //
    // The refusal is `Ok(vec![])` and not an `Err`, and that is deliberate: an
    // aggregating instance importing a work is a *success*. It has the metadata,
    // the attribution and the canonical link, which is the whole of what §11.15
    // says such an instance produces. Returning an error would mark the import
    // failed, and the reader would see a failed import for a work that was in
    // fact imported exactly as this instance intends.
    let retention = lorehaven_db::retention::resolve_for_source(
        state.db(),
        Some(&row.source_key),
        /* source_blocked */ false,
        /* vanished */ false,
    )
    .await
    .map_err(transient)?;
    if let Err(refusal) =
        lorehaven_domain::retention::check_body_allowed(&retention, Some(&row.source_key))
    {
        tracing::info!(
            source = %row.source_key,
            code = refusal.code(),
            "an aggregating instance is not storing this work's body"
        );
        return Ok(Vec::new());
    }

    let previous = imports::list_import_chapters(state.db(), &row.id)
        .await
        .map_err(transient)?;
    let already_stored: std::collections::HashSet<String> = previous
        .iter()
        .filter(|chapter| chapter.state == "stored")
        .map(|chapter| chapter.source_chapter_key.clone())
        .collect();
    let failed: Vec<u32> = previous
        .iter()
        .filter(|chapter| chapter.state == "failed")
        .filter_map(|chapter| u32::try_from(chapter.ordinal).ok())
        .collect();

    let capabilities = adapter.capabilities();
    // Bulk for a fresh import; per-chapter for a retry, and only when the source
    // genuinely addresses chapters one at a time. Anything else is a bulk fetch
    // whose already-stored chapters are skipped rather than re-read.
    let per_chapter_retry =
        !previous.is_empty() && !failed.is_empty() && capabilities.per_chapter_fetch;

    let fetched = if per_chapter_retry {
        let mut chapters = Vec::new();
        for ordinal in &failed {
            if jobs::is_cancelled(state.db(), job)
                .await
                .map_err(transient)?
            {
                return Err(HandlerError::Cancelled);
            }
            match adapter
                .fetch_chapter(fetcher, work, *ordinal, credentials)
                .await
            {
                Ok(chapter) => chapters.push(chapter),
                Err(error) => {
                    let note = classify(error, &row.source_key).message();
                    imports::upsert_import_chapter(
                        state.db(),
                        &row.id,
                        Some(&item.id),
                        &imports::ImportChapterInput {
                            source_chapter_key: format!("ordinal-{ordinal}"),
                            ordinal: i64::from(*ordinal),
                            title: String::new(),
                            state: "failed".to_owned(),
                            content_blob_checksum: None,
                            note: Some(note),
                        },
                    )
                    .await
                    .map_err(transient)?;
                    tracing::warn!(ordinal, source = %row.source_key, "a chapter could not be re-read");
                }
            }
        }
        chapters
    } else {
        adapter
            .fetch_chapters(fetcher, work, credentials)
            .await
            .map_err(|error| classify(error, &row.source_key))?
    };

    let store = BlobStore::new(state.config().storage.root.clone());
    let total = i64::try_from(fetched.len()).unwrap_or(i64::MAX).max(1);
    let mut outcomes = Vec::with_capacity(fetched.len());
    let mut any_failed = false;

    for (index, chapter) in fetched.iter().enumerate() {
        // Between units of work: a ten-minute import that ignores a cancel is a
        // lie about what cancelling does.
        if jobs::is_cancelled(state.db(), job)
            .await
            .map_err(transient)?
        {
            return Err(HandlerError::Cancelled);
        }

        if already_stored.contains(&chapter.source_chapter_key) {
            outcomes.push(StoredChapter {
                ordinal: chapter.ordinal,
                source_chapter_key: chapter.source_chapter_key.clone(),
                title: chapter.title.clone(),
                checksum: None,
                note: Some("already held from an earlier attempt".to_owned()),
                stored: true,
                image_urls: Vec::new(),
            });
            continue;
        }

        let bytes = chapter.content_html.as_bytes();
        match store.put(state.db(), bytes, "text/html").await {
            Ok((checksum, _key)) => {
                store
                    .reference(state.db(), &checksum, CHAPTER_OWNER_TYPE, &item.id)
                    .await
                    .map_err(transient)?;
                imports::upsert_import_chapter(
                    state.db(),
                    &row.id,
                    Some(&item.id),
                    &imports::ImportChapterInput {
                        source_chapter_key: chapter.source_chapter_key.clone(),
                        ordinal: i64::from(chapter.ordinal),
                        title: chapter.title.clone(),
                        state: "stored".to_owned(),
                        content_blob_checksum: Some(checksum.clone()),
                        note: None,
                    },
                )
                .await
                .map_err(transient)?;
                outcomes.push(StoredChapter {
                    ordinal: chapter.ordinal,
                    source_chapter_key: chapter.source_chapter_key.clone(),
                    title: chapter.title.clone(),
                    checksum: Some(checksum),
                    note: None,
                    stored: true,
                    image_urls: chapter.image_urls.clone(),
                });
            }
            Err(error) => {
                // One chapter that cannot be written does not throw away the
                // chapters that already were. It is recorded and the import
                // carries on, then fails at the end so the queue retries it.
                let note = error.to_string();
                imports::upsert_import_chapter(
                    state.db(),
                    &row.id,
                    Some(&item.id),
                    &imports::ImportChapterInput {
                        source_chapter_key: chapter.source_chapter_key.clone(),
                        ordinal: i64::from(chapter.ordinal),
                        title: chapter.title.clone(),
                        state: "failed".to_owned(),
                        content_blob_checksum: None,
                        note: Some(note.clone()),
                    },
                )
                .await
                .map_err(transient)?;
                outcomes.push(StoredChapter {
                    ordinal: chapter.ordinal,
                    source_chapter_key: chapter.source_chapter_key.clone(),
                    title: chapter.title.clone(),
                    checksum: None,
                    note: Some(note),
                    stored: false,
                    image_urls: chapter.image_urls.clone(),
                });
                any_failed = true;
            }
        }

        let done = i64::try_from(index + 1).unwrap_or(i64::MAX);
        let permille = PREVIEW_PERMILLE + (950 * done / total);
        progress(
            state,
            job,
            permille,
            Some(chapter.source_chapter_key.as_str()),
        )
        .await?;
    }

    if any_failed {
        let failed = outcomes.iter().filter(|outcome| !outcome.stored).count();
        return Err(HandlerError::Transient(format!(
            "{failed} of {} chapter(s) could not be stored; the rest are held, and a retry will only re-read what failed",
            outcomes.len()
        )));
    }

    Ok(outcomes)
}

/// Write the import's final state and report.
async fn finish(
    state: &AppState,
    row: &imports::ImportJob,
    import_job_id: &str,
    state_name: &str,
    report: &str,
) -> Result<(), HandlerError> {
    imports::set_import_state(state.db(), import_job_id, state_name, None, Some(report))
        .await
        .map_err(transient)?;
    if let Some(item_id) = &row.library_item_id {
        imports::touch_library_item_synced(state.db(), item_id)
            .await
            .map_err(transient)?;
    }
    Ok(())
}

/// Build the import report JSON, including media rescue stats (§32.7.9).
fn report_with_media(
    plan: &ImportPlan,
    stored: &[StoredChapter],
    dry_run: bool,
    media: &lorehaven_db::media_resilience::ImportMediaSummary,
) -> String {
    let base = report(plan, stored, dry_run);
    // Merge the media summary into the existing JSON object.
    let mut obj: serde_json::Value = serde_json::from_str(&base).unwrap_or_default();
    if let Some(inner) = obj.as_object_mut() {
        inner.insert(
            "media_rescue".to_owned(),
            serde_json::json!({
                "total_urls": media.total_urls,
                "already_held": media.already_held,
                "new_references": media.new_references,
                "unparseable": media.unparseable,
            }),
        );
    }
    obj.to_string()
}

/// §32.7.9: rescue every image URL from an import's chapters.
///
/// For each chapter's `image_urls`, deduplicate within the import and call
/// `upsert_media_reference_for_import`, which checks for an existing
/// availability link by URL and creates a new reference if none exists. The
/// summary counts URLs, deduplicated URLs, and new references created.
async fn rescue_import_media(
    db: &lorehaven_db::Database,
    item: &imports::LibraryItem,
    stored: &[StoredChapter],
) -> lorehaven_db::media_resilience::ImportMediaSummary {
    use lorehaven_db::media_resilience;

    let mut total_urls = 0;
    let mut already_held = 0;
    let mut new_references = 0;
    let mut unparseable = 0;
    let mut seen = std::collections::HashSet::new();

    for chapter in stored {
        for url in &chapter.image_urls {
            total_urls += 1;
            // Deduplicate within the import: a URL that appears in two chapters
            // of the same work creates one media reference, not two.
            if !seen.insert(url.clone()) {
                continue;
            }
            match media_resilience::upsert_media_reference_for_import(
                db,
                &item.id,
                Some(&chapter.source_chapter_key),
                url,
            )
            .await
            {
                Ok((created, _id)) => {
                    if created {
                        new_references += 1;
                    } else {
                        already_held += 1;
                    }
                }
                Err(_) => {
                    unparseable += 1;
                }
            }
        }
    }

    media_resilience::ImportMediaSummary {
        total_urls,
        already_held,
        new_references,
        unparseable,
    }
}

/// The report, as JSON: what the plan said, and what each chapter ended up as.
///
/// It is stored rather than recomputed so that "what did this import do?" has an
/// answer after the fact, including for the imports that did not finish.
fn report(plan: &ImportPlan, stored: &[StoredChapter], dry_run: bool) -> String {
    let (kind, changes) = match plan {
        ImportPlan::Create => ("create", Vec::new()),
        ImportPlan::NoChange => ("no_change", Vec::new()),
        ImportPlan::Update { changes } => (
            "update",
            changes
                .iter()
                .map(|change| match change {
                    ChapterChange::Added { chapter } => json!({
                        "kind": "added",
                        "source_chapter_key": chapter.source_chapter_key,
                        "ordinal": chapter.ordinal,
                        "title": chapter.title,
                    }),
                    ChapterChange::Removed { chapter } => json!({
                        "kind": "removed",
                        "source_chapter_key": chapter.source_chapter_key,
                        "ordinal": chapter.ordinal,
                        "title": chapter.title,
                    }),
                    ChapterChange::Reordered {
                        source_chapter_key,
                        from,
                        to,
                    } => json!({
                        "kind": "reordered",
                        "source_chapter_key": source_chapter_key,
                        "from": from,
                        "to": to,
                    }),
                    ChapterChange::Retitled {
                        source_chapter_key,
                        was,
                        now,
                    } => json!({
                        "kind": "retitled",
                        "source_chapter_key": source_chapter_key,
                        "was": was,
                        "now": now,
                    }),
                })
                .collect(),
        ),
    };

    json!({
        "dry_run": dry_run,
        "plan": kind,
        "changes": changes,
        "added": plan.added(),
        "removed": plan.removed(),
        "reordered": plan.reordered(),
        "retitled": plan.retitled(),
        "stored": stored.iter().filter(|chapter| chapter.stored).count(),
        "failed": stored.iter().filter(|chapter| !chapter.stored).count(),
        "chapters": stored
            .iter()
            .map(|chapter| {
                json!({
                    "ordinal": chapter.ordinal,
                    "source_chapter_key": chapter.source_chapter_key,
                    "title": chapter.title,
                    "stored": chapter.stored,
                    "checksum": chapter.checksum,
                    "note": chapter.note,
                })
            })
            .collect::<Vec<Value>>(),
    })
    .to_string()
}
