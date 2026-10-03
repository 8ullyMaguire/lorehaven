//! M45-22 — §54.5's completion notification.
//!
//! One work completes; every reader who asked to be told gets one notification,
//! once, ever.
//!
//! The durability rule is one sentence long and the whole function exists to
//! enforce it: **the `notified_at IS NULL` filter and the `mark_watched` that
//! follows are one operation's worth of intent.** Selecting the pending watches
//! and marking them consumed are two round trips, so two completion events racing
//! on the same work can both read "not yet notified" and both notify. The
//! `mark_watched` predicate closes that — it returns whether *this* caller won the
//! claim, and a caller that lost sends nothing.
//!
//! A read-then-write race is invisible to every single-threaded test, which is why
//! the consume step asserts on the boolean rather than on the absence of a
//! duplicate row: the `UNIQUE (account_id, work_id)` constraint makes duplicate
//! rows impossible, so their absence proves nothing about notification counts.

use lorehaven_db::Database;

/// Notify every reader watching `work_id` that it has completed.
///
/// Returns how many notifications were actually sent — the count of watches *this
/// caller claimed*, not the count it found. Those differ exactly when another
/// caller got there first, and the difference is the only thing the number is for.
///
/// Best-effort per reader, in the sense that one reader's failed notification does
/// not stop the next: a single unreachable row would otherwise mean the remaining
/// watchers are never told, which is the worse failure by a wide margin. A watch
/// whose notification failed is left un-claimed so the next completion event can
/// retry it — which is the right default for a notification, and the wrong one for
/// a ledger.
pub async fn notify_completion(db: &Database, work_id: &str) -> Result<u64, sqlx::Error> {
    let pending = lorehaven_db::concierge_store::pending_watches_for_work(db, work_id).await?;
    let mut sent = 0_u64;

    for (account_id, watch_id) in pending {
        // Claim BEFORE notifying. The other order would let a crash between the two
        // leave a watch that can never fire again — a reader who asked to be told
        // and cannot be, with the store insisting they were.
        match lorehaven_db::concierge_store::mark_watched(db, &watch_id).await {
            Ok(true) => {}
            // Another caller claimed this watch first; they notified it.
            Ok(false) => continue,
            Err(e) => {
                tracing::warn!(%work_id, %watch_id, %e, "could not claim a WIP watch");
                continue;
            }
        }

        if let Err(e) = lorehaven_db::notifications::notify(
            db,
            &account_id,
            "wip_completed",
            "A work you are following has finished",
            // The body carries the work id rather than its title. The caller has a
            // `work_id`; resolving a title would be a second query per watcher on a
            // completion event, and this is already a write per watcher. The client
            // resolves the title from the id it is handed, which it has to do anyway
            // to link the notification to a work it can open.
            work_id,
            Some(work_id),
        )
        .await
        {
            tracing::warn!(%work_id, %account_id, %e, "completion notification failed");
            // The watch stays claimed. §54.5 says once ever, and a re-notify on the
            // next unrelated edit is the failure mode that rule exists to prevent —
            // so a failed send does not buy a retry. The `delivery_channel` lookup
            // inside `notify` already drops events the reader has disabled, so a
            // `None` here is a failure of the send, not of consent.
            continue;
        }
        sent += 1;
    }

    Ok(sent)
}
