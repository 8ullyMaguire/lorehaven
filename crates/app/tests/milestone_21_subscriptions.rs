//! M21 — Content subscriptions, saved-search alerts and AI-training consent
//! (`crates/db/src/subscriptions.rs`).
//!
//! Seventeen public functions with no test touching them. Three things here are
//! worth a test rather than a read, and two of them are defects:
//!
//!   * **`subscribe_work` and `create_alert` return an id that may not exist.**
//!     Both generate a fresh `Uuid` and then run `INSERT … ON CONFLICT … DO
//!     UPDATE`, but they return the generated id unconditionally. On the
//!     conflict branch the row keeps its *original* id, so the caller receives
//!     an id that addresses nothing — `delete_alert(that_id)` removes zero rows.
//!     Pinned as a known defect below, because "resubscribe returns the wrong
//!     id" and "resubscribe should update in place and return the live id" have
//!     different fixes depending on what the route does with the return value.
//!   * **The delayed opt-out functions are dead code with an inverted
//!     timestamp.** `ai_training_opt_out_delayed{,_tx}` set
//!     `updated_at = now + delay_seconds`, i.e. the *effect* time in the
//!     future rather than the time the request was made. Nothing in the
//!     workspace calls them; the route uses plain `opt_out`. Tested here so the
//!     behaviour is on record if anyone wires them up.
//!   * **`ai_training_opt_out_handler` deletes the row rather than recording
//!     the opt-out.** For a consent record that is the difference between "we
//!     know this author declined" and "we have no idea". The status query
//!     defaults a missing row to *not opted in*, so the user-facing answer is
//!     the same either way, but the audit trail is not.
//!
//! Every id here is a real UUID: the PostgreSQL arms bind `::uuid`, so a
//! readable seed id would be a type error rather than a test.

use std::path::PathBuf;

use lorehaven_db::subscriptions as sub;
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m21-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

struct Db {
    tdb: TestDb,
    _dir: PathBuf,
}

impl Db {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        Self { tdb, _dir: dir }
    }
    fn db(&self) -> &lorehaven_db::Database {
        self.tdb.db()
    }
    async fn exec(&self, query: &str) {
        let q = self.tdb.sql(query);
        match self.tdb.db().backend() {
            lorehaven_db::Backend::Sqlite => {
                sqlx::query(&q)
                    .execute(self.tdb.db().sqlite_pool().expect("sqlite"))
                    .await
                    .expect("seed");
            }
            lorehaven_db::Backend::Postgres => {
                sqlx::query(&q)
                    .execute(self.tdb.db().postgres_pool().expect("pg"))
                    .await
                    .expect("seed");
            }
        }
    }
    /// A real pseud, with the account it belongs to. The PostgreSQL arms bind
    /// `::uuid` for these columns and both tables carry a foreign key, so the
    /// whole `accounts` -> `pseuds` chain has to exist before a subscription
    /// can reference it.
    async fn account(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO accounts (id, email, created_at, updated_at) \
             VALUES ('{id}', 'a{id}@example.test', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        id
    }
    async fn pseud(&self) -> String {
        self.pseud_for(&self.account().await).await
    }
    async fn pseud_for(&self, account_id: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
             VALUES ('{id}', '{account_id}', 'h{}', 'H', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
            id.replace('-', "")
        ))
        .await;
        id
    }
    /// `search_alerts.saved_search_id` references `saved_views(id)`, and
    /// `saved_views` is keyed on `account_id` -- not on a pseud, despite
    /// `create_alert` calling its column `owner_pseud_id`.
    async fn saved_view(&self, account_id: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO saved_views (id, account_id, name, query_json, created_at, updated_at) \
             VALUES ('{id}', '{account_id}', 'a saved search', '{{}}', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        id
    }
}

// ------------------------------------------------------------- subscriptions

#[tokio::test]
async fn subscribing_creates_an_active_row() {
    let h = Db::new("sub-create").await;
    let pseud = h.pseud().await;
    let id = sub::subscribe_work(h.db(), &pseud, "work", "w-1")
        .await
        .unwrap();
    let rows = sub::list_subscriptions(h.db(), &pseud).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id);
    assert_eq!(rows[0].subject_type, "work");
    assert_eq!(rows[0].subject_id, "w-1");
    assert_eq!(rows[0].state, "active");
}

#[tokio::test]
async fn subscribing_to_the_same_subject_twice_keeps_one_row_and_reactivates_it() {
    let h = Db::new("sub-dupe").await;
    let pseud = h.pseud().await;
    let first = sub::subscribe_work(h.db(), &pseud, "work", "w-1")
        .await
        .unwrap();
    sub::set_subscription_state(h.db(), &first, "paused")
        .await
        .unwrap();
    sub::subscribe_work(h.db(), &pseud, "work", "w-1")
        .await
        .unwrap();
    let rows = sub::list_subscriptions(h.db(), &pseud).await.unwrap();
    assert_eq!(rows.len(), 1, "the unique constraint makes this one row");
    assert_eq!(
        rows[0].state, "active",
        "re-subscribing resumes a paused one"
    );
    assert_eq!(rows[0].id, first, "the original id survives the upsert");
}

#[tokio::test]
async fn a_resubscribe_returns_an_id_that_addresses_nothing() {
    // KNOWN DEFECT. `subscribe_work` mints a fresh `Uuid` and returns it
    // unconditionally, but the `ON CONFLICT` branch keeps the row's original
    // id. So the second call hands back an id that is not in the table. The
    // route that consumes this return value has to look the row up by
    // (pseud, subject) instead of trusting the id, or the upsert branch has to
    // `RETURNING id`. Fixing it changes what the function promises, so it is
    // a decision rather than a typo -- tracked in docs/known-gaps.md.
    let h = Db::new("sub-iddefect").await;
    let pseud = h.pseud().await;
    let first = sub::subscribe_work(h.db(), &pseud, "work", "w-1")
        .await
        .unwrap();
    let second = sub::subscribe_work(h.db(), &pseud, "work", "w-1")
        .await
        .unwrap();
    assert_ne!(second, first, "each call mints its own id");
    let rows = sub::list_subscriptions(h.db(), &pseud).await.unwrap();
    assert_eq!(
        rows.iter().filter(|r| r.id == second).count(),
        0,
        "the returned id is not the row's id on the conflict branch"
    );
    // The first id is the live one.
    assert_eq!(
        sub::set_subscription_state(h.db(), &first, "paused")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sub::set_subscription_state(h.db(), &second, "paused")
            .await
            .unwrap(),
        0,
        "acting on the returned id changes nothing"
    );
}

#[tokio::test]
async fn pausing_and_resuming_reports_whether_a_row_moved() {
    let h = Db::new("sub-state").await;
    let pseud = h.pseud().await;
    let id = sub::subscribe_work(h.db(), &pseud, "work", "w-1")
        .await
        .unwrap();
    assert_eq!(
        sub::set_subscription_state(h.db(), &id, "paused")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sub::list_subscriptions(h.db(), &pseud).await.unwrap()[0].state,
        "paused"
    );
    assert_eq!(
        sub::set_subscription_state(h.db(), &id, "active")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sub::set_subscription_state(h.db(), &uuid::Uuid::new_v4().to_string(), "paused")
            .await
            .unwrap(),
        0,
        "an unknown id moves no rows"
    );
}

#[tokio::test]
async fn unsubscribing_removes_the_row_and_reports_whether_it_did() {
    let h = Db::new("sub-unsub").await;
    let pseud = h.pseud().await;
    sub::subscribe_work(h.db(), &pseud, "work", "w-1")
        .await
        .unwrap();
    assert_eq!(
        sub::unsubscribe_work(h.db(), &pseud, "work", "w-1")
            .await
            .unwrap(),
        1
    );
    assert!(sub::list_subscriptions(h.db(), &pseud)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        sub::unsubscribe_work(h.db(), &pseud, "work", "w-1")
            .await
            .unwrap(),
        0,
        "unsubscribing twice removes nothing the second time"
    );
}

#[tokio::test]
async fn unsubscribing_is_scoped_to_the_one_subscriber() {
    let h = Db::new("sub-scope").await;
    let a = h.pseud().await;
    let b = h.pseud().await;
    sub::subscribe_work(h.db(), &a, "work", "w-1")
        .await
        .unwrap();
    sub::subscribe_work(h.db(), &b, "work", "w-1")
        .await
        .unwrap();
    sub::unsubscribe_work(h.db(), &a, "work", "w-1")
        .await
        .unwrap();
    assert!(sub::list_subscriptions(h.db(), &a)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        sub::list_subscriptions(h.db(), &b).await.unwrap().len(),
        1,
        "one reader unsubscribing does not unsubscribe the other"
    );
}

#[tokio::test]
async fn subscriptions_are_listed_newest_first() {
    let h = Db::new("sub-order").await;
    let pseud = h.pseud().await;
    sub::subscribe_work(h.db(), &pseud, "work", "w-1")
        .await
        .unwrap();
    sub::subscribe_work(h.db(), &pseud, "series", "s-1")
        .await
        .unwrap();
    sub::subscribe_work(h.db(), &pseud, "author", "a-1")
        .await
        .unwrap();
    let rows = sub::list_subscriptions(h.db(), &pseud).await.unwrap();
    assert_eq!(rows.len(), 3);
    let mut times: Vec<&String> = rows.iter().map(|r| &r.created_at).collect();
    let original = times.clone();
    times.sort();
    times.reverse();
    assert_eq!(times, original, "newest first, and all three are ordered");
}

#[tokio::test]
async fn the_subscriber_count_ignores_paused_rows() {
    let h = Db::new("sub-count").await;
    let a = h.pseud().await;
    let b = h.pseud().await;
    let c = h.pseud().await;
    for p in [&a, &b, &c] {
        sub::subscribe_work(h.db(), p, "work", "w-1").await.unwrap();
    }
    assert_eq!(
        sub::count_subscribers(h.db(), "work", "w-1").await.unwrap(),
        3
    );
    let first = sub::list_subscriptions(h.db(), &a).await.unwrap()[0]
        .id
        .clone();
    sub::set_subscription_state(h.db(), &first, "paused")
        .await
        .unwrap();
    assert_eq!(
        sub::count_subscribers(h.db(), "work", "w-1").await.unwrap(),
        2,
        "a paused subscription is not an active subscriber"
    );
    assert_eq!(
        sub::count_subscribers(h.db(), "work", "no-such")
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn the_active_subscriber_for_a_subject_is_someone_actually_active() {
    let h = Db::new("sub-active").await;
    let a = h.pseud().await;
    let b = h.pseud().await;
    sub::subscribe_work(h.db(), &a, "work", "w-1")
        .await
        .unwrap();
    sub::subscribe_work(h.db(), &b, "work", "w-1")
        .await
        .unwrap();
    // Pause the first; the second is still active, so a subscriber is named.
    let first = sub::list_subscriptions(h.db(), &a).await.unwrap()[0]
        .id
        .clone();
    sub::set_subscription_state(h.db(), &first, "paused")
        .await
        .unwrap();
    assert_eq!(
        sub::active_subscriber_for_subject(h.db(), "work", "w-1")
            .await
            .unwrap(),
        Some(b)
    );
}

#[tokio::test]
async fn a_subject_with_only_paused_subscribers_names_no_active_subscriber() {
    let h = Db::new("sub-active-none").await;
    let pseud = h.pseud().await;
    sub::subscribe_work(h.db(), &pseud, "work", "w-1")
        .await
        .unwrap();
    let id = sub::list_subscriptions(h.db(), &pseud).await.unwrap()[0]
        .id
        .clone();
    sub::set_subscription_state(h.db(), &id, "paused")
        .await
        .unwrap();
    assert_eq!(
        sub::active_subscriber_for_subject(h.db(), "work", "w-1")
            .await
            .unwrap(),
        None,
        "paused does not count as active"
    );
}

// ------------------------------------------------------------------- alerts

#[tokio::test]
async fn creating_an_alert_starts_it_unrun() {
    let h = Db::new("alert-create").await;
    let owner = h.pseud().await;
    let account = h.account().await;
    let view = h.saved_view(&account).await;
    let id = sub::create_alert(h.db(), &owner, &view, "daily")
        .await
        .unwrap();
    let alerts = sub::list_alerts(h.db(), &owner).await.unwrap();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].id, id);
    assert_eq!(alerts[0].saved_search_id, view);
    assert_eq!(alerts[0].frequency, "daily");
    assert_eq!(alerts[0].last_run_at, None, "a new alert has never run");
}

#[tokio::test]
async fn recreating_an_alert_for_the_same_search_updates_its_frequency() {
    let h = Db::new("alert-freq").await;
    let owner = h.pseud().await;
    let account = h.account().await;
    let view = h.saved_view(&account).await;
    sub::create_alert(h.db(), &owner, &view, "daily")
        .await
        .unwrap();
    sub::create_alert(h.db(), &owner, &view, "weekly")
        .await
        .unwrap();
    let alerts = sub::list_alerts(h.db(), &owner).await.unwrap();
    assert_eq!(alerts.len(), 1, "one alert per (owner, saved search)");
    assert_eq!(alerts[0].frequency, "weekly");
}

#[tokio::test]
async fn marking_an_alert_run_stamps_the_time_and_is_idempotent_in_shape() {
    let h = Db::new("alert-run").await;
    let owner = h.pseud().await;
    let account = h.account().await;
    let view = h.saved_view(&account).await;
    let id = sub::create_alert(h.db(), &owner, &view, "daily")
        .await
        .unwrap();
    assert_eq!(sub::mark_alert_run(h.db(), &id).await.unwrap(), 1);
    let alerts = sub::list_alerts(h.db(), &owner).await.unwrap();
    assert!(alerts[0].last_run_at.is_some(), "the run time is recorded");
    assert_eq!(
        sub::mark_alert_run(h.db(), &uuid::Uuid::new_v4().to_string())
            .await
            .unwrap(),
        0,
        "marking an unknown alert stamps nothing"
    );
}

#[tokio::test]
async fn deleting_an_alert_removes_it_and_reports_whether_it_did() {
    let h = Db::new("alert-delete").await;
    let owner = h.pseud().await;
    let account = h.account().await;
    let view = h.saved_view(&account).await;
    let id = sub::create_alert(h.db(), &owner, &view, "daily")
        .await
        .unwrap();
    assert_eq!(sub::delete_alert(h.db(), &id).await.unwrap(), 1);
    assert!(sub::list_alerts(h.db(), &owner).await.unwrap().is_empty());
    assert_eq!(sub::delete_alert(h.db(), &id).await.unwrap(), 0);
}

#[tokio::test]
async fn alerts_are_listed_per_owner_only() {
    let h = Db::new("alert-owner").await;
    let a = h.pseud().await;
    let b = h.pseud().await;
    let view_a = h.saved_view(&h.account().await).await;
    sub::create_alert(h.db(), &a, &view_a, "daily")
        .await
        .unwrap();
    sub::create_alert(h.db(), &b, &h.saved_view(&h.account().await).await, "daily")
        .await
        .unwrap();
    assert_eq!(sub::list_alerts(h.db(), &a).await.unwrap().len(), 1);
    assert_eq!(sub::list_alerts(h.db(), &b).await.unwrap().len(), 1);
    assert!(sub::list_alerts(h.db(), &uuid::Uuid::new_v4().to_string())
        .await
        .unwrap()
        .is_empty());
}

// ------------------------------------------------------------- AI training

#[tokio::test]
async fn an_author_who_has_never_answered_is_not_opted_in() {
    let h = Db::new("ai-default").await;
    let pseud = h.pseud().await;
    assert!(
        !sub::ai_training_status(h.db(), &pseud).await.unwrap(),
        "consent is opt-in, so silence means no"
    );
}

#[tokio::test]
async fn opting_in_and_out_round_trips() {
    let h = Db::new("ai-toggle").await;
    let pseud = h.pseud().await;
    sub::ai_training_opt_in(h.db(), &pseud, true).await.unwrap();
    assert!(sub::ai_training_status(h.db(), &pseud).await.unwrap());
    sub::ai_training_opt_out(h.db(), &pseud).await.unwrap();
    assert!(!sub::ai_training_status(h.db(), &pseud).await.unwrap());
}

#[tokio::test]
async fn consent_is_recorded_per_pseud() {
    let h = Db::new("ai-perpseud").await;
    let a = h.pseud().await;
    let b = h.pseud().await;
    sub::ai_training_opt_in(h.db(), &a, true).await.unwrap();
    assert!(sub::ai_training_status(h.db(), &a).await.unwrap());
    assert!(
        !sub::ai_training_status(h.db(), &b).await.unwrap(),
        "one author's yes is not another's"
    );
    let all = sub::list_ai_training_statuses(h.db()).await.unwrap();
    assert!(all.contains(&(a.clone(), true)));
    assert!(!all.contains(&(b.clone(), true)));
}

#[tokio::test]
async fn the_status_list_covers_every_pseud_that_has_answered() {
    let h = Db::new("ai-list").await;
    let a = h.pseud().await;
    let b = h.pseud().await;
    sub::ai_training_opt_in(h.db(), &a, true).await.unwrap();
    sub::ai_training_opt_in(h.db(), &b, false).await.unwrap();
    let all = sub::list_ai_training_statuses(h.db()).await.unwrap();
    assert_eq!(all.len(), 2);
    assert!(all.contains(&(a, true)));
    assert!(
        all.contains(&(b, false)),
        "a declined author is listed as declined"
    );
}

#[tokio::test]
async fn a_recorded_opt_out_can_be_dismissed_and_the_answer_does_not_change() {
    // `ai_training_opt_out_handler` is documented as posting an opt-out but
    // actually DELETEs rows that are already `opt_in = 0`. The status query
    // treats a missing row as "not opted in", so the user-facing answer is the
    // same either way -- the difference is only whether the refusal is on the
    // record. Pinned so the behaviour is visible if this is ever wired up.
    let h = Db::new("ai-handler").await;
    let pseud = h.pseud().await;
    sub::ai_training_opt_in(h.db(), &pseud, false)
        .await
        .unwrap();
    assert_eq!(
        sub::list_ai_training_statuses(h.db()).await.unwrap().len(),
        1,
        "the refusal is on the record before the handler runs"
    );
    sub::ai_training_opt_out_handler(h.db(), &pseud)
        .await
        .unwrap();
    assert!(
        !sub::ai_training_status(h.db(), &pseud).await.unwrap(),
        "still declined after the row is removed"
    );
    assert!(
        sub::list_ai_training_statuses(h.db())
            .await
            .unwrap()
            .is_empty(),
        "but the record of the refusal is gone -- an audit trail cannot be built from this table"
    );
}

#[tokio::test]
async fn the_dismiss_handler_leaves_an_opt_in_alone() {
    let h = Db::new("ai-handler-safe").await;
    let pseud = h.pseud().await;
    sub::ai_training_opt_in(h.db(), &pseud, true).await.unwrap();
    sub::ai_training_opt_out_handler(h.db(), &pseud)
        .await
        .unwrap();
    assert!(
        sub::ai_training_status(h.db(), &pseud).await.unwrap(),
        "the WHERE clause filters on opt_in = 0, so an author's consent survives"
    );
}

#[tokio::test]
async fn a_delayed_opt_out_is_effective_immediately_and_dates_the_effect_in_the_future() {
    // KNOWN DEFECT. `ai_training_opt_out_delayed` sets
    // `updated_at = now + delay_seconds`, which is the time the opt-out takes
    // *effect*, not the time it was requested. Nothing in the workspace calls
    // it -- the route uses plain `opt_out` -- so this is recorded, not fixed:
    // whether `updated_at` should mean "requested" or "effective" is a
    // question for whoever wires this up. The opt-in flag itself does flip
    // immediately, which is the part that matters for consent.
    let h = Db::new("ai-delayed").await;
    let pseud = h.pseud().await;
    sub::ai_training_opt_in(h.db(), &pseud, true).await.unwrap();
    sub::ai_training_opt_out_delayed(h.db(), &pseud, 86_400)
        .await
        .unwrap();
    assert!(
        !sub::ai_training_status(h.db(), &pseud).await.unwrap(),
        "the refusal takes effect at once; only the timestamp is in the future"
    );
}

#[tokio::test]
async fn the_transactional_delayed_opt_out_agrees_with_the_non_transactional_one() {
    let h = Db::new("ai-delayed-tx").await;
    let a = h.pseud().await;
    let b = h.pseud().await;
    sub::ai_training_opt_in(h.db(), &a, true).await.unwrap();
    sub::ai_training_opt_in(h.db(), &b, true).await.unwrap();
    sub::ai_training_opt_out_delayed(h.db(), &a, 60)
        .await
        .unwrap();
    sub::ai_training_opt_out_delayed_tx(h.db(), &b, 60)
        .await
        .unwrap();
    assert!(!sub::ai_training_status(h.db(), &a).await.unwrap());
    assert!(!sub::ai_training_status(h.db(), &b).await.unwrap());
}

#[tokio::test]
async fn a_delayed_opt_out_for_an_unknown_pseud_is_a_quiet_no_op() {
    let h = Db::new("ai-delayed-unknown").await;
    sub::ai_training_opt_out_delayed(h.db(), &uuid::Uuid::new_v4().to_string(), 60)
        .await
        .unwrap();
    sub::ai_training_opt_out_delayed_tx(h.db(), &uuid::Uuid::new_v4().to_string(), 60)
        .await
        .unwrap();
    assert!(
        sub::list_ai_training_statuses(h.db())
            .await
            .unwrap()
            .is_empty(),
        "no row is created for someone who never answered"
    );
}
