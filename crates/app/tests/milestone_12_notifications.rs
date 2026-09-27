//! M12 — Notifications inbox repository (`crates/db/src/notifications.rs`,
//! spec §5.5 / §46.4 / §46.7.1).
//!
//! Seven `pub async fn` with no test touching them. The module is the backing
//! store for the reader-facing inbox (GET /notifications, POST
//! /notifications/read-all, POST /notifications/{id}/read) and has five live
//! writers in `routes/` and `community.rs`, so every one of these is on a
//! shipped path.
//!
//! Two things make this module worth its own suite rather than an extension of
//! an existing one:
//!
//! 1. **Phantom ids.** `notify` mints a UUID and returns it *before* it knows
//!    whether the row is written. When the account has disabled the event,
//!    `resolve_notification_channel` returns `None` and `notify` returns
//!    `Ok(id)` having inserted nothing. All five current callers discard the
//!    return with `let _ =`, so nothing breaks today -- but the signature
//!    promises an id that resolves to no row, which is exactly the contract
//!    `roles::pin_work` violated (known-gaps M18-P42-D03). Pinned here so a
//!    future caller that trusts the id trips a test instead of production.
//!
//! 2. **The filtered/unfiltered split.** `list` ignores content filters and
//!    `list_filtered` applies them; `unread_count` ignores them and
//!    `unread_count_filtered` applies them. The module's own doc comments make
//!    a strong safety claim about the filtered pair -- a filtered work must not
//!    leak through the badge count "through a one-digit door". That claim is
//!    load-bearing for §46.7.1 and it was untested.

use std::path::PathBuf;

use lorehaven_db::notifications::{
    list, list_filtered, mark_all_read, mark_read, notify, unread_count, unread_count_filtered,
};
use lorehaven_db::search::content_filter_sql::FilterRule;
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m12-notif-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Which dialect a column's `WHERE` comparison needs.
///
/// PostgreSQL types `accounts.id`, `notifications.account_id` and
/// `notifications.work_id` as `UUID` while `notifications.id` stays `TEXT`, so
/// a bound parameter has to be cast for one and not the other.
#[derive(Clone, Copy)]
enum ColKind {
    Uuid,
    Text,
}

/// Convenience wrapper over [`TestDb`] holding the scratch dir alive.
///
/// The repository layer is exercised directly; accounts and works are seeded as
/// raw rows because the point of the suite is this module's own SQL, and going
/// through `accounts::create` would drag in trust-level and pseud-handle side
/// effects that have nothing to do with it.
struct Harness {
    tdb: TestDb,
    #[allow(dead_code)]
    dir: PathBuf,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        Self { tdb, dir }
    }

    fn db(&self) -> &lorehaven_db::Database {
        self.tdb.db()
    }

    /// Run a statement on whichever backend is live.
    async fn exec(&self, query: &str) {
        let q = self.tdb.sql(query);
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => {
                sqlx::query(&q)
                    .execute(self.db().sqlite_pool().expect("sqlite"))
                    .await
                    .expect("exec");
            }
            lorehaven_db::Backend::Postgres => {
                sqlx::query(&q)
                    .execute(self.db().postgres_pool().expect("pg"))
                    .await
                    .expect("exec");
            }
        }
    }

    /// A `bool` literal in the running backend's dialect.
    ///
    /// SQLite has no boolean type, so `TRUE` there is the keyword TRUE and
    /// stores integer 1; PostgreSQL stores a real `BOOLEAN`. `notification_routes.enabled`
    /// is `BOOLEAN` on PG, and `settings::resolve_notification_channel` decodes
    /// it as `bool` on both, so the literal has to be spelled per backend.
    fn bool_lit(&self, v: bool) -> &'static str {
        match self.db().backend() {
            lorehaven_db::Backend::Postgres if v => "TRUE",
            lorehaven_db::Backend::Postgres => "FALSE",
            _ if v => "1",
            _ => "0",
        }
    }

    /// A `uuid` comparison, as a string both backends accept.
    fn uuid_eq(&self, col: &str, value: &str) -> String {
        match self.db().backend() {
            lorehaven_db::Backend::Postgres => format!("{col} = '{value}'::uuid"),
            lorehaven_db::Backend::Sqlite => format!("{col} = '{value}'"),
        }
    }

    /// A `text` comparison.
    ///
    /// `notifications.id` is `TEXT PRIMARY KEY` on *both* backends, unlike
    /// `account_id` / `work_id`, which are `UUID` on PostgreSQL. Casting it to
    /// uuid is a `42883 operator does not exist: text = uuid` on PG, so the
    /// two shapes need separate helpers.
    fn text_eq(&self, col: &str, value: &str) -> String {
        format!("{col} = '{value}'")
    }

    async fn account(&self, tag: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO accounts (id, email, created_at, updated_at) \
             VALUES ('{id}', 'notif-{tag}-{id}@example.test', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        id
    }

    /// A work owned by a fresh pseud, tagged with the taxonomy node named by
    /// `node`. Tagging is a `work_tags` row plus a `taxonomy_nodes` row, because
    /// that is what `content_filter_sql::build_for` correlates on.
    async fn work(&self, node: Option<&str>) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let acct = self.account("w").await;
        let owner = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
             VALUES ('{owner}', '{acct}', 'w{owner}', 'W', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at) \
             VALUES ('{id}', '{owner}', 'A Work', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        if let Some(node) = node {
            self.tag(&id, "tag", node).await;
        }
        id
    }

    /// Attach a taxonomy node to a work, creating the node if needed.
    ///
    /// `INSERT OR IGNORE` is SQLite-only syntax -- PostgreSQL parses it as a
    /// syntax error at the `OR`, so the upsert has to be spelled per dialect.
    /// It is written as a conditional `UPDATE`-then-`INSERT` rather than
    /// `ON CONFLICT` because SQLite's and PostgreSQL's conflict clauses differ
    /// in what they allow.
    async fn tag(&self, work: &str, kind: &str, canonical: &str) {
        let node = format!("{kind}:{canonical}");
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => {
                for stmt in [
                    format!(
                        "INSERT OR IGNORE INTO taxonomy_nodes (id, kind, canonical, norm, created_at) \
                         VALUES ('{node}', '{kind}', '{canonical}', '{canonical}', '2026-01-01T00:00:00Z')"
                    ),
                    format!(
                        "INSERT OR IGNORE INTO work_tags (work_id, node_id, weight, added_at) \
                         VALUES ('{work}', '{node}', 1, '2026-01-01T00:00:00Z')"
                    ),
                ] {
                    self.exec(&stmt).await;
                }
            }
            // PostgreSQL takes the conflict clause as a suffix, SQLite as a
            // prefix. Neither accepts the other's spelling, and a shared
            // literal fails as a bare `syntax error at or near "OR"` with no
            // hint that a dialect is involved -- so the whole statement is
            // written twice rather than stitched from a prefix.
            lorehaven_db::Backend::Postgres => {
                for stmt in [
                    format!(
                        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) \
                         VALUES ('{node}', '{kind}', '{canonical}', '{canonical}', '2026-01-01T00:00:00Z') \
                         ON CONFLICT (id) DO NOTHING"
                    ),
                    format!(
                        "INSERT INTO work_tags (work_id, node_id, weight, added_at) \
                         VALUES ('{work}', '{node}', 1, '2026-01-01T00:00:00Z') \
                         ON CONFLICT (work_id, node_id) DO NOTHING"
                    ),
                ] {
                    self.exec(&stmt).await;
                }
            }
        }
    }

    /// Set the per-event delivery route for an account.
    async fn route(&self, account: &str, event: &str, channel: &str, enabled: bool) {
        self.exec(&format!(
            "INSERT INTO notification_routes (id, account_id, event_type, channel, enabled, created_at, updated_at) \
             VALUES ('{id}', '{account}', '{event}', '{channel}', {lit}, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
            id = uuid::Uuid::new_v4(),
            lit = self.bool_lit(enabled),
        ))
        .await;
    }

    /// Read one column as text, delegating to the harness's dialect-aware
    /// probe. `Option<String>` cannot tell SQL NULL from the empty string, so
    /// callers that care about NULL use [`Self::is_null`].
    async fn cell(&self, query: &str, id: &str) -> Option<String> {
        self.tdb.fetch_text(query, id).await
    }

    /// Whether the named row's single column is SQL NULL.
    ///
    /// This cannot be built on `TestDb::cell` + `TestDb::exists`: `cell`
    /// collapses NULL and `''` into the same `None`, and `exists` selects a
    /// non-nullable `i32` so it raises `UnexpectedNullError` on exactly the
    /// column this needs to inspect. The row and the value are read as one
    /// `Option<Option<String>>`, where the outer `None` is "no such row" and
    /// the inner one is NULL.
    ///
    /// It also does not use `TestDb::id_where`, which rewrites `id = ?` to
    /// `id::text = ?`; a nullable-column probe is selected for its value, not
    /// filtered by id, so the cast is applied here.
    /// The `WHERE` fragment for a comparison, as a bound-parameter form.
    ///
    /// `uuid_eq`/`text_eq` inline the value, which a bound-parameter probe
    /// cannot do. Same two shapes: `account_id`/`work_id` are `UUID` on
    /// PostgreSQL and TEXT on SQLite; `notifications.id` is TEXT on both.
    fn bound_eq(&self, col: &str, kind: ColKind) -> String {
        match kind {
            ColKind::Uuid => match self.db().backend() {
                lorehaven_db::Backend::Postgres => format!("{col}::text = ?"),
                lorehaven_db::Backend::Sqlite => format!("{col} = ?"),
            },
            ColKind::Text => format!("{col} = ?"),
        }
    }

    async fn is_null(&self, query: &str, id: &str) -> bool {
        let q = self.tdb.sql(query);
        // Outer `None` is "no such row"; `Some(None)` is the row with a NULL
        // column; `Some(Some(v))` is a value. Only the middle case is NULL --
        // a bare `is_some()` would answer "does the row exist" instead.
        let outer: Option<Option<String>> = match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar::<_, Option<String>>(&q)
                .bind(id)
                .fetch_optional(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("is_null"),
            lorehaven_db::Backend::Postgres => sqlx::query_scalar::<_, Option<String>>(&q)
                .bind(id)
                .fetch_optional(self.db().postgres_pool().expect("pg"))
                .await
                .expect("is_null"),
        };
        matches!(outer, Some(None))
    }

    /// How many rows an account has, ignoring read state.
    async fn count(&self, account: &str) -> i64 {
        let q = self.tdb.sql(&format!(
            "SELECT COUNT(*) FROM notifications WHERE {}",
            self.uuid_eq("account_id", account)
        ));
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar::<_, i64>(&q)
                .fetch_one(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("count"),
            lorehaven_db::Backend::Postgres => sqlx::query_scalar::<_, i64>(&q)
                .fetch_one(self.db().postgres_pool().expect("pg"))
                .await
                .expect("count"),
        }
    }
}

/// A rule hiding works tagged `blocked`.
///
/// Built through the real constructor, so the predicate is the one production
/// builds rather than a hand-rolled copy that could drift from it. `kind` and
/// `canonical` are matched as a pair, which is why the node is tagged through
/// [`Harness::tag`] with the same pair.
fn hide(canonical: &str) -> Vec<FilterRule> {
    vec![FilterRule {
        filter_type: "tag".to_string(),
        value: canonical.to_string(),
    }]
}

// ---------------------------------------------------------------------------
// notify
// ---------------------------------------------------------------------------

/// The plain case: a row lands, carrying the fields the caller passed.
#[tokio::test]
async fn notify_writes_a_row_with_the_fields_it_was_given() {
    let h = Harness::new("notify-basic").await;
    let account = h.account("basic").await;
    let work = h.work(None).await;

    let id = notify(
        h.db(),
        &account,
        "reply",
        "Someone replied",
        "Nice chapter",
        Some(&work),
    )
    .await
    .expect("notify");

    assert_eq!(h.count(&account).await, 1, "one row written");
    assert_eq!(
        h.cell("SELECT kind FROM notifications WHERE id = ?", &id)
            .await
            .as_deref(),
        Some("reply")
    );
}

/// `work_id` is optional context, and NULL is the normal case for an instance
/// notice. It must stay NULL rather than becoming an empty string.
#[tokio::test]
async fn a_notification_without_a_work_has_a_null_work_id() {
    let h = Harness::new("notify-no-work").await;
    let account = h.account("nowork").await;

    notify(
        h.db(),
        &account,
        "system",
        "Instance notice",
        "Welcome",
        None,
    )
    .await
    .expect("notify");

    // Read it back through the repository's own row type, which is the
    // contract callers see.
    let rows = list(h.db(), &account, 10).await.expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].work_id, None,
        "an instance notice carries no work, not an empty id"
    );
}

/// The NULL is real SQL NULL, not the empty string -- a `''` would render as a
/// link to a work that does not exist.
#[tokio::test]
async fn the_absent_work_id_is_sql_null_not_an_empty_string() {
    let h = Harness::new("notify-null-work").await;
    let account = h.account("nullwork").await;
    notify(h.db(), &account, "system", "t", "b", None)
        .await
        .expect("notify");

    assert!(
        h.is_null(
            &format!(
                "SELECT work_id FROM notifications WHERE {}",
                h.bound_eq("account_id", ColKind::Uuid)
            ),
            &account,
        )
        .await,
        "work_id is NULL"
    );
}

/// A `work_id` that is present comes back as the string it was given.
#[tokio::test]
async fn a_notification_with_a_work_keeps_its_work_id() {
    let h = Harness::new("notify-with-work").await;
    let account = h.account("withwork").await;
    let work = h.work(None).await;

    notify(h.db(), &account, "sale", "On sale", "50% off", Some(&work))
        .await
        .expect("notify");

    let rows = list(h.db(), &account, 10).await.expect("list");
    assert_eq!(rows[0].work_id.as_deref(), Some(work.as_str()));
}

/// A disabled event writes nothing -- and still returns an id. All five
/// current callers discard it, so this is a contract hazard rather than a live
/// bug; pinned so the next caller to trust it finds out here.
#[tokio::test]
async fn a_disabled_event_writes_nothing_but_still_returns_an_id() {
    let h = Harness::new("notify-disabled").await;
    let account = h.account("disabled").await;
    h.route(&account, "reply", "in_app", false).await;

    let returned = notify(h.db(), &account, "reply", "t", "b", None)
        .await
        .expect("notify must not error for a disabled event");

    assert_eq!(h.count(&account).await, 0, "no row is inserted");
    assert!(
        !returned.is_empty(),
        "it still returns a non-empty id ({returned}) that resolves to no row -- \
         every caller discards it today, so this is a hazard not a break"
    );
    assert_eq!(
        list(h.db(), &account, 10).await.expect("list").len(),
        0,
        "the returned id addresses nothing"
    );
}

/// The phantom id is genuinely unaddressable, which is the sharper statement
/// of the same contract hazard.
#[tokio::test]
async fn a_disabled_event_returns_an_id_that_addresses_no_row() {
    let h = Harness::new("phantom-id-contract").await;
    let account = h.account("phantom").await;
    h.route(&account, "gift", "in_app", false).await;

    let id = notify(h.db(), &account, "gift", "t", "b", None)
        .await
        .expect("notify");

    assert!(
        !mark_read(h.db(), &account, &id).await.expect("mark_read"),
        "the id notify returned cannot address any row, because notify wrote none"
    );
}

/// With no route configured the channel is `email`.
///
/// `settings::resolve_notification_channel` returns `Some("email")` for a
/// missing row, and the *column* default is `'in_app'`. Those are two
/// different defaults for two different things, and only the function's is
/// reachable: `notify` always supplies the resolved value, so the column
/// default is only ever used by a direct INSERT. Recorded because the
/// divergence is easy to misread as a bug in one direction or the other.
#[tokio::test]
async fn an_unset_channel_resolves_to_email() {
    let h = Harness::new("notify-default-channel").await;
    let account = h.account("defaultchan").await;

    notify(
        h.db(),
        &account,
        "mention",
        "You were mentioned",
        "hi",
        None,
    )
    .await
    .expect("notify");

    assert_eq!(
        h.cell(
            "SELECT delivery_channel FROM notifications WHERE account_id = ?",
            &account,
        )
        .await
        .as_deref(),
        Some("email"),
        "no route -> the resolver's default, which is not the column default"
    );
}

/// The column default is `'in_app'`, so the two defaults are genuinely
/// different values rather than the same one spelled two ways. This is what
/// makes the test above a real fact and not a tautology.
#[tokio::test]
async fn the_column_default_and_the_resolver_default_differ() {
    let h = Harness::new("notify-column-default").await;
    let account = h.account("coldefault").await;
    h.exec(&format!(
        "INSERT INTO notifications (id, account_id, kind, title, body, created_at) \
         VALUES ('{id}', '{account}', 'system', 't', 'b', '2026-01-01T00:00:00Z')",
        id = uuid::Uuid::new_v4(),
    ))
    .await;

    assert_eq!(
        h.cell(
            "SELECT delivery_channel FROM notifications WHERE account_id = ?",
            &account,
        )
        .await
        .as_deref(),
        Some("in_app"),
        "a direct INSERT falls back to the column default, not the resolver's"
    );
}

/// An enabled route's channel is stored. The value is `in_app` here on
/// purpose: it differs from both the resolver default (`email`) and the
/// disabled case, so a pass proves the route row was read rather than a
/// fallback happening to match.
#[tokio::test]
async fn an_enabled_routes_channel_is_stored() {
    let h = Harness::new("notify-route-channel").await;
    let account = h.account("routechan").await;
    h.route(&account, "sale", "in_app", true).await;

    notify(h.db(), &account, "sale", "Sale", "buy", None)
        .await
        .expect("notify");

    assert_eq!(
        h.cell(
            "SELECT delivery_channel FROM notifications WHERE account_id = ?",
            &account,
        )
        .await
        .as_deref(),
        Some("in_app"),
        "the routed channel, which differs from the resolver default"
    );
}

/// Routing is per event type: disabling replies must not silence sales.
#[tokio::test]
async fn routing_is_scoped_to_one_event_type() {
    let h = Harness::new("notify-route-scoped").await;
    let account = h.account("routescoped").await;
    h.route(&account, "reply", "in_app", false).await;

    notify(h.db(), &account, "reply", "t", "b", None)
        .await
        .expect("notify");
    notify(h.db(), &account, "sale", "t", "b", None)
        .await
        .expect("notify");

    assert_eq!(h.count(&account).await, 1, "only the sale landed");
    assert_eq!(
        h.cell(
            "SELECT kind FROM notifications WHERE account_id = ?",
            &account
        )
        .await
        .as_deref(),
        Some("sale")
    );
}

/// `work_id` is a bare `UUID` with no foreign key to `works` -- the column
/// definition in migration 0023 declares no `REFERENCES`, and only
/// `account_id` cascades. So a notification for a work that does not exist is
/// *stored*, and the row survives the work's deletion.
///
/// Recorded as a known gap (docs/known-gaps.md M12-D01) rather than pinned as
/// desired behaviour: the natural read is that a notification links to a work
/// and a dangling link is a bug. The fix is a migration plus a decision about
/// existing rows, so it is not a test-suite change.
#[tokio::test]
async fn a_notification_for_a_missing_work_is_stored_not_rejected() {
    let h = Harness::new("notify-missing-work").await;
    let account = h.account("missingwork").await;
    let ghost = uuid::Uuid::new_v4().to_string();

    let id = notify(h.db(), &account, "sale", "t", "b", Some(&ghost))
        .await
        .expect("notify -- the column has no foreign key, so this succeeds");

    assert_eq!(h.count(&account).await, 1);
    assert_eq!(
        list(h.db(), &account, 10).await.expect("list")[0]
            .work_id
            .as_deref(),
        Some(ghost.as_str()),
        "the dangling work id round-trips as given (known gap M12-D01)"
    );
    assert!(uuid::Uuid::parse_str(&id).is_ok());
}

// ---------------------------------------------------------------------------
// list / list_filtered
// ---------------------------------------------------------------------------

/// Only the caller's own entries, newest first.
#[tokio::test]
async fn list_returns_only_this_accounts_entries_newest_first() {
    let h = Harness::new("list-own").await;
    let account = h.account("own").await;
    let other = h.account("other").await;

    // Distinct `created_at`s written after the insert: `now_rfc3339` has
    // sub-second precision, so two rows written back to back can land in the
    // same millisecond and make the ordering assertion a coin flip.
    for (acct, title, when) in [
        (&account, "older", "2026-01-01T00:00:01Z"),
        (&account, "newer", "2026-01-01T00:00:02Z"),
        (&other, "theirs", "2026-01-01T00:00:03Z"),
    ] {
        notify(h.db(), acct, "system", title, "b", None)
            .await
            .expect("notify");
        h.exec(&format!(
            "UPDATE notifications SET created_at = '{when}' WHERE title = '{title}'"
        ))
        .await;
    }

    let rows = list(h.db(), &account, 10).await.expect("list");
    let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(
        titles,
        vec!["newer", "older"],
        "own entries only, newest first"
    );
}

/// The `LIMIT` is honoured.
#[tokio::test]
async fn list_honours_the_limit() {
    let h = Harness::new("list-limit").await;
    let account = h.account("limit").await;
    for i in 0..5 {
        notify(h.db(), &account, "system", &format!("n{i}"), "b", None)
            .await
            .expect("notify");
    }
    let rows = list(h.db(), &account, 10).await.expect("list");
    assert_eq!(list(h.db(), &account, 2).await.expect("list").len(), 2);
    assert_eq!(rows.len(), 5);
}

/// An empty inbox is an empty list, not an error.
#[tokio::test]
async fn an_empty_inbox_lists_nothing() {
    let h = Harness::new("list-empty").await;
    let account = h.account("empty").await;
    assert!(list(h.db(), &account, 10).await.expect("list").is_empty());
}

/// A limit of zero is an empty page, not "no limit".
#[tokio::test]
async fn a_limit_of_zero_returns_nothing() {
    let h = Harness::new("list-zero").await;
    let account = h.account("zero").await;
    notify(h.db(), &account, "system", "a", "b", None)
        .await
        .expect("notify");
    assert!(list(h.db(), &account, 0).await.expect("list").is_empty());
}

/// `unread` mirrors `read_at IS NULL` for every row.
#[tokio::test]
async fn the_unread_flag_mirrors_read_at() {
    let h = Harness::new("list-unread-flag").await;
    let account = h.account("flag").await;
    notify(h.db(), &account, "system", "unread one", "b", None)
        .await
        .expect("notify");
    notify(h.db(), &account, "system", "read one", "b", None)
        .await
        .expect("notify");
    h.exec("UPDATE notifications SET read_at = '2026-01-02T00:00:00Z' WHERE title = 'read one'")
        .await;

    let rows = list(h.db(), &account, 10).await.expect("list");
    let by_title = |t: &str| rows.iter().find(|r| r.title == t).expect("row");
    assert!(by_title("unread one").unread);
    assert!(!by_title("read one").unread);
}

/// Rows with the same `created_at` fall back to `id DESC`, so a page boundary
/// inside one timestamp is still ordered rather than arbitrary.
#[tokio::test]
async fn equal_timestamps_break_ties_on_id_descending() {
    let h = Harness::new("list-tiebreak").await;
    let account = h.account("tiebreak").await;
    for i in 0..3 {
        notify(h.db(), &account, "system", &format!("n{i}"), "b", None)
            .await
            .expect("notify");
    }
    h.exec(&format!(
        "UPDATE notifications SET created_at = '2026-01-01T00:00:00Z' WHERE {}",
        h.uuid_eq("account_id", &account)
    ))
    .await;

    let first = list(h.db(), &account, 10).await.expect("list");
    let second = list(h.db(), &account, 10).await.expect("list");
    let ids: Vec<&str> = first.iter().map(|r| r.id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.reverse();
    assert_eq!(ids, sorted, "id DESC within a tie");
    assert_eq!(
        first.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
        second.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
        "the order is stable across calls"
    );
}

/// A filtered work disappears from the list -- the case §46.7.1 exists for.
#[tokio::test]
async fn list_filtered_hides_a_filtered_work() {
    let h = Harness::new("list-filtered-hides").await;
    let account = h.account("filterhides").await;
    let blocked = h.work(Some("blocked")).await;
    let ok = h.work(Some("keep")).await;

    notify(
        h.db(),
        &account,
        "sale",
        "blocked work",
        "b",
        Some(&blocked),
    )
    .await
    .expect("notify");
    notify(h.db(), &account, "sale", "visible work", "b", Some(&ok))
        .await
        .expect("notify");

    let titles: Vec<String> = list_filtered(h.db(), &account, 10, &hide("blocked"))
        .await
        .expect("list_filtered")
        .into_iter()
        .map(|r| r.title)
        .collect();
    assert_eq!(titles, vec!["visible work"]);
}

/// A rule matching `kind` + `canonical` as a pair cannot be satisfied by a
/// node that shares the name under a different kind.
#[tokio::test]
async fn a_filter_matches_kind_and_canonical_together() {
    let h = Harness::new("list-filtered-pair").await;
    let account = h.account("filterpair").await;
    // Same canonical text, different kind.
    let work = h.work(None).await;
    h.tag(&work, "fandom", "blocked").await;

    notify(
        h.db(),
        &account,
        "sale",
        "same name other kind",
        "b",
        Some(&work),
    )
    .await
    .expect("notify");

    let titles: Vec<String> = list_filtered(h.db(), &account, 10, &hide("blocked"))
        .await
        .expect("list_filtered")
        .into_iter()
        .map(|r| r.title)
        .collect();
    assert_eq!(
        titles,
        vec!["same name other kind"],
        "blocking tag 'blocked' must not block fandom 'blocked'"
    );
}

/// An entry with no work is *not* filtered. §46.7.1 is about works not
/// reaching the reader, not about silencing the instance at someone who
/// filtered a tag -- and the predicate's `NOT EXISTS` over a NULL `work_id` is
/// trivially true, so the system notice survives without a special case.
#[tokio::test]
async fn an_entry_without_a_work_survives_a_filter() {
    let h = Harness::new("list-filtered-system").await;
    let account = h.account("filtersystem").await;
    let blocked = h.work(Some("blocked")).await;

    notify(h.db(), &account, "system", "instance notice", "b", None)
        .await
        .expect("notify");
    notify(
        h.db(),
        &account,
        "sale",
        "blocked work",
        "b",
        Some(&blocked),
    )
    .await
    .expect("notify");

    let titles: Vec<String> = list_filtered(h.db(), &account, 10, &hide("blocked"))
        .await
        .expect("list_filtered")
        .into_iter()
        .map(|r| r.title)
        .collect();
    assert_eq!(
        titles,
        vec!["instance notice"],
        "a workless notice is not a work and cannot be filtered"
    );
}

/// Several rules are OR'd, so any one of them excludes.
#[tokio::test]
async fn several_rules_exclude_on_any_match() {
    let h = Harness::new("list-filtered-multi").await;
    let account = h.account("filtermulti").await;
    let a = h.work(Some("aaa")).await;
    let b = h.work(Some("bbb")).await;
    let ok = h.work(Some("keep")).await;

    for (w, t) in [(&a, "first"), (&b, "second"), (&ok, "visible")] {
        notify(h.db(), &account, "sale", t, "b", Some(w))
            .await
            .expect("notify");
    }

    let mut rules = hide("aaa");
    rules.extend(hide("bbb"));
    let titles: Vec<String> = list_filtered(h.db(), &account, 10, &rules)
        .await
        .expect("list_filtered")
        .into_iter()
        .map(|r| r.title)
        .collect();
    assert_eq!(titles, vec!["visible"]);
}

/// No rules means no `AND` left dangling -- the empty-predicate case the module
/// comment calls out.
#[tokio::test]
async fn an_empty_rule_set_leaves_no_stray_and() {
    let h = Harness::new("list-filtered-empty-rules").await;
    let account = h.account("emptyrules").await;
    notify(h.db(), &account, "system", "still here", "b", None)
        .await
        .expect("notify");

    assert_eq!(
        list_filtered(h.db(), &account, 10, &[])
            .await
            .expect("list_filtered with no rules")
            .len(),
        1
    );
}

/// The exclusion sits in the `WHERE`, not after the `LIMIT`: a page filtered
/// after the fact comes back short and is indistinguishable from the end of
/// the inbox.
#[tokio::test]
async fn filtering_happens_before_the_limit() {
    let h = Harness::new("list-filtered-pre-limit").await;
    let account = h.account("prelimit").await;
    let blocked = h.work(Some("blocked")).await;
    let ok = h.work(Some("keep")).await;

    for i in 0..3 {
        notify(
            h.db(),
            &account,
            "sale",
            &format!("blocked {i}"),
            "b",
            Some(&blocked),
        )
        .await
        .expect("notify");
    }
    notify(h.db(), &account, "sale", "visible", "b", Some(&ok))
        .await
        .expect("notify");

    let titles: Vec<String> = list_filtered(h.db(), &account, 1, &hide("blocked"))
        .await
        .expect("list_filtered")
        .into_iter()
        .map(|r| r.title)
        .collect();
    assert_eq!(
        titles,
        vec!["visible"],
        "limit 1 returns the one unfiltered row, not a short page of blocked ones"
    );
}

/// `list` ignores filters; `list_filtered` does not. Same rows, different
/// result -- so the two cannot be wired to the same route by accident without a
/// test noticing.
#[tokio::test]
async fn list_ignores_filters_where_list_filtered_applies_them() {
    let h = Harness::new("list-vs-filtered").await;
    let account = h.account("vsfiltered").await;
    let blocked = h.work(Some("blocked")).await;
    notify(
        h.db(),
        &account,
        "sale",
        "blocked work",
        "b",
        Some(&blocked),
    )
    .await
    .expect("notify");

    assert_eq!(list(h.db(), &account, 10).await.expect("list").len(), 1);
    assert_eq!(
        list_filtered(h.db(), &account, 10, &hide("blocked"))
            .await
            .expect("list_filtered")
            .len(),
        0
    );
}

// ---------------------------------------------------------------------------
// unread_count / unread_count_filtered
// ---------------------------------------------------------------------------

/// The count sees only unread entries.
#[tokio::test]
async fn unread_count_ignores_entries_already_read() {
    let h = Harness::new("unread-basic").await;
    let account = h.account("unreadbasic").await;
    for i in 0..3 {
        notify(h.db(), &account, "system", &format!("n{i}"), "b", None)
            .await
            .expect("notify");
    }
    let id = list(h.db(), &account, 10).await.expect("list")[0]
        .id
        .clone();
    mark_read(h.db(), &account, &id).await.expect("mark_read");

    assert_eq!(unread_count(h.db(), &account).await.expect("unread"), 2);
}

/// The badge is per-account.
#[tokio::test]
async fn unread_count_is_scoped_to_the_account() {
    let h = Harness::new("unread-scoped").await;
    let mine = h.account("unreadmine").await;
    let theirs = h.account("unreadtheirs").await;
    notify(h.db(), &mine, "system", "mine", "b", None)
        .await
        .expect("notify");
    notify(h.db(), &theirs, "system", "theirs", "b", None)
        .await
        .expect("notify");

    assert_eq!(unread_count(h.db(), &mine).await.expect("unread"), 1);
    assert_eq!(unread_count(h.db(), &theirs).await.expect("unread"), 1);
}

/// An account with no notifications counts zero.
#[tokio::test]
async fn an_empty_inbox_counts_zero_unread() {
    let h = Harness::new("unread-empty").await;
    let account = h.account("unreadempty").await;
    assert_eq!(unread_count(h.db(), &account).await.expect("unread"), 0);
}

/// The filtered count agrees with the filtered list -- the "one-digit door" the
/// module's comment says §46.7.1 must not be left open. This is the
/// load-bearing test of the suite.
#[tokio::test]
async fn the_filtered_badge_never_counts_a_hidden_work() {
    let h = Harness::new("unread-filtered").await;
    let account = h.account("badgefilter").await;
    let blocked = h.work(Some("blocked")).await;
    let ok = h.work(Some("keep")).await;

    notify(h.db(), &account, "sale", "blocked", "b", Some(&blocked))
        .await
        .expect("notify");
    notify(h.db(), &account, "sale", "visible", "b", Some(&ok))
        .await
        .expect("notify");

    let listed = list_filtered(h.db(), &account, 10, &hide("blocked"))
        .await
        .expect("list_filtered");
    let badge = unread_count_filtered(h.db(), &account, &hide("blocked"))
        .await
        .expect("unread_count_filtered");

    assert_eq!(listed.len(), 1);
    assert_eq!(
        badge, 1,
        "badge agrees with the list it describes; counting the hidden work \
         would leak its existence through a one-digit door"
    );
}

/// A read entry does not contribute to the filtered badge either.
#[tokio::test]
async fn the_filtered_badge_ignores_read_entries() {
    let h = Harness::new("unread-filtered-read").await;
    let account = h.account("badgeread").await;
    let ok = h.work(Some("keep")).await;
    notify(h.db(), &account, "sale", "visible", "b", Some(&ok))
        .await
        .expect("notify");
    let id = list(h.db(), &account, 10).await.expect("list")[0]
        .id
        .clone();
    mark_read(h.db(), &account, &id).await.expect("mark_read");

    assert_eq!(
        unread_count_filtered(h.db(), &account, &hide("blocked"))
            .await
            .expect("unread_count_filtered"),
        0
    );
}

/// An empty rule set on the filtered count is the no-filter count.
#[tokio::test]
async fn an_empty_rule_set_leaves_the_badge_unfiltered() {
    let h = Harness::new("unread-filtered-empty").await;
    let account = h.account("badgeempty").await;
    notify(h.db(), &account, "system", "a", "b", None)
        .await
        .expect("notify");
    assert_eq!(
        unread_count_filtered(h.db(), &account, &[])
            .await
            .expect("unread"),
        1
    );
}

/// The unfiltered count still counts a hidden work. That is the intended
/// asymmetry: the filtered pair is what the inbox uses, and §46.7.1 is
/// enforced there. Pinned so a change that "fixes" the unfiltered count is
/// noticed as a behaviour change rather than passing silently.
#[tokio::test]
async fn the_unfiltered_count_still_counts_a_hidden_work() {
    let h = Harness::new("unread-unfiltered-asymmetry").await;
    let account = h.account("asymmetry").await;
    let blocked = h.work(Some("blocked")).await;
    notify(h.db(), &account, "sale", "blocked", "b", Some(&blocked))
        .await
        .expect("notify");

    assert_eq!(unread_count(h.db(), &account).await.expect("unread"), 1);
    assert_eq!(
        unread_count_filtered(h.db(), &account, &hide("blocked"))
            .await
            .expect("unread_count_filtered"),
        0
    );
}

/// Two rules bind two pairs, and the binding order has to follow the rule
/// order -- a mismatch would compare one rule's kind against another's value
/// and silently filter the wrong works.
#[tokio::test]
async fn multiple_rules_bind_in_rule_order() {
    let h = Harness::new("unread-filtered-bindorder").await;
    let account = h.account("bindorder").await;
    let a = h.work(Some("aaa")).await;
    let b = h.work(Some("bbb")).await;

    notify(h.db(), &account, "sale", "first", "b", Some(&a))
        .await
        .expect("notify");
    notify(h.db(), &account, "sale", "second", "b", Some(&b))
        .await
        .expect("notify");

    let mut rules = hide("aaa");
    rules.extend(hide("bbb"));
    assert_eq!(
        unread_count_filtered(h.db(), &account, &rules)
            .await
            .expect("unread_count_filtered"),
        0,
        "both pairs bound to their own rules"
    );
}

// ---------------------------------------------------------------------------
// mark_read
// ---------------------------------------------------------------------------

/// Marking an unread entry read returns `true` and clears the unread flag.
#[tokio::test]
async fn marking_an_entry_read_returns_true_and_clears_the_flag() {
    let h = Harness::new("mark-read-true").await;
    let account = h.account("marktrue").await;
    notify(h.db(), &account, "system", "a", "b", None)
        .await
        .expect("notify");
    let id = list(h.db(), &account, 10).await.expect("list")[0]
        .id
        .clone();

    assert!(mark_read(h.db(), &account, &id).await.expect("mark_read"));
    assert!(!list(h.db(), &account, 10).await.expect("list")[0].unread);
}

/// `read_at` is stamped, not left NULL.
#[tokio::test]
async fn marking_read_stamps_read_at() {
    let h = Harness::new("mark-read-stamp").await;
    let account = h.account("markstamp").await;
    notify(h.db(), &account, "system", "a", "b", None)
        .await
        .expect("notify");
    let id = list(h.db(), &account, 10).await.expect("list")[0]
        .id
        .clone();
    mark_read(h.db(), &account, &id).await.expect("mark_read");

    assert!(
        !h.is_null(
            &format!(
                "SELECT read_at FROM notifications WHERE {}",
                h.bound_eq("id", ColKind::Text)
            ),
            &id,
        )
        .await,
        "read_at is set"
    );
}

/// Idempotent: a second tap returns `false` rather than erroring, so a
/// double-tap or a replayed request is harmless.
#[tokio::test]
async fn marking_an_entry_read_twice_returns_false_the_second_time() {
    let h = Harness::new("mark-read-idempotent").await;
    let account = h.account("markidem").await;
    notify(h.db(), &account, "system", "a", "b", None)
        .await
        .expect("notify");
    let id = list(h.db(), &account, 10).await.expect("list")[0]
        .id
        .clone();

    assert!(mark_read(h.db(), &account, &id).await.expect("first"));
    assert!(
        !mark_read(h.db(), &account, &id).await.expect("second"),
        "the second call is a no-op, not an error"
    );
}

/// The first `read_at` is kept -- a second call must not restamp it, or "when
/// did I read this" would drift on every open.
#[tokio::test]
async fn a_second_mark_does_not_restamp_the_first_read_time() {
    let h = Harness::new("mark-read-no-restamp").await;
    let account = h.account("markrestamp").await;
    notify(h.db(), &account, "system", "a", "b", None)
        .await
        .expect("notify");
    let id = list(h.db(), &account, 10).await.expect("list")[0]
        .id
        .clone();

    h.exec(&format!(
        "UPDATE notifications SET read_at = '2026-01-01T00:00:00Z' WHERE {}",
        h.text_eq("id", &id)
    ))
    .await;
    assert!(!mark_read(h.db(), &account, &id).await.expect("mark_read"));
    assert_eq!(
        h.cell("SELECT read_at FROM notifications WHERE id = ?", &id)
            .await
            .as_deref(),
        Some("2026-01-01T00:00:00Z"),
        "the original read time stands"
    );
}

/// An unknown id is `false`, not an error.
#[tokio::test]
async fn marking_an_unknown_entry_read_returns_false() {
    let h = Harness::new("mark-read-unknown").await;
    let account = h.account("markunknown").await;
    assert!(
        !mark_read(h.db(), &account, &uuid::Uuid::new_v4().to_string())
            .await
            .expect("mark_read")
    );
}

/// One account cannot mark another's entry read. The `account_id` predicate is
/// what stops a guessed id from working, so this is a real authorization check.
#[tokio::test]
async fn one_account_cannot_mark_another_accounts_entry_read() {
    let h = Harness::new("mark-read-cross-account").await;
    let mine = h.account("markmine").await;
    let theirs = h.account("marktheirs").await;
    notify(h.db(), &theirs, "system", "theirs", "b", None)
        .await
        .expect("notify");
    let id = list(h.db(), &theirs, 10).await.expect("list")[0].id.clone();

    assert!(
        !mark_read(h.db(), &mine, &id).await.expect("mark_read"),
        "the WHERE clause scopes to the caller"
    );
    assert!(
        list(h.db(), &theirs, 10).await.expect("list")[0].unread,
        "and the other account's entry is still unread"
    );
}

// ---------------------------------------------------------------------------
// mark_all_read
// ---------------------------------------------------------------------------

/// Returns how many entries the call closed.
#[tokio::test]
async fn mark_all_read_reports_how_many_it_closed() {
    let h = Harness::new("mark-all-count").await;
    let account = h.account("markallcount").await;
    for i in 0..3 {
        notify(h.db(), &account, "system", &format!("n{i}"), "b", None)
            .await
            .expect("notify");
    }
    assert_eq!(
        mark_all_read(h.db(), &account)
            .await
            .expect("mark_all_read"),
        3
    );
    assert_eq!(unread_count(h.db(), &account).await.expect("unread"), 0);
}

/// Already-read entries are not recounted.
#[tokio::test]
async fn mark_all_read_counts_only_what_it_actually_closed() {
    let h = Harness::new("mark-all-only-unread").await;
    let account = h.account("markallunread").await;
    for i in 0..3 {
        notify(h.db(), &account, "system", &format!("n{i}"), "b", None)
            .await
            .expect("notify");
    }
    let id = list(h.db(), &account, 10).await.expect("list")[0]
        .id
        .clone();
    mark_read(h.db(), &account, &id).await.expect("mark_read");

    assert_eq!(
        mark_all_read(h.db(), &account)
            .await
            .expect("mark_all_read"),
        2,
        "one was already read"
    );
}

/// An empty inbox closes nothing.
#[tokio::test]
async fn mark_all_read_on_an_empty_inbox_closes_nothing() {
    let h = Harness::new("mark-all-empty").await;
    let account = h.account("markallempty").await;
    assert_eq!(
        mark_all_read(h.db(), &account)
            .await
            .expect("mark_all_read"),
        0
    );
}

/// It does not touch another account's entries.
#[tokio::test]
async fn mark_all_read_leaves_other_accounts_entries_unread() {
    let h = Harness::new("mark-all-scoped").await;
    let mine = h.account("markallmine").await;
    let theirs = h.account("markalltheirs").await;
    notify(h.db(), &mine, "system", "mine", "b", None)
        .await
        .expect("notify");
    notify(h.db(), &theirs, "system", "theirs", "b", None)
        .await
        .expect("notify");

    assert_eq!(
        mark_all_read(h.db(), &mine).await.expect("mark_all_read"),
        1
    );
    assert_eq!(
        unread_count(h.db(), &theirs).await.expect("unread"),
        1,
        "scoped to the caller"
    );
}

/// Running it twice is harmless and the second pass closes nothing.
#[tokio::test]
async fn mark_all_read_twice_is_harmless() {
    let h = Harness::new("mark-all-twice").await;
    let account = h.account("markalltwice").await;
    notify(h.db(), &account, "system", "a", "b", None)
        .await
        .expect("notify");
    assert_eq!(mark_all_read(h.db(), &account).await.expect("first"), 1);
    assert_eq!(mark_all_read(h.db(), &account).await.expect("second"), 0);
}

/// Read-all also closes entries the reader has filtered -- they are still
/// their rows, and leaving them unread would keep inflating the unfiltered
/// badge forever with entries the reader can never act on.
#[tokio::test]
async fn read_all_also_closes_entries_the_reader_has_filtered() {
    let h = Harness::new("mark-all-filtered").await;
    let account = h.account("markallfiltered").await;
    let blocked = h.work(Some("blocked")).await;
    notify(h.db(), &account, "sale", "blocked", "b", Some(&blocked))
        .await
        .expect("notify");

    assert_eq!(
        mark_all_read(h.db(), &account)
            .await
            .expect("mark_all_read"),
        1
    );
    assert_eq!(unread_count(h.db(), &account).await.expect("unread"), 0);
}

// ---------------------------------------------------------------------------
// Cross-cutting
// ---------------------------------------------------------------------------

/// The `id` is a UUID-shaped string on both backends, so a row written here is
/// addressable by a later `mark_read` that got its id from a JSON response.
#[tokio::test]
async fn the_returned_id_addresses_the_row_it_wrote() {
    let h = Harness::new("notify-id-usable").await;
    let account = h.account("idusable").await;
    let id = notify(h.db(), &account, "system", "a", "b", None)
        .await
        .expect("notify");

    assert!(
        uuid::Uuid::parse_str(&id).is_ok(),
        "id {id} is a parseable UUID on both backends"
    );
    assert!(mark_read(h.db(), &account, &id).await.expect("mark_read"));
}

/// Titles and bodies carry user-controlled text and are bound, never
/// interpolated, so SQL-ish punctuation round-trips unchanged.
#[tokio::test]
async fn titles_and_bodies_round_trip_unchanged() {
    let h = Harness::new("notify-escaping").await;
    let account = h.account("escaping").await;
    let nasty = "O'Brien; DROP TABLE notifications; -- \u{1F600}";

    notify(h.db(), &account, "system", nasty, nasty, None)
        .await
        .expect("notify");

    let rows = list(h.db(), &account, 10).await.expect("list");
    assert_eq!(rows[0].title, nasty);
    assert_eq!(rows[0].body, nasty);
    assert_eq!(h.count(&account).await, 1, "the table is still there");
}

/// The insert and the two SELECTs agree on column order. If this fails, the
/// `RawRow` tuple and the statements have drifted apart -- which would show up
/// as a title in the body field, not as an error.
#[tokio::test]
async fn the_insert_and_the_selects_agree_on_column_order() {
    let h = Harness::new("notify-column-order").await;
    let account = h.account("colorder").await;
    notify(h.db(), &account, "mention", "the title", "the body", None)
        .await
        .expect("notify");

    let rows = list(h.db(), &account, 10).await.expect("list");
    assert_eq!(rows[0].kind, "mention");
    assert_eq!(rows[0].title, "the title");
    assert_eq!(rows[0].body, "the body");
    assert!(!rows[0].created_at.is_empty());
}

/// Deleting the account cascades the inbox away, so a deleted account leaves no
/// notifications behind for a recycled address to inherit.
#[tokio::test]
async fn deleting_an_account_cascades_its_notifications() {
    let h = Harness::new("notify-cascade").await;
    let account = h.account("cascade").await;
    notify(h.db(), &account, "system", "a", "b", None)
        .await
        .expect("notify");
    assert_eq!(h.count(&account).await, 1);

    h.exec(&format!(
        "DELETE FROM accounts WHERE {}",
        h.uuid_eq("id", &account)
    ))
    .await;
    assert_eq!(h.count(&account).await, 0, "ON DELETE CASCADE fires");
}
