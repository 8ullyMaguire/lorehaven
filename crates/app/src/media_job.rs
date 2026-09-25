//! The media fetch job (spec §32.7.2).
//!
//! This is the driver M32-07b and M32-07c left missing. Those milestones built
//! the pieces — [`crate::media_fetch::plan_fetch`] refuses a forbidden host,
//! [`crate::media_fetch::fingerprint_encoded`] decodes bytes into both hashes,
//! and `lorehaven_db::media_resilience::record_fingerprint` persists them — and
//! nothing called them in sequence, so the perceptual dedup search read a
//! column nothing ever wrote.
//!
//! `add_media_reference` has always answered 201 with a `pending` placeholder
//! and a comment promising "content hash computed async by the pipeline". This
//! is that pipeline. The add-reference door enqueues [`JobKind::MediaFetch`]
//! naming only the reference id; the worker claims it, fetches, decodes and
//! writes.
//!
//! The URL is author-supplied and the fetch is a network call, so it is a job
//! rather than work inside the request: a slow or dead mirror must not hold an
//! author's HTTP connection open, and a retry must not re-run the door.
//!
//! # Refusals are classified, not lumped together
//!
//! A 404 will still be a 404 tomorrow, so retrying it every minute for the life
//! of the instance is wrong. A 503 or a connection reset may clear, so giving up
//! on the first one loses a mirror that was briefly down. [`FetchOutcome`]
//! already makes that distinction and this module honours it: a permanent
//! failure is [`HandlerError::Fatal`] and the job stops; a retryable one is
//! [`HandlerError::Transient`].

use crate::media_fetch::{
    self, classify_with_length, plan_fetch_allowing, FetchOutcome, MediaFingerprint,
    MAX_MEDIA_BYTES,
};
use crate::state::AppState;
use crate::worker::HandlerError;
use lorehaven_db::media_resilience;
use lorehaven_db::Database;
use lorehaven_domain::jobs::{JobKind, RetryPolicy};
use serde_json::json;
use std::net::IpAddr;
use std::time::Duration;

/// How long the fetcher waits for a media body.
///
/// Bounded because a worker slot is held for the duration: a mirror that
/// accepts a connection and then dribbles one byte a minute would otherwise
/// occupy a worker indefinitely. The house `webhook_timeout_secs` is for
/// webhooks, not media, so this is its own constant rather than a reused one.
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// Queue a fetch for a media reference.
///
/// Called by the add-reference door. The payload names the reference and
/// nothing else: the URL is read from the reference's own availability links at
/// fetch time, so a job row never holds an author-supplied URL where a queue
/// reader can see it, and a corrected link is picked up by a retry.
pub async fn enqueue_media_fetch(
    db: &Database,
    reference_id: &str,
) -> anyhow::Result<lorehaven_domain::JobId> {
    let job_id = lorehaven_db::jobs::enqueue(
        db,
        JobKind::MediaFetch,
        &json!({ "reference_id": reference_id }).to_string(),
        None,
        None,
        0,
        &RetryPolicy::default(),
    )
    .await?;
    Ok(job_id)
}

/// Fetch a media reference's bytes and store both hashes.
///
/// `trusted_address` exists only so a test can drive this handler against a
/// loopback server. Loopback is refused unconditionally by
/// [`plan_fetch`] — correctly, since on a self-hosted instance the thing being
/// protected is usually on the same machine — so the test needs a way to say
/// "this one loopback address is the subject under test" without relaxing the
/// guard for real traffic. When it is `None` the address is checked normally,
/// which is every production call.
pub async fn handle_media_fetch(
    state: &AppState,
    reference_id: &str,
    trusted_address: Option<IpAddr>,
) -> Result<(), HandlerError> {
    // 1. The reference must exist. A job for a row that has been deleted is
    //    permanent: retrying cannot bring it back.
    if media_resilience::find_media_reference_by_id(state.db(), reference_id)
        .await
        .map_err(transient)?
        .is_none()
    {
        return Err(HandlerError::Fatal(format!(
            "media reference {reference_id} not found"
        )));
    }

    // 2. Find the URL. The first link by priority is the one to try; a
    //    reference with no link has nothing to fetch, which is permanent rather
    //    than something to retry.
    let links = media_resilience::find_availability_links_for_reference(state.db(), reference_id)
        .await
        .map_err(transient)?;
    let Some(link) = links.first() else {
        return Err(HandlerError::Fatal(format!(
            "media reference {reference_id} has no availability link to fetch"
        )));
    };

    // 3. Refuse the URL before connecting.
    //
    // `trusted_address` is injected at the *guard*, not used to skip it. The
    // test needs loopback to be reachable, and loopback is refused
    // unconditionally by `plan_fetch` — correctly, since on a self-hosted
    // instance the thing being protected is usually on the same machine. Bypassing
    // the check with an `if` would leave a second, untested path through the
    // most security-sensitive function in the chain; instead the guard is given
    // a one-address allowlist that only the test wrapper sets. A production call
    // passes `None` and gets the full check.
    let allow: &[IpAddr] = trusted_address.as_slice();
    let url = reqwest::Url::parse(&link.url)
        .map_err(|error| HandlerError::Fatal(format!("link url is not a url: {error}")))?;
    let plan = plan_fetch_allowing(&url, allow, FETCH_TIMEOUT).map_err(HandlerError::Fatal)?;

    // Resolve the host and refuse if *any* answer is private. A host with one
    // public and one private address is a valid SSRF vector if only the first is
    // checked. The allowlist applies here too: a test's loopback host would
    // otherwise be refused by the resolver, which is the same check again.
    if allow.is_empty() {
        let resolved = lorehaven_scrapers::safety::resolve_public(&plan.host, FETCH_TIMEOUT).await;
        let resolved = match resolved {
            Ok(addrs) => addrs,
            Err(error) => return Err(HandlerError::Fatal(error.to_string())),
        };
        for addr in &resolved {
            if lorehaven_scrapers::safety::is_forbidden_ip(addr.ip()) {
                return Err(HandlerError::Fatal(format!(
                    "{} resolves to a non-public address",
                    plan.host
                )));
            }
        }
    }

    // 4. Fetch. The instance client already has redirects disabled, so a 302 to
    //    a private address cannot be followed — a redirect chain is the usual
    //    way past a scheme-and-host check.
    let response = state
        .reqwest_client()
        .get(plan.url.clone())
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .map_err(|error| {
            // A connect or read failure may be a mirror being down, so this is
            // retryable. A timeout certainly is.
            HandlerError::Transient(format!("fetching {} failed: {error}", plan.host))
        })?;

    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let declared = response.content_length();

    // 5. Classify before reading the body, so an oversized declared length is
    //    refused without the bytes being pulled over the wire.
    if let Err(outcome) = classify_with_length(status, &content_type, declared) {
        return Err(classify_failure(outcome));
    }

    // 6. Read the body, bounded even when the server declared no length.
    let bytes = read_bounded(response).await?;

    // 7. Fingerprint and store. A body that will not decode is still a
    //    successful fetch: the exact content hash of those bytes is a true fact
    //    worth keeping, and the perceptual hash is honestly absent rather than
    //    fabricated. `fingerprint_encoded` returning `None` is the undecodable
    //    case, and `without_perceptual_hash` is what carries the exact hash
    //    forward with the perceptual one left NULL.
    let fingerprint = fingerprint_encoded(&bytes)
        .unwrap_or_else(|| MediaFingerprint::without_perceptual_hash(&bytes));
    media_resilience::record_fingerprint(state.db(), reference_id, &(&fingerprint).into())
        .await
        .map_err(transient)?;
    dedup_after_fetch(state, reference_id, &fingerprint).await;
    Ok(())
}

/// The §32.7.2 dedup decision, made once the bytes are known.
///
/// Two branches, and the order matters. An exact `content_hash` match is a fact,
/// not a guess, so it attaches without asking anyone: the spec says "zero new
/// storage cost" and that a single popular faceclaim image has *one*
/// MediaReference. A perceptual match is not a fact -- pHash agrees across
/// re-encodes and disagrees across different images that happen to share
/// structure -- so it is written as a proposal for a curator and nothing is
/// merged.
///
/// Best-effort by design. A dedup failure must not fail the fetch: the bytes
/// are already mirrored and the reference already has its hashes, and losing
/// that because a proposal could not be written would be a far worse outcome
/// than a duplicate that a curator merges later. So this logs and continues
/// rather than propagating.
async fn dedup_after_fetch(state: &AppState, reference_id: &str, fingerprint: &MediaFingerprint) {
    let db = state.db();
    let exact = fingerprint.content_hash.clone();

    // The exact-match branch: attach this reference's link to the reference that
    // already holds these bytes. Skipped when the match is the reference itself,
    // which is the case on a re-fetch.
    match media_resilience::find_media_reference_by_content_hash(db, &exact)
        .await
        .map_err(transient)
    {
        Ok(Some(existing)) if existing.id != reference_id => {
            if let Err(e) =
                media_resilience::attach_reference_to_existing(db, &existing.id, reference_id).await
            {
                tracing::warn!(
                    reference_id,
                    existing_id = %existing.id,
                    error = %e,
                    "exact media match found but the link could not be attached"
                );
            }
            return;
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(reference_id, error = %e, "exact media match lookup failed"),
    }

    // The perceptual branch. No perceptual hash means nothing to compare, and an
    // undecodable body legitimately has none.
    let Some(perceptual) = fingerprint.perceptual_hash.as_deref() else {
        return;
    };
    let config = state.config();
    if !config.media_resilience.enabled {
        return;
    }
    let threshold = config.media_resilience.perceptual_match_threshold;
    let matches =
        match media_resilience::find_by_perceptual_hash(db, perceptual, threshold as i32).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(reference_id, error = %e, "perceptual media match lookup failed");
                return;
            }
        };
    for candidate in matches {
        if candidate.id == reference_id {
            continue;
        }
        let Some(stored) = candidate.perceptual_hash.as_deref() else {
            continue;
        };
        let Some(distance) =
            lorehaven_domain::media_resilience::hamming_distance(perceptual, stored)
        else {
            continue;
        };
        // `find_by_perceptual_hash` already filtered on distance, so this is a
        // re-assertion rather than a new check -- and `hamming_distance` is what
        // the confidence is computed from, so the proposal records the number
        // the curator will see.
        if let Err(e) = media_resilience::record_match_proposal(
            db,
            reference_id,
            &candidate.id,
            &exact,
            Some(perceptual),
            distance,
        )
        .await
        {
            tracing::warn!(
                reference_id,
                existing_id = %candidate.id,
                error = %e,
                "perceptual media match could not be proposed"
            );
        }
    }
}

/// Read a response body, refusing anything over the limit even when the server
/// declared no `Content-Length`.
///
/// `response.bytes()` trusts the connection, so a server that sends no length
/// can send as much as it likes. The chunks are counted as they arrive and the
/// read stops at the limit.
async fn read_bounded(mut response: reqwest::Response) -> Result<Vec<u8>, HandlerError> {
    let limit = MAX_MEDIA_BYTES as usize;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| HandlerError::Transient(format!("reading body failed: {error}")))?
    {
        if body.len() + chunk.len() > limit {
            return Err(HandlerError::Fatal(format!(
                "the body exceeds the {limit}-byte media limit"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Turn a classification into the right kind of job failure.
fn classify_failure(outcome: FetchOutcome) -> HandlerError {
    match outcome {
        FetchOutcome::Transient { status } => {
            HandlerError::Transient(format!("the mirror answered {status}; retrying"))
        }
        FetchOutcome::Gone { status } => {
            HandlerError::Fatal(format!("the mirror answered {status}; not retrying"))
        }
        FetchOutcome::TooLarge { limit } => HandlerError::Fatal(format!(
            "the body is larger than the {limit}-byte media limit"
        )),
        FetchOutcome::NotMedia { content_type } => HandlerError::Fatal(format!(
            "the link served {content_type:?}, which is not an image"
        )),
        FetchOutcome::Media => {
            // `classify_with_length` returns `Media` as its success value and
            // the caller only reaches this function on `Err`, so this arm is
            // unreachable in practice. Treated as fatal rather than transient
            // so a future caller cannot turn it into a retry loop by accident.
            HandlerError::Fatal("the body was accepted but the fetch still failed".to_owned())
        }
    }
}

/// Map a handler error onto the error type the worker's callers see, for tests
/// and for the `doctor` probe.
#[must_use]
pub fn error_message(error: &HandlerError) -> String {
    error.message()
}

fn transient(error: impl std::fmt::Display) -> HandlerError {
    HandlerError::Transient(error.to_string())
}

/// Drive the handler with `trusted_address` set.
///
/// Public rather than `#[cfg(test)]` because the end-to-end test lives in
/// `tests/`, which links the library as a dependency and cannot see the
/// library's own `#[cfg(test)]` items. It is a thin wrapper that does nothing
/// the handler cannot already do — `handle_media_fetch` takes the same
/// argument — so there is no capability here that a caller could not have.
pub async fn handle_media_fetch_for_test(
    state: &AppState,
    reference_id: &str,
    trusted: std::net::IpAddr,
) -> Result<(), crate::worker::HandlerError> {
    handle_media_fetch(state, reference_id, Some(trusted)).await
}

/// Re-exported so the shared PNG builders in `media_fetch` tests and here
/// cannot drift.
pub use media_fetch::fingerprint_encoded;
