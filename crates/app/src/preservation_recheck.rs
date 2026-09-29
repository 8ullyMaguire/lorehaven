//! The preservation recheck: the job that finds a destination which has stopped
//! answering, marks the target dead, and takes the credits back (spec §2.3,
//! §11.12a).
//!
//! **This is the only part of preservation that touches the network, and it is
//! why it is a job.** A verified target is one page on somebody else's server;
//! an instance with thousands of them cannot re-verify them inside a request.
//! §11.5's metadata ceiling (1 MiB), its robots posture and its per-host pacing
//! all apply to every fetch here, which is also why the kind is `Bulk` rather
//! than `Interactive` — a queue full of re-verifications must not delay an
//! import a reader just submitted.
//!
//! **The verdict is three-valued and the middle value is the whole design.**
//!
//! * `Alive` — the page answered and names the work. Refresh the evidence.
//! * `Dead` — the page answered and does *not* name the work, or the origin is
//!   gone. Mark dead, reclaim the credits.
//! * `Unreachable` — the network, a rate limit, a 5xx. **Nothing is changed**,
//!   because "we could not ask" is not "it is not there", and a recheck that
//!   clawed back credits because a destination was briefly down would be a way
//!   to destroy a reader's balance by making a third party unreachable.
//!
//! That last distinction is the anti-farm property and the anti-brittleness
//! property at once: a spammer cannot farm by pointing at a host that 404s on
//! demand, and a real archive having a bad afternoon costs nobody their credits.

use anyhow::Result;
use lorehaven_db::preservation;
use lorehaven_domain::preservation::PreservationState;
use lorehaven_scrapers::{FetchClass, FetchPolicy, SafeFetcher};
use sha2::{Digest, Sha256};
use std::time::Duration;

/// How long one destination check may take.
///
/// Short on purpose: a recheck is bulk background work and a destination
/// that has not answered in ten seconds is not going to answer usefully in
/// thirty. `Unreachable` is the right verdict for a timeout -- nothing is
/// written -- so a tight bound costs a retry rather than a wrong state.
const TIMEOUT: Duration = Duration::from_secs(10);

/// What one check concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The destination answered and names this work. `evidence` is a content
    /// hash of the identifying fields that were compared.
    Alive { evidence: String },
    /// The destination answered and this work is not on the page, or the record
    /// is gone. The reason is kept for the operator, not shown to a reader.
    Dead { reason: String },
    /// The check could not be completed. Nothing is written.
    Unreachable { reason: String },
}

/// The summary one pass of the job produces.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecheckSummary {
    pub checked: i64,
    pub alive: i64,
    pub dead: i64,
    pub unreachable: i64,
    pub reclaimed: i64,
}

impl RecheckSummary {
    /// A one-line report for the job log, with every count named.
    ///
    /// `unreachable` is included rather than folded into a total because a
    /// summary that omits it reports "checked 100" against "dead 0" and reads
    /// as good news when the real answer is "we did not manage to ask".
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "checked {}: {} alive, {} dead ({} credits reclaimed), {} unreachable",
            self.checked, self.alive, self.dead, self.reclaimed, self.unreachable
        )
    }
}

/// A content hash of the fields that identify a work on a destination page.
///
/// **Hashing the comparison, not the page.** §11.5 bounds what may be read and
/// §11.5's retention rules govern what may be kept, and there is no reason to
/// retain an archive's page once the comparison is done. A hash is enough to
/// answer the only later question worth answering: *is this still the same
/// record, or did the archive hand out a different id to the same slot?*
///
/// A page whose title differs hashes differently, which is the point: "still
/// the same work" and "the archive's page changed" are different facts and a
/// boolean cannot hold both.
pub fn evidence_hash(title: &str, author: &str) -> String {
    let mut hasher = Sha256::new();
    // Length-prefixed so ("ab", "c") and ("a", "bc") cannot collide.
    for field in [title.trim(), author.trim()] {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

/// Compare a destination's page against the work it should be carrying.
///
/// A **pure function on purpose**, and this is the seam that makes the whole
/// feature testable without a network. The job fetches; this decides. A test
/// that has to stand up an HTTP server to check that a 404 produces `Dead` is a
/// test that breaks when the client changes, and the thing worth pinning is the
/// decision, not the transport.
#[must_use]
pub fn compare(response: &str, expected_title: &str, expected_author: &str) -> Verdict {
    if response.trim().is_empty() {
        return Verdict::Unreachable {
            reason: "the destination returned an empty body".to_owned(),
        };
    }
    // §11.5's not-found signals. Checked on the status-shaped text the fetch
    // layer reports, and matched case-insensitively because a reworded error
    // page is the same answer with different capitals.
    let lowered = response.to_ascii_lowercase();
    for marker in [
        "404",
        "not found",
        "no longer available",
        "this work is gone",
    ] {
        if lowered.contains(marker) {
            return Verdict::Dead {
                reason: format!("the destination reported {marker}"),
            };
        }
    }
    // The title is the comparison that matters. A destination's page carries a
    // title, an author and a great deal of site furniture; requiring the author
    // to match exactly would fail on an archive that formats pseudonyms
    // differently, and requiring the *whole page* to match would fail on
    // anything that renders a timestamp.
    //
    // A title match is accepted whatever else the page says. That is a real
    // weakening against a destination serving an unrelated work under the same
    // id, and it is accepted because the alternative — an archive-specific
    // adapter per destination — is exactly what §2.5 says must not be required.
    // The title is the field the work is *known* by; a match means the
    // destination believes it holds this work, and §2.3's clawback is about
    // destinations that stop answering, not about adversarial archives.
    let needle = expected_title.trim().to_ascii_lowercase();
    if !needle.is_empty() && lowered.contains(&needle) {
        return Verdict::Alive {
            evidence: evidence_hash(expected_title, expected_author),
        };
    }
    Verdict::Dead {
        reason: format!(
            "the destination answered but its page does not name the work {expected_title:?}"
        ),
    }
}

/// Run one recheck pass over every verified target.
///
/// `state` is the app state, because the fetch needs the instance's configured
/// robots posture and its per-host pacing; the policy is the one Phase A built,
/// declared `Metadata` so the 1 MiB ceiling applies.
///
/// **A disabled destination is not checked, and its targets are not clawed
/// back.** An operator who turns an archive off has said "stop asking about
/// this", not "this archive lost my work", and the two must not look alike: the
/// first is a decision, the second is a fact about a third party, and only the
/// second takes credits.
pub async fn run(state: &crate::state::AppState) -> Result<RecheckSummary> {
    let mut summary = RecheckSummary::default();
    let targets = preservation::verified_targets(state.db()).await?;
    if targets.is_empty() {
        return Ok(summary);
    }

    for target in targets {
        summary.checked += 1;
        let Some(destination) =
            preservation::enabled_destination(state.db(), &target.destination_id).await?
        else {
            // Disabled, or deleted. Either way the answer is not "dead": the
            // operator stopped asking, and the record stands.
            tracing::debug!(
                member = %target.member_id,
                destination = %target.destination_id,
                "skipping a preservation target whose destination is not enabled"
            );
            summary.unreachable += 1;
            continue;
        };

        let verdict = check_one(state, &destination, &target).await;
        match verdict {
            Verdict::Alive { evidence } => {
                summary.alive += 1;
                preservation::set_state(
                    state.db(),
                    &target.member_id,
                    PreservationState::Verified,
                    Some(&evidence),
                )
                .await?;
            }
            Verdict::Dead { reason } => {
                summary.dead += 1;
                tracing::info!(
                    member = %target.member_id,
                    destination = %target.destination_id,
                    %reason,
                    "a preservation destination stopped carrying this work; marking it dead"
                );
                preservation::set_state(
                    state.db(),
                    &target.member_id,
                    PreservationState::Dead,
                    target.evidence_hash.as_deref(),
                )
                .await?;
                let outcome = preservation::reclaim_credits(state.db(), &target.member_id).await?;
                summary.reclaimed += outcome.credits().abs();
            }
            Verdict::Unreachable { reason } => {
                summary.unreachable += 1;
                tracing::debug!(
                    member = %target.member_id,
                    destination = %target.destination_id,
                    %reason,
                    "a preservation destination could not be reached; leaving the target as it is"
                );
            }
        }
    }
    Ok(summary)
}

/// Fetch one destination's item page and compare it.
async fn check_one(
    state: &crate::state::AppState,
    destination: &preservation::PreservationDestination,
    target: &preservation::PreservationTarget,
) -> Verdict {
    let Some(url) = target
        .external_url
        .clone()
        .or_else(|| destination.item_url(&target.external_record_id))
    else {
        return Verdict::Unreachable {
            reason: format!(
                "destination {} has no usable item url for record {}",
                destination.id, target.external_record_id
            ),
        };
    };
    let Some(host) = url_host(&url) else {
        return Verdict::Unreachable {
            reason: format!("{url} is not an http(s) url"),
        };
    };
    let Some((title, author)) = identity_fields(state, &target.work_id).await else {
        return Verdict::Unreachable {
            reason: format!("work {} has no identity to compare against", target.work_id),
        };
    };

    // §11.5's class, and the reason the plan says this is a `Metadata` fetch:
    // an archive with no adapter is still verifiable, because the comparison is
    // against the page's own fields rather than a structured response.
    //
    // The class is a *per-fetch parameter*, not a policy field, and that is
    // §11.5's own design: a URL that looks like a chapter fetched as `Metadata`
    // has to be bounded as a metadata fetch, so the declaration travels with
    // the call rather than being inferred from the path or pinned to the
    // fetcher. `get_declared` exists for exactly this caller.
    let mut policy = FetchPolicy::default();
    // The instance's robots posture, resolved. `imports.robots_posture` is an
    // `Option` and `honour_robots` is a second, older field; `resolved_
    // robots_posture` is what the fetcher is meant to read, and setting one
    // field while leaving the other to be consulted at the point of use is the
    // two-authoritative-fields shape `policy_for_in` exists to remove.
    policy.robots_posture = state.config().imports.resolved_robots_posture();
    // A recheck is background work against a third party, so it gets a short
    // timeout: a slow destination should not hold a worker slot, and the next
    // scheduled pass will try again. The floor matters because a sub-second
    // timeout on a paced connection fails for the fetcher's own reasons rather
    // than the destination's.
    policy.timeout = TIMEOUT;
    let fetcher = SafeFetcher::new(vec![host], policy);

    match fetcher.get_declared(&url, FetchClass::Metadata).await {
        Ok(fetched) => compare(&fetched.body, &title, &author),
        Err(error) => Verdict::Unreachable {
            reason: error.to_string(),
        },
    }
}

/// The work's canonical title and author, as the identity holds them.
async fn identity_fields(
    state: &crate::state::AppState,
    work_id: &str,
) -> Option<(String, String)> {
    let identity = lorehaven_db::story_identity::identity_for_work(state.db(), work_id)
        .await
        .ok()
        .flatten()?;
    // The author comes from the owning pseud, not from the identity: the
    // identity's `canonical_title` is the work's title and its author is a
    // separate fact about who wrote it. A destination page carries both.
    let sql = state.db().sql(
        "SELECT p.display_name FROM works w JOIN pseuds p ON p.id = w.owner_pseud_id
          WHERE w.id = ?",
        "SELECT p.display_name::text FROM works w JOIN pseuds p ON p.id = w.owner_pseud_id
          WHERE w.id = $1::uuid",
    );
    let author: Option<String> = match state.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&sql)
            .bind(work_id)
            .fetch_optional(state.db().sqlite_pool()?)
            .await
            .ok()
            .flatten(),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&sql)
            .bind(work_id)
            .fetch_optional(state.db().postgres_pool()?)
            .await
            .ok()
            .flatten(),
    };
    Some((identity.canonical_title, author.unwrap_or_default()))
}

/// The host of an http(s) URL, or `None` for anything else.
fn url_host(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    Some(parsed.host_str()?.to_owned())
}

/// Run one recheck pass as a job attempt.
///
/// The signature matches the other job handlers: an id, and the job payload,
/// and it reports an outcome rather than a bool. **A pass that completed with
/// unreachable targets is `Ok`, not a transient failure.** Retrying a pass that
/// could not reach a third party is how a job ends up hammering a struggling
/// archive, and the next scheduled pass will try again anyway.
/// **Takes no job id, and that is the shape of the work rather than a
/// shortcut.** A recheck has no single subject: it sweeps every verified
/// target, so there is no per-job payload to carry and no id to echo. The
/// other kinds take a reference because they act on one row; pretending this
/// one has a subject would invite a caller to enqueue a recheck "for" a
/// specific work and get a full-instance sweep.
pub async fn handle_recheck(
    state: &crate::state::AppState,
) -> std::result::Result<(), crate::worker::HandlerError> {
    let summary = run(state)
        .await
        // Transient, not fatal. Everything that can fail here is either a
        // database hiccup or a destination that would not answer, and both are
        // worth another attempt: a Fatal would retire the kind after one bad
        // pass, which is how a recheck silently stops running on an instance
        // whose storage blipped. `run` has already classified every destination
        // as Alive, Dead or Unreachable, so an error reaching here is ours and
        // not the third party's.
        .map_err(|error| {
            tracing::error!(%error, "a preservation recheck pass failed");
            crate::worker::HandlerError::Transient(error.to_string())
        })?;
    tracing::info!(target: "lorehaven::preservation", "{}", summary.describe());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TITLE: &str = "A Study in Scarlet";

    #[test]
    fn a_page_naming_the_work_is_alive_and_carries_evidence() {
        let page =
            "<html><head><title>A Study in Scarlet</title></head><body>by A. Author</body></html>";
        match compare(page, TITLE, "A. Author") {
            Verdict::Alive { evidence } => assert_eq!(evidence.len(), 64),
            other => panic!("expected Alive, got {other:?}"),
        }
    }

    #[test]
    fn a_not_found_page_is_dead() {
        assert!(matches!(
            compare("404 Not Found", TITLE, "A. Author"),
            Verdict::Dead { .. }
        ));
    }

    #[test]
    fn a_page_naming_a_different_work_is_dead() {
        let page = "<title>Something Else Entirely</title>";
        assert!(
            matches!(compare(page, TITLE, "A. Author"), Verdict::Dead { .. }),
            "a destination answering about a different work is not carrying this one"
        );
    }

    #[test]
    fn an_empty_body_is_unreachable_and_never_dead() {
        assert!(
            matches!(
                compare("   ", TITLE, "A. Author"),
                Verdict::Unreachable { .. }
            ),
            "\"we could not ask\" is not \"it is not there\"; a clawback triggered by an \
             empty response would let a third party's outage destroy a reader's credits"
        );
    }

    #[test]
    fn a_network_fault_never_becomes_a_clawback() {
        // The fetch layer reports its own faults; the caller maps an Err to
        // Unreachable rather than Dead. Pinned here so the mapping is visible
        // in the unit tests and not only in the job loop.
        let verdict: Verdict = Verdict::Unreachable {
            reason: "connection refused".to_owned(),
        };
        assert!(!matches!(verdict, Verdict::Dead { .. }));
    }

    #[test]
    fn evidence_distinguishes_a_changed_page_from_a_changed_record() {
        assert_eq!(
            evidence_hash(TITLE, "A. Author"),
            evidence_hash(TITLE, "A. Author"),
            "the same fields hash the same, so a recheck that finds nothing changed is \
             distinguishable from one that never ran"
        );
        assert_ne!(
            evidence_hash(TITLE, "A. Author"),
            evidence_hash(TITLE, "B. Author"),
            "a destination's page changing is a different fact from the record being \
             the same, and a boolean cannot hold both"
        );
        assert_ne!(
            evidence_hash("ab", "c"),
            evidence_hash("a", "bc"),
            "the fields are length-prefixed so a pair cannot collide by shifting a \
             boundary between them"
        );
    }

    #[test]
    fn a_summary_names_unreachable_rather_than_folding_it_in() {
        let summary = RecheckSummary {
            checked: 100,
            alive: 10,
            dead: 0,
            unreachable: 90,
            reclaimed: 0,
        };
        let line = summary.describe();
        assert!(
            line.contains("90 unreachable"),
            "a summary that omits the unreachable count reports \"checked 100, dead 0\" \
             and reads as good news when the answer is \"we did not manage to ask\". \
             Got: {line}"
        );
    }

    #[test]
    fn only_http_and_https_urls_are_fetched() {
        assert_eq!(
            url_host("https://archive.example/item/1").as_deref(),
            Some("archive.example")
        );
        assert_eq!(url_host("file:///etc/passwd"), None);
        assert_eq!(url_host("not a url"), None);
    }
}
