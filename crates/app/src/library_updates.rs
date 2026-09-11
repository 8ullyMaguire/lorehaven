//! The library update check (spec §14.1: "a list of what updated since they last
//! looked").
//!
//! # What it does, and what it deliberately does not
//!
//! For each of the reader's library items it re-reads the source's **metadata**
//! — the chapter list and the source's own "last changed" stamp — and writes
//! down what differs from the copy that was imported. It never re-fetches a
//! chapter body: the point is to tell the reader something changed, not to
//! change it, and an item the reader wants refreshed is a new import they ask
//! for.
//!
//! # It reads anonymously, and says so
//!
//! Items whose source needs a credential cannot be read this way. Rather than
//! fail the whole job for one such item, the check records that it could not
//! see the work and moves on — so the reader gets a truthful report about the
//! items that could be checked and an honest "not checked" for the rest. Using
//! the reader's saved credential here would be the better answer and is the
//! obvious next step; it is not done yet, and pretending otherwise in the
//! report would be worse than the gap.
//!
//! # Bounded on purpose
//!
//! One job checks at most [`CHECK_BATCH`] items, oldest first. A reader with a
//! thousand imports asking for a check should get an answer in bounded time and
//! a bounded number of requests to somebody else's server, and the oldest-first
//! order is what makes the next job continue where this one stopped.

use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use lorehaven_db::{imports, library};
use lorehaven_domain::library::update_check_retention_cutoff;
use lorehaven_scrapers::{SafeFetcher, SourceKey};

use crate::imports::policy_for;
use crate::state::AppState;
use crate::worker::HandlerError;

/// The payload key carrying the account whose library is checked.
const PAYLOAD_ACCOUNT_ID: &str = "account_id";

/// How many items one job checks.
pub const CHECK_BATCH: i64 = 50;

fn transient(error: impl std::fmt::Display) -> HandlerError {
    HandlerError::Transient(error.to_string())
}

fn fatal(message: impl Into<String>) -> HandlerError {
    HandlerError::Fatal(message.into())
}

/// What one item's check found.
#[derive(Debug, Clone)]
struct ItemReport {
    /// The differences, each a field with what it was and what it is now.
    changes: Vec<Value>,
    /// Why the check could not see the work, when it could not.
    unavailable: Option<String>,
}

impl ItemReport {
    /// An item that could not be read.
    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            changes: Vec::new(),
            unavailable: Some(reason.into()),
        }
    }
}

/// Run the check for one account.
///
/// # Errors
///
/// Transient for anything that a retry could fix, fatal for a payload this
/// build cannot use.
pub async fn run(state: &AppState, payload: &Value) -> Result<(), HandlerError> {
    let account = payload
        .get(PAYLOAD_ACCOUNT_ID)
        .and_then(Value::as_str)
        .ok_or_else(|| {
            fatal(format!(
                "an update-check payload must carry {PAYLOAD_ACCOUNT_ID}"
            ))
        })?;

    // Retention first, so a library that has been checked for years does not
    // keep its history for ever (spec §16's 90 days). A failure here is logged
    // and the check continues: the sweep is housekeeping, not the job.
    let now = OffsetDateTime::now_utc();
    let cutoff = update_check_retention_cutoff(now)
        .format(&Rfc3339)
        .unwrap_or_default();
    if let Err(error) = library::sweep_update_checks(state.db(), &cutoff).await {
        tracing::warn!(%error, "could not sweep expired update checks");
    }

    let items = library::all_library_items(state.db(), account, CHECK_BATCH)
        .await
        .map_err(transient)?;
    if items.is_empty() {
        return Ok(());
    }

    for item in &items {
        let report = check_one(state, item).await;
        let found = i64::try_from(report.changes.len()).unwrap_or(i64::MAX);
        let detail = json!({
            "title": item.title,
            "source_key": item.source_key,
            "source_url": item.source_url,
            "changes": report.changes,
            "unavailable": report.unavailable,
        });
        library::record_update_check(state.db(), account, &item.id, found, &detail.to_string())
            .await
            .map_err(transient)?;
    }

    Ok(())
}

/// Check one item against its source.
///
/// Every failure is turned into a report rather than propagated: one source
/// being down is a fact about that item, and it must not cost the reader the
/// results for the other forty-nine.
async fn check_one(state: &AppState, item: &imports::LibraryItem) -> ItemReport {
    let key = SourceKey::new(item.source_key.clone());
    let Ok(adapter) = state.registry().by_key(&key) else {
        return ItemReport::unavailable(format!(
            "this build has no adapter for `{}`",
            item.source_key
        ));
    };
    let Ok(url) = url::Url::parse(&item.source_url) else {
        return ItemReport::unavailable("the stored source URL is not a URL");
    };

    let policy = policy_for(adapter, state.config());
    let fetcher = SafeFetcher::new(adapter.hosts(), policy);
    let work = match adapter.preview(&fetcher, &url, None).await {
        Ok(work) => work,
        Err(error) => return ItemReport::unavailable(describe(&error)),
    };

    let mut changes = Vec::new();
    let was_chapters = item.chapter_count;
    let now_chapters = i64::try_from(work.chapters.len()).unwrap_or(i64::MAX);
    if now_chapters != was_chapters {
        changes.push(json!({
            "field": "chapter_count",
            "was": was_chapters,
            "now": now_chapters,
        }));
    }
    if let Some(now_updated) = work.updated_at {
        let now_text = now_updated.format(&Rfc3339).ok();
        // The source's own stamp, compared as text, because that is how it is
        // stored. A null stored stamp means the source never said when it
        // changed — which is a difference worth reporting once, not a match.
        if item.source_updated_at.as_deref() != now_text.as_deref() {
            changes.push(json!({
                "field": "source_updated_at",
                "was": item.source_updated_at,
                "now": now_text,
            }));
        }
    }
    if let Some(word_count) = work.word_count {
        if item.word_count != Some(word_count) {
            changes.push(json!({
                "field": "word_count",
                "was": item.word_count,
                "now": word_count,
            }));
        }
    }
    if work.title != item.title {
        changes.push(json!({
            "field": "title",
            "was": item.title,
            "now": work.title,
        }));
    }

    ItemReport {
        changes,
        unavailable: None,
    }
}

/// A short, reader-safe reason a source could not be read.
fn describe(error: &lorehaven_scrapers::SourceError) -> String {
    match error {
        lorehaven_scrapers::SourceError::Unsupported(message) => message.clone(),
        lorehaven_scrapers::SourceError::Blocked => {
            "the source refused this instance without a challenge solver".to_owned()
        }
        other => format!("the source could not be read ({other})"),
    }
}
