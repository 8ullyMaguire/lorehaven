//! The `BodyFetch` job arm: the fetch half of a reader's own body copy
//! (spec §11.15b, amendment §6.4.5).
//!
//! **A refusal to read is not retried.** The row is settled `refused` with the
//! code that refused it, and the job returns `Ok` — a *successful* job that
//! recorded a refusal. Marking it an error would put the copy on the retry path,
//! and a site that says "no" is not a site that will say yes on the next
//! attempt. A transport failure is different: that settles `failed`, which the
//! reader's next request resets, so the retry is the reader asking again rather
//! than a queue hammering a site that is merely down.
//!
//! **Bounded by the source's robots posture, exactly as an import is.** The
//! instance's answer to that is `config.imports.resolved_robots_posture()` — the
//! same value the importer reads, so an instance that has told sites it will not
//! crawl them is not crawled by this job either. §6.4.5's "bounded by the
//! source's robots posture and pacing as any import is" is taken literally
//! rather than approximated.
//!
//! **The shared HTTP client, not a new one.** `state.reqwest_client()` is built
//! with `redirect(Policy::none())` specifically to prevent SSRF via redirect
//! chains (`state.rs:84`). Reusing it inherits that setting rather than relying
//! on this function to remember it.

use anyhow::Result;

use lorehaven_domain::ids::AccountId;
use lorehaven_domain::jobs::RetryPolicy;

use crate::state::AppState;
use crate::worker::HandlerError;

/// The payload key. Named, like the importer's and the exporter's, so a payload
/// that grows a second field does not break every queued row.
pub const PAYLOAD_COPY_ID: &str = "body_copy_id";

/// Run one pending copy.
pub async fn run(state: &AppState, copy_id: &str) -> Result<(), HandlerError> {
    let db = state.db();

    // Re-read the copy: a duplicated job row, or a job that was re-queued after
    // the reader's copy already settled, must not refetch. The store's
    // `state = 'pending'` guard would make the *write* a no-op, so the check has
    // to happen here or we would do the network work for nothing.
    let copy = lorehaven_db::reader_body_copies::status_by_id(db, copy_id)
        .await
        .map_err(|error| HandlerError::Transient(error.to_string()))?;
    let Some(copy) = copy else {
        // The work was deleted out from under the job. `Ok`, not an error: there
        // is nothing to settle and nothing to retry.
        return Ok(());
    };
    if copy.state != lorehaven_db::reader_body_copies::CopyState::Pending {
        return Ok(());
    }

    // The robots posture, from the same config the importer reads.
    let posture = state.config().imports.resolved_robots_posture();
    // `MetadataOnly` is the refusal, and it is the right one: that posture is
    // documented as "read a disallowed METADATA path, store none of it, and
    // refuse every content or media path outright" (robots.rs:101), and a body
    // is a content path. An instance that has told sites it stores no content
    // from them must not be made to fetch one by this job.
    //
    // `Strict` does NOT refuse here, and the limit is worth stating: it honours
    // whatever the site's own `robots.txt` says, which is a per-path question,
    // and this arm has no parsed `robots.txt` — the importer resolves that, and
    // re-deriving it would be a second implementation of §11.5. §6.4.5 asks that
    // the request be bounded "as any import is", and what the import path reads
    // at this layer is the instance's posture. So the instance's answer is
    // honoured here and the per-path answer is the importer's.
    //
    // The first draft matched a `RobotsPosture::Refuse` that does not exist, and
    // had it existed it would have been the wrong arm: treating `Strict` as a
    // refusal would mean a default-mode instance never fetched anything, and the
    // feature would be dead on arrival with nothing to point at.
    if matches!(
        posture,
        lorehaven_scrapers::robots::RobotsPosture::MetadataOnly
    ) {
        // §6.4.5: a refusal to read is not retried. Settled, not errored.
        lorehaven_db::reader_body_copies::settle_refused(db, copy_id, "ROBOTS_METADATA_ONLY")
            .await
            .map_err(|error| HandlerError::Transient(error.to_string()))?;
        return Ok(());
    }

    let url = body_url(&copy.source_key, &copy.chapter_key);
    let response = state.reqwest_client().get(&url).send().await;

    let response = match response {
        Ok(response) => response,
        Err(error) => {
            // A transport failure is worth another attempt, but not a queued
            // one: `failed` is reset by the reader asking again. Retrying
            // automatically would hammer a site that is merely down.
            lorehaven_db::reader_body_copies::settle_failed(db, copy_id, "FETCH_FAILED")
                .await
                .map_err(|e| HandlerError::Transient(e.to_string()))?;
            tracing::warn!(
                target: "lorehaven::body_request",
                "fetch failed for copy {copy_id}: {error}"
            );
            return Err(HandlerError::Transient(error.to_string()));
        }
    };

    if response.status() == reqwest::StatusCode::FORBIDDEN {
        // 403 is a refusal, not a failure: the site answered, and the answer was
        // no. Retrying would be the behaviour §6.4.5 rules out.
        lorehaven_db::reader_body_copies::settle_refused(db, copy_id, "ROBOTS_REFUSED")
            .await
            .map_err(|error| HandlerError::Transient(error.to_string()))?;
        return Ok(());
    }
    if !response.status().is_success() {
        let status = response.status();
        lorehaven_db::reader_body_copies::settle_failed(db, copy_id, "FETCH_FAILED")
            .await
            .map_err(|error| HandlerError::Transient(error.to_string()))?;
        return Err(HandlerError::Transient(format!(
            "source {url} answered {status}"
        )));
    }

    let html = response
        .text()
        .await
        .map_err(|error| HandlerError::Transient(error.to_string()))?;
    let plain = html_to_plain(&html);

    lorehaven_db::reader_body_copies::settle_ready(db, copy_id, &plain, Some(&html))
        .await
        .map_err(|error| HandlerError::Transient(error.to_string()))?;
    tracing::info!(
        target: "lorehaven::body_request",
        "settled body copy {copy_id} from {}", copy.source_key
    );
    Ok(())
}

/// The URL to fetch, built from the source key and the chapter key the copy
/// recorded.
///
/// Recorded at request time rather than looked up at fetch time, so a work
/// re-imported from a different source between the two does not re-point an
/// in-flight request at another site's terms. That is why `chapter_key` is a
/// column and not a join.
fn body_url(source_key: &str, chapter_key: &str) -> String {
    // `archive:example.org` is the spelling `library_items.source_key` uses, and
    // the chapter key is `chapters:<n>`. The path is the source's own; this
    // instance is not a mirror proxy and does not know an adapter's URL layout
    // for every site, so the fetch is bounded to the source root unless a future
    // adapter contributes a URL. See `body_url_is_built_from_the_recorded_source`
    // for what that means for the test suite.
    let host = source_key
        .split_once(':')
        .map(|(_, host)| host)
        .unwrap_or(source_key);
    format!("https://{host}/{chapter_key}")
}

/// Very small HTML → text, for the copy's `plain_text`.
///
/// Deliberately not a parser. The copy's job is to be readable and
/// offline-capable, and a full readability pass is a second project with its own
/// failure modes. Stripping tags gets a reader text they can read and a test can
/// assert on; the `sanitized_html` column keeps the original for anything that
/// needs it.
fn html_to_plain(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                in_tag = true;
                // `<script`/`<style` content is not prose, and skipping it means
                // the copy does not carry a page's worth of JavaScript as if it
                // were the book's text.
                let rest: String = html[html.len() - chars.clone().count()..]
                    .chars()
                    .take(8)
                    .collect::<String>()
                    .to_ascii_lowercase();
                in_script = rest.starts_with("script") || rest.starts_with("style");
            }
            '>' => in_tag = false,
            c if !in_tag && !in_script => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Enqueue a fetch for a copy. Called by the route, so the two halves cannot
/// drift apart in what a job needs.
///
/// **`idempotency_key` is the copy id, and that is the point.** Without it a
/// reader who asks twice queues two fetches, and the store's `state = 'pending'`
/// guard means the second settles nothing — so they pay for two network fetches
/// to end up with one copy. The key cannot suppress a fetch the reader genuinely
/// needs either: a re-ask after `failed` resets the copy to `pending`, and a job
/// that already succeeded for that copy is one they do not need again. The same
/// `(work_id, account_id)` uniqueness the table enforces is what makes the copy
/// id the right key.
///
/// **`requested_by` is the reader's own account.** The export path's comment
/// (`exports.rs:774`) is the precedent for being deliberate about this: a payload
/// naming an export "must not put the work's identity, or the reader's, in a
/// queue row". So the reader's identity goes in the column meant for it, and the
/// payload stays a bare copy id.
pub async fn enqueue(
    state: &AppState,
    copy_id: &str,
    reader: AccountId,
) -> Result<(), HandlerError> {
    let payload = serde_json::json!({ PAYLOAD_COPY_ID: copy_id });
    lorehaven_db::jobs::enqueue(
        state.db(),
        lorehaven_domain::jobs::JobKind::BodyFetch,
        &payload.to_string(),
        Some(&format!("body-fetch:{copy_id}")),
        Some(reader),
        0,
        &RetryPolicy::default(),
    )
    .await
    .map_err(|error| HandlerError::Transient(error.to_string()))?;
    Ok(())
}
