//! Acceptance: reason-tagged kudos and line-level highlights (spec §50.1,
//! M45-24).
//!
//! The arithmetic was already unit-tested in `crates/domain/src/reasons.rs` and
//! stored by `crates/db/src/reasons_store.rs`; **no route read or wrote either**.
//! These tests drive the door, for the reason `tasting_menu.rs` gives at the top:
//! a mechanism nothing reaches is what `docs/goal.md` calls not-complete.
//!
//! §50.4's clauses, and the test that pins each:
//!
//! | clause | test |
//! |---|---|
//! | a reason trains; the same kudos without one trains nothing; both accepted | `a_kudos_with_a_reason_is_stored_and_reads_back`, `a_bare_kudos_is_accepted_and_carries_no_reason` |
//! | a reason outside the fixed set is refused | `a_reason_outside_the_fixed_set_is_refused` |
//! | a free-text note is stored but never aggregated | `a_note_is_stored_but_does_not_become_a_reason` |
//! | fifty highlights move gravity by one signal | `many_highlights_move_a_work_by_one_signal` |
//! | a highlight counts once, however many | `one_readers_many_highlights_count_as_one_reader` |
//! | a work's highlights die with the work | `deleting_a_work_deletes_its_highlights` |
//! | a highlight is reachable at all | `a_highlight_requires_a_session` |

use axum::http::StatusCode;
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::{Backend, Database};
use serde_json::{json, Value};
use test_support::{id, scratch_dir, TestClient, TestDb};

fn config_for(dir: &std::path::Path) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config.database = match std::env::var("LOREHAVEN_TEST_PG_URL") {
        Ok(url) => lorehaven_db::DatabaseConfig::new(url),
        Err(_) => lorehaven_db::DatabaseConfig::new(format!(
            "sqlite://{}/lorehaven.sqlite?mode=rwc",
            dir.display()
        )),
    };
    // The rate-limit buckets are process-global at 127.0.0.1, so neighbouring
    // suites exhaust the development defaults long before this file finishes.
    // Both endpoints here are `RouteClass::Write`, so the write bucket is the one
    // that matters -- and the auth bucket, because `reader` signs in.
    config.rate_limits.auth = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.write = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.default = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config
}

struct Harness {
    tdb: TestDb,
    _dir: std::path::PathBuf,
    config: Config,
    db: Database,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let config = config_for(&dir);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let db = tdb.db().clone();
        Self {
            tdb,
            _dir: dir,
            config,
            db,
        }
    }

    fn client(&self) -> TestClient {
        TestClient::new(server::build_router(AppState::new(
            self.config.clone(),
            self.db.clone(),
        )))
    }

    /// `sign_in_as` rather than register-then-login: it clears the previous
    /// identity's cookies, so two readers inside one test cannot silently answer
    /// as each other -- which is the failure `one_readers_many_highlights_count_as
    /// _one_reader` would otherwise hide.
    async fn reader(&self, handle: &str) -> (TestClient, String) {
        let mut client = self.client();
        let email = format!("{handle}@example.test");
        let account = test_support::sign_in_as(&mut client, &self.tdb, &email, handle).await;
        (client, account)
    }

    /// One published, public work owned by `account`.
    ///
    /// The pseud is keyed on the *work*, not the account, because `pseuds` has a
    /// UNIQUE index on the normalized handle and a reader may own several works.
    /// Keying it on the account makes the second call in a test collide on that
    /// index -- which is a fixture bug that reads exactly like a product defect.
    ///
    /// Every statement is spelled per engine: `datetime('now')` is SQLite-only,
    /// `now()` is PostgreSQL-only, and the id columns are `TEXT` on one engine and
    /// `UUID` on the other.
    async fn work(&self, account: &str, title: &str) -> String {
        let pseud = id(&format!("m50-pseud-{account}-{title}"));
        let work = id(&format!("m50-work-{account}-{title}"));
        match self.db.backend() {
            Backend::Sqlite => {
                sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES (?, ?, ?, ?, datetime('now'), datetime('now'))")
                    .bind(&pseud)
                    .bind(account)
                    .bind(format!("p-{pseud}"))
                    .bind(format!("p-{pseud}"))
                    .execute(self.db.sqlite_pool().expect("sqlite"))
                    .await
                    .expect("pseud");
                sqlx::query("INSERT INTO works (id, title, owner_pseud_id, summary, visibility, lifecycle, created_at, updated_at, published_at) VALUES (?, ?, ?, 'A summary.', 'public', 'published', datetime('now'), datetime('now'), datetime('now'))")
                    .bind(&work)
                    .bind(title)
                    .bind(&pseud)
                    .execute(self.db.sqlite_pool().expect("sqlite"))
                    .await
                    .expect("work");
            }
            Backend::Postgres => {
                sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES ($1::uuid, $2::uuid, $3, $4, now(), now())")
                    .bind(&pseud)
                    .bind(account)
                    .bind(format!("p-{pseud}"))
                    .bind(format!("p-{pseud}"))
                    .execute(self.db.postgres_pool().expect("postgres"))
                    .await
                    .expect("pseud");
                sqlx::query("INSERT INTO works (id, title, owner_pseud_id, summary, visibility, lifecycle, created_at, updated_at, published_at) VALUES ($1::uuid, $2, $3::uuid, 'A summary.', 'public', 'published', now(), now(), now())")
                    .bind(&work)
                    .bind(title)
                    .bind(&pseud)
                    .execute(self.db.postgres_pool().expect("postgres"))
                    .await
                    .expect("work");
            }
        }
        work
    }

    async fn cleanup(self) {
        self.tdb.cleanup().await;
    }
}

async fn post(client: &mut TestClient, uri: &str, body: Value) -> (StatusCode, Value) {
    client.post(uri, body).await
}

/// Read the stored signal for a work, through the same store the route uses.
async fn signal(db: &Database, work_id: &str) -> lorehaven_domain::reasons::HighlightSignal {
    lorehaven_db::reasons_store::highlight_signal(db, work_id)
        .await
        .expect("highlight signal")
}

// ---------------------------------------------------------------------------
// §50.4: a kudos with a reason trains; the same kudos without one trains nothing
// ---------------------------------------------------------------------------

/// The first half of the clause: the reason is stored and reads back.
#[tokio::test]
async fn a_kudos_with_a_reason_is_stored_and_reads_back() {
    let h = Harness::new("m50-kudos-reason").await;
    let (mut client, account) = h.reader("reasons").await;
    let work = h.work(&account, "Reasoned").await;

    let (status, body) = post(
        &mut client,
        &format!("/api/v1/works/{work}/kudos"),
        json!({ "reason": "prose" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "kudos failed: {body}");
    assert_eq!(body["kudoed"], json!(true));
    assert_eq!(body["reason"], json!("prose"));

    let stored = lorehaven_db::reasons_store::kudos_reason(&h.db, &work, &account)
        .await
        .expect("read reason");
    assert_eq!(stored, Some(lorehaven_domain::reasons::Reason::Prose));

    // And it is the *reason-bearing* count that moved, not the kudos count.
    let trained = lorehaven_db::reasons_store::reason_bearing_kudos(&h.db, &work)
        .await
        .expect("count");
    assert_eq!(
        trained, 1,
        "a kudos with a reason must be countable as a signal"
    );

    h.cleanup().await;
}

/// The second half: the *same* kudos with the reason omitted is accepted and
/// trains nothing. §50.1 says a bare kudos is valid, so this is not an error --
/// it is the clause that makes the reason column worth having.
#[tokio::test]
async fn a_bare_kudos_is_accepted_and_carries_no_reason() {
    let h = Harness::new("m50-kudos-bare").await;
    let (mut client, account) = h.reader("bare").await;
    let work = h.work(&account, "Bare").await;

    let (status, body) = post(
        &mut client,
        &format!("/api/v1/works/{work}/kudos"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a bare kudos must be valid: {body}");
    assert_eq!(body["kudoed"], json!(true));
    assert_eq!(
        body["reason"],
        Value::Null,
        "an absent reason must read back as absent, never defaulted"
    );

    let stored = lorehaven_db::reasons_store::kudos_reason(&h.db, &work, &account)
        .await
        .expect("read reason");
    assert_eq!(stored, None, "a bare kudos must not acquire a reason");

    let trained = lorehaven_db::reasons_store::reason_bearing_kudos(&h.db, &work)
        .await
        .expect("count");
    assert_eq!(trained, 0, "a kudos with no reason must train nothing");

    h.cleanup().await;
}

/// §50.4: a reason outside the fixed set is refused, and the error names the set.
#[tokio::test]
async fn a_reason_outside_the_fixed_set_is_refused() {
    let h = Harness::new("m50-bad-reason").await;
    let (mut client, account) = h.reader("bogus").await;
    let work = h.work(&account, "Bogus").await;

    let (status, body) = post(
        &mut client,
        &format!("/api/v1/works/{work}/kudos"),
        json!({ "reason": "vibes" }),
    )
    .await;
    assert!(
        status.is_client_error(),
        "an unknown reason must be refused, got {status}: {body}"
    );
    let message = body.to_string();
    assert!(message.contains("vibes"), "{message}");
    assert!(
        message.contains("prose"),
        "the refusal should list what is allowed: {message}"
    );

    // And nothing was stored, so a refused reason cannot later train anything.
    let trained = lorehaven_db::reasons_store::reason_bearing_kudos(&h.db, &work)
        .await
        .expect("count");
    assert_eq!(trained, 0, "a refused reason must leave nothing behind");

    h.cleanup().await;
}

/// §50.3: the note is stored and never becomes a reason. A free-text field that
/// trained the profile would be an unreviewable store of prose, and the clause
/// that keeps it out of the aggregate is the one worth pinning.
#[tokio::test]
async fn a_note_is_stored_but_does_not_become_a_reason() {
    let h = Harness::new("m50-note").await;
    let (mut client, account) = h.reader("noter").await;
    let work = h.work(&account, "Noted").await;

    let (status, body) = post(
        &mut client,
        &format!("/api/v1/works/{work}/kudos"),
        json!({ "note": "the second chapter undoes the first" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a note alone is a valid bare kudos: {body}"
    );
    assert_eq!(body["reason"], Value::Null);

    let trained = lorehaven_db::reasons_store::reason_bearing_kudos(&h.db, &work)
        .await
        .expect("count");
    assert_eq!(
        trained, 0,
        "free text must never be aggregated into a signal"
    );

    h.cleanup().await;
}

// ---------------------------------------------------------------------------
// §50.1 / §50.4: highlights
// ---------------------------------------------------------------------------

/// The door exists and is reachable.
#[tokio::test]
async fn a_highlight_requires_a_session() {
    let h = Harness::new("m50-highlight-anon").await;
    let (_client, account) = h.reader("owner").await;
    let work = h.work(&account, "Public").await;

    let mut anon = h.client();
    let (status, _body) = post(
        &mut anon,
        &format!("/api/v1/works/{work}/highlights"),
        json!({ "start_offset": 0, "end_offset": 40, "reason": "prose" }),
    )
    .await;
    assert!(
        status.is_client_error() || status == StatusCode::UNAUTHORIZED,
        "an anonymous highlight must not be accepted, got {status}"
    );

    h.cleanup().await;
}

/// A highlight is a reason about a SPAN, so the reason is required outright —
/// unlike a kudos, where §50.1 makes it optional.
#[tokio::test]
async fn a_highlight_requires_a_reason_and_a_span() {
    let h = Harness::new("m50-highlight-validation").await;
    let (mut client, account) = h.reader("highlighter").await;
    let work = h.work(&account, "Validated").await;
    let uri = format!("/api/v1/works/{work}/highlights");

    let (status, body) = post(
        &mut client,
        &uri,
        json!({ "start_offset": 0, "end_offset": 40 }),
    )
    .await;
    assert!(
        status.is_client_error(),
        "a highlight with no reason must be refused, got {status}: {body}"
    );

    let (status, body) = post(
        &mut client,
        &uri,
        json!({ "start_offset": 40, "end_offset": 40, "reason": "prose" }),
    )
    .await;
    assert!(
        status.is_client_error(),
        "an empty span must be refused, got {status}: {body}"
    );

    let (status, body) = post(
        &mut client,
        &uri,
        json!({ "start_offset": 0, "end_offset": 40, "reason": "vibes" }),
    )
    .await;
    assert!(
        status.is_client_error(),
        "an unknown reason must be refused, got {status}: {body}"
    );

    h.cleanup().await;
}

/// §50.4: "fifty highlights move its gravity by the weight of one signal."
///
/// The literal claim, at literal scale, so the clause cannot pass on a
/// convenient small number.
#[tokio::test]
async fn many_highlights_move_a_work_by_one_signal() {
    let h = Harness::new("m50-highlight-fifty").await;
    let (mut client, account) = h.reader("quoter").await;
    let work = h.work(&account, "Quoted").await;
    let uri = format!("/api/v1/works/{work}/highlights");

    let before = signal(&h.db, &work).await;
    assert_eq!(
        before.gravity_weight(),
        0.0,
        "a work with no highlights carries no highlight signal"
    );

    for n in 0..50i64 {
        let (status, body) = post(
            &mut client,
            &uri,
            json!({
                "start_offset": n * 10,
                "end_offset": n * 10 + 8,
                "reason": "prose",
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "highlight {n} failed: {body}");
    }

    let after = signal(&h.db, &work).await;
    assert_eq!(after.highlights, 50, "all fifty should be stored");
    assert_eq!(after.readers, 1, "one reader highlighted them all");
    assert_eq!(
        after.gravity_weight(),
        1.0,
        "fifty highlights must move the work by one signal's weight, not fifty"
    );

    h.cleanup().await;
}

/// One reader's fifty highlights and one reader's single highlight are the same
/// signal. This is the test that fails if `COUNT(*)` ever creeps back in.
#[tokio::test]
async fn one_readers_many_highlights_count_as_one_reader() {
    let h = Harness::new("m50-highlight-one-reader").await;
    let (mut client, account) = h.reader("single").await;
    let busy = h.work(&account, "Busy").await;
    let quiet = h.work(&account, "Quiet").await;

    for n in 0..10i64 {
        post(
            &mut client,
            &format!("/api/v1/works/{busy}/highlights"),
            json!({ "start_offset": n * 10, "end_offset": n * 10 + 8, "reason": "pacing" }),
        )
        .await;
    }
    post(
        &mut client,
        &format!("/api/v1/works/{quiet}/highlights"),
        json!({ "start_offset": 0, "end_offset": 8, "reason": "pacing" }),
    )
    .await;

    let busy_signal = signal(&h.db, &busy).await;
    let quiet_signal = signal(&h.db, &quiet).await;
    assert_eq!(busy_signal.highlights, 10);
    assert_eq!(quiet_signal.highlights, 1);
    assert_eq!(
        busy_signal.gravity_weight(),
        quiet_signal.gravity_weight(),
        "ten highlights from one reader and one highlight from that reader are the same signal"
    );

    h.cleanup().await;
}

/// A highlight is a reference to a span of text that no longer exists once the
/// work is deleted, so it must not outlive the work — §50.1's last clause, and the
/// one that would otherwise let the corpus of "what readers respond to" outlive the
/// works it describes.
#[tokio::test]
async fn deleting_a_work_deletes_its_highlights() {
    let h = Harness::new("m50-highlight-cascade").await;
    let (mut client, account) = h.reader("cascader").await;
    let work = h.work(&account, "Doomed").await;

    let (status, body) = post(
        &mut client,
        &format!("/api/v1/works/{work}/highlights"),
        json!({ "start_offset": 0, "end_offset": 40, "reason": "prose" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "highlight failed: {body}");
    assert_eq!(signal(&h.db, &work).await.readers, 1);

    // `ON DELETE CASCADE` on `work_highlights.work_id` is the mechanism; this
    // asserts the behaviour rather than the constraint, because the constraint is
    // declared twice and only one of the two files is under test here.
    let sql = match h.db.backend() {
        Backend::Sqlite => "DELETE FROM works WHERE id = ?",
        Backend::Postgres => "DELETE FROM works WHERE id::text = $1",
    };
    // `rows_affected()` read inside each arm rather than after: `SqliteQueryResult`
    // and `PgQueryResult` are unrelated types, so one `match` cannot bind both.
    let deleted = match h.db.backend() {
        Backend::Sqlite => sqlx::query(sql)
            .bind(&work)
            .execute(h.db.sqlite_pool().expect("sqlite"))
            .await
            .expect("delete work")
            .rows_affected(),
        Backend::Postgres => sqlx::query(sql)
            .bind(&work)
            .execute(h.db.postgres_pool().expect("postgres"))
            .await
            .expect("delete work")
            .rows_affected(),
    };
    assert_eq!(deleted, 1, "the work should have been deleted");

    assert_eq!(
        signal(&h.db, &work).await.highlights,
        0,
        "a deleted work's highlights must go with it, or the corpus outlives the works"
    );

    h.cleanup().await;
}
