//! Acceptance: the tasting menu (spec §49.5, M45-19; plan
//! `m45-phase1-taste-signal.md` step 3).
//!
//! The mechanism existed and was unit-tested in `crates/db/src/tasting.rs`;
//! **no route called it**. These tests drive the door, because a selector
//! nothing reaches is the exact shape `docs/goal.md` names as a definition of
//! not-complete ("a unit test on a function nothing calls").
//!
//! §49.5's clauses, and the test that pins each:
//!
//! | clause | test |
//! |---|---|
//! | chosen by uncertainty, not randomly or by popularity | `the_queue_offers_the_least_certain_works_first` |
//! | a 300-word passage is a sample, reproducible by coordinates | `a_sample_carries_the_offset_that_reproduces_it` |
//! | reason tags required, never optional | `a_rating_without_a_reason_is_refused` |
//! | a decline is a negative carrying its reason, not discarded | `a_declined_sample_is_recorded_and_trains_the_profile` |
//! | sampling bounded per session | `a_session_gets_at_most_its_quota` |
//! | the selector's own uncertainty is recorded, not re-derived | `the_recorded_uncertainty_is_the_selectors_own` |
//! | the queue is reachable at all | `the_queue_is_authenticated` |

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
    // Honoured, so this file runs on whichever backend the selector names.
    config.database = match std::env::var("LOREHAVEN_TEST_PG_URL") {
        Ok(url) => lorehaven_db::DatabaseConfig::new(url),
        Err(_) => lorehaven_db::DatabaseConfig::new(format!(
            "sqlite://{}/lorehaven.sqlite?mode=rwc",
            dir.display()
        )),
    };
    // The rate-limit buckets are process-global at 127.0.0.1, so the development
    // defaults are exhausted by neighbouring suites long before this file's own
    // requests finish. The tasting queue is `RouteClass::Write`, so the write
    // bucket is the one that matters.
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

/// One scratch instance, a router per client, and the database behind them.
struct Harness {
    /// Held so the scratch database lives as long as the harness.
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

    /// A signed-in reader, and its account id.
    ///
    /// `sign_in_as` rather than register-then-login, because the helper clears
    /// the previous identity's cookies first — so two readers inside one test
    /// cannot silently answer as each other, which is the failure the
    /// `a_sample_is_answered_once_and_only_by_its_owner` test is about.
    async fn reader(&self, handle: &str) -> (TestClient, String) {
        let mut client = self.client();
        let email = format!("{handle}@example.test");
        let account = test_support::sign_in_as(&mut client, &self.tdb, &email, handle).await;
        (client, account)
    }

    /// One published, public work owned by `account`, with an optional tag.
    ///
    /// `confirmation` is a parameter because §49.2 makes it the whole difference
    /// between a tag that moves anybody's ranking and one that does not, and
    /// `a_work_whose_only_tag_is_unconfirmed_trains_nothing` is a real case
    /// rather than a hypothetical.
    ///
    /// Every statement is spelled per engine. `datetime('now')` is SQLite-only,
    /// `now()` is PostgreSQL-only, and the id columns are `TEXT` on one engine
    /// and `UUID` on the other — so a single spelling cannot satisfy both, and
    /// this is the shape `arena.rs` established.
    async fn work(&self, account: &str, title: &str, tag: Option<(&str, &str)>) -> String {
        // `test_support::id` is a **deterministic** hash of its label, so two
        // calls with the same label return the same id. A fixture that seeds
        // several works therefore needs a label that varies -- the title is
        // unique per work, and the pseud is keyed on the work so a reader who
        // seeds eight works does not trip `pseuds_handle_normalized`.
        let work = id(&format!("tasting-work-{title}"));
        let pseud = id(&format!("tasting-pseud-{title}"));
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

        if let Some((tag_name, confirmation)) = tag {
            // `taxonomy_nodes.norm` is NOT NULL and (kind, norm) is UNIQUE, so a
            // node carries its own lowercase form and a second work tagged the
            // same way reuses the node rather than colliding. `work_tags.added_at`
            // is NOT NULL too, and is TEXT on **both** engines (verified in
            // 0011_taxonomy.sql on each arm) -- so the timestamp is the house
            // RFC 3339 string, not `now()`, which is PostgreSQL-only.
            // Keyed on the tag, so seeding several works on the SAME tag reuses
            // one node via the conflict clause instead of tripping the id primary
            // key -- `id` is a deterministic hash, so a constant label would hand
            // out the same id every call.
            let node = id(&format!("tasting-node-{tag_name}"));
            let stamp = lorehaven_db::identity::now_rfc3339();
            let norm = tag_name.to_lowercase();
            match self.db.backend() {
                Backend::Sqlite => {
                    sqlx::query("INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES (?, 'tag', ?, ?, ?) ON CONFLICT (kind, norm) DO NOTHING")
                        .bind(&node)
                        .bind(tag_name)
                        .bind(&norm)
                        .bind(&stamp)
                        .execute(self.db.sqlite_pool().expect("sqlite"))
                        .await
                        .expect("node");
                    // The node id is looked up rather than reused from `node`,
                    // because on a conflict the insert was a no-op and `node` is
                    // an id that does not exist.
                    sqlx::query("INSERT INTO work_tags (work_id, node_id, weight, added_at, confirmation) VALUES (?, (SELECT id FROM taxonomy_nodes WHERE kind = 'tag' AND norm = ?), 10, ?, ?)")
                        .bind(&work)
                        .bind(&norm)
                        .bind(&stamp)
                        .bind(confirmation)
                        .execute(self.db.sqlite_pool().expect("sqlite"))
                        .await
                        .expect("work_tag");
                }
                Backend::Postgres => {
                    sqlx::query("INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES ($1, 'tag', $2, $3, $4) ON CONFLICT (kind, norm) DO NOTHING")
                        .bind(&node)
                        .bind(tag_name)
                        .bind(&norm)
                        .bind(&stamp)
                        .execute(self.db.postgres_pool().expect("postgres"))
                        .await
                        .expect("node");
                    sqlx::query("INSERT INTO work_tags (work_id, node_id, weight, added_at, confirmation) VALUES ($1::uuid, (SELECT id FROM taxonomy_nodes WHERE kind = 'tag' AND norm = $2), 10, $3, $4)")
                        .bind(&work)
                        .bind(&norm)
                        .bind(&stamp)
                        .bind(confirmation)
                        .execute(self.db.postgres_pool().expect("postgres"))
                        .await
                        .expect("work_tag");
                }
            }
        }
        work
    }

    /// Force a work's publication time, so "newest first" and "most uncertain
    /// first" can be made to disagree deliberately.
    async fn published_at(&self, work: &str, when: &str) {
        // `works.published_at` is TEXT on **both** engines -- read from
        // 0003_works.sql rather than assumed, because an earlier draft of this
        // file asserted a TIMESTAMPTZ asymmetry that does not exist. The only
        // per-engine difference is the id cast.
        let sql = match self.db.backend() {
            Backend::Sqlite => {
                format!("UPDATE works SET published_at = '{when}' WHERE id = '{work}'")
            }
            Backend::Postgres => {
                format!("UPDATE works SET published_at = '{when}' WHERE id = '{work}'::uuid")
            }
        };
        match self.db.backend() {
            Backend::Sqlite => {
                sqlx::query(&sql)
                    .execute(self.db.sqlite_pool().expect("sqlite"))
                    .await
                    .expect("published_at");
            }
            Backend::Postgres => {
                sqlx::query(&sql)
                    .execute(self.db.postgres_pool().expect("postgres"))
                    .await
                    .expect("published_at");
            }
        }
    }

    /// Seed a settled reader weight, so the selector has a "known" dimension.
    async fn weigh(&self, account: &str, dimension: &str, weight: f64) {
        match self.db.backend() {
            Backend::Sqlite => {
                let stamp = id("ts");
                sqlx::query("INSERT INTO arena_weights (id, account_id, dimension_key, weight, elo_rating, matches_played, created_at, updated_at) VALUES (?, ?, ?, ?, 1500.0, 10, ?, ?)")
                    .bind(&stamp)
                    .bind(account)
                    .bind(dimension)
                    .bind(weight)
                    .bind(&stamp)
                    .bind(&stamp)
                    .execute(self.db.sqlite_pool().expect("sqlite"))
                    .await
                    .expect("arena weight");
            }
            Backend::Postgres => {
                sqlx::query("INSERT INTO arena_weights (account_id, dimension_key, weight, elo_rating, matches_played) VALUES ($1::uuid, $2, $3, 1500.0, 10)")
                    .bind(account)
                    .bind(dimension)
                    .bind(weight)
                    .execute(self.db.postgres_pool().expect("postgres"))
                    .await
                    .expect("arena weight");
            }
        }
    }

    /// A scalar from a counting query, matched on one value.
    async fn count(&self, query: &str, value: &str) -> i64 {
        self.tdb.count_by(query, value).await
    }

    /// `uncertainty_at_draw` from one of the two tables, as a number.
    ///
    /// A method rather than an inline query because the column is REAL on both
    /// engines while `fetch_text_by` decodes `Option<String>` — so the cast has to
    /// be in the projection, and the projection has to be spelled per engine.
    /// `test_support::sql` rewrites the *placeholder* and never the cast, so
    /// writing one query and letting the helper fix it is not an option here.
    async fn uncertainty_in(&self, table: &str, key: &str, value: &str) -> Option<f64> {
        let query = match self.db.backend() {
            Backend::Sqlite => {
                format!("SELECT CAST(uncertainty_at_draw AS TEXT) FROM {table} WHERE {key} = ?")
            }
            Backend::Postgres => {
                format!("SELECT CAST(uncertainty_at_draw AS TEXT) FROM {table} WHERE {key} = $1")
            }
        };
        self.tdb
            .fetch_text_by(&query, value)
            .await
            .and_then(|v| v.parse().ok())
    }

    async fn cleanup(self) {
        self.tdb.cleanup().await;
    }
}

async fn get(client: &mut TestClient, uri: &str) -> (StatusCode, Value) {
    client.get(uri).await
}

async fn post(client: &mut TestClient, uri: &str, body: Value) -> (StatusCode, Value) {
    client.post(uri, body).await
}

/// The first sample's id, or a panic that says the queue was empty.
fn first_sample_id(body: &Value) -> String {
    body["samples"][0]["sample_id"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a sample in the queue, got {body}"))
        .to_owned()
}

/// §49.5: "Items are chosen by uncertainty (active learning), not randomly and
/// not by popularity."
///
/// The reader has settled `prose` at 0.9 and has never weighed `vampires`, so the
/// work tagged `vampires` is the one the model is least sure about and must lead
/// the queue. It is also the **oldest** of the three, so an implementation that
/// returned the candidate pool in query order — `published_at DESC` — fails this
/// rather than passing it by accident.
#[tokio::test]
async fn the_queue_offers_the_least_certain_works_first() {
    let h = Harness::new("tasting-uncertainty").await;
    let (mut client, account) = h.reader("taster").await;

    let weighed_new = h
        .work(&account, "Weighed, newest", Some(("prose", "reader")))
        .await;
    let weighed_old = h
        .work(&account, "Weighed, oldest", Some(("prose", "reader")))
        .await;
    let unweighed = h
        .work(&account, "Unweighed, middle", Some(("vampires", "reader")))
        .await;

    h.published_at(&weighed_new, "2026-01-03T00:00:00Z").await;
    h.published_at(&unweighed, "2026-01-02T00:00:00Z").await;
    h.published_at(&weighed_old, "2026-01-01T00:00:00Z").await;
    h.weigh(&account, "prose", 0.9).await;

    let (status, body) = get(&mut client, "/api/v1/tasting/queue?session_id=s1").await;
    assert_eq!(status, StatusCode::OK, "queue failed: {body}");
    let samples = body["samples"].as_array().expect("samples array");
    assert!(!samples.is_empty(), "expected a queue, got {body}");
    assert_eq!(
        samples[0]["work_id"], unweighed,
        "the work whose tag the reader has never weighed must lead; got {body}"
    );
    h.cleanup().await;
}

/// §49.5: "The 300-word passage is a sample, not a summary", and §49.7 requires
/// coordinates be reproducible. The card must carry the offset and the stored
/// row must carry the same one — a card and a row that disagree would make the
/// sample unreproducible while appearing to be fine.
#[tokio::test]
async fn a_sample_carries_the_offset_that_reproduces_it() {
    let h = Harness::new("tasting-offset").await;
    let (mut client, account) = h.reader("sampler").await;
    let _work = h
        .work(&account, "With a sample", Some(("prose", "reader")))
        .await;

    let (status, body) = get(&mut client, "/api/v1/tasting/queue?session_id=s1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let sample = &body["samples"][0];
    let advertised = sample["sample_offset"]
        .as_i64()
        .expect("a sample must carry a numeric offset");
    let sample_id = first_sample_id(&body);

    // `fetch_text_by` decodes into `Option<String>`, so an INTEGER column is a
    // type error on both engines rather than a silent `None` -- the row is there
    // and the assertion fails on the decode. The count carries the comparison in
    // SQL instead, and `count_by` binds exactly one value, so the offset is
    // spliced into the predicate rather than bound.
    let matching = h
        .count(
            &format!(
                "SELECT COUNT(*) FROM tasting_samples WHERE id = '{sample_id}' \
                      AND sample_offset = {advertised}"
            ),
            "",
        )
        .await;
    assert_eq!(
        matching, 1,
        "the stored sample must carry the offset the card advertised, or the \
         passage cannot be reproduced from its coordinates"
    );
    h.cleanup().await;
}

/// §49.5: "Reason tags are required, not optional … a bare rating teaches almost
/// nothing."
///
/// Four refusals, because the schema makes a reason unrepresentable and the route
/// has to agree with it: no reason field, an unknown reason, free text on a
/// reason that does not take it, and the free-text reason with no text. Then the
/// row count, which is the assertion that matters — a refusal that wrote a row
/// would have collected exactly the ratings §49.5 says teach nothing.
#[tokio::test]
async fn a_rating_without_a_reason_is_refused() {
    let h = Harness::new("tasting-reason").await;
    let (mut client, account) = h.reader("reasonless").await;
    let _work = h
        .work(&account, "Needs a reason", Some(("prose", "reader")))
        .await;

    let (_, body) = get(&mut client, "/api/v1/tasting/queue?session_id=s1").await;
    let sample_id = first_sample_id(&body);

    let attempts = [
        // No reason at all.
        json!({"sample_id": sample_id, "verdict": "like", "session_id": "s1"}),
        // A reason outside the enumerated set.
        json!({"sample_id": sample_id, "verdict": "like", "reason": "vibes", "session_id": "s1"}),
        // Free text on a reason that does not carry it.
        json!({"sample_id": sample_id, "verdict": "like", "reason": "prose",
               "free_text": "actually the pacing", "session_id": "s1"}),
        // The free-text reason, with nothing to refer to.
        json!({"sample_id": sample_id, "verdict": "dislike", "reason": "not_for_me",
               "session_id": "s1"}),
    ];
    for attempt in attempts {
        let (status, _) = post(&mut client, "/api/v1/tasting/respond", attempt.clone()).await;
        assert!(
            status.is_client_error(),
            "this rating must be refused, got {status} for {attempt}"
        );
    }

    let rows = h
        .count(
            "SELECT COUNT(*) FROM tasting_responses WHERE sample_id = ?",
            &sample_id,
        )
        .await;
    assert_eq!(rows, 0, "a refused rating must leave no response row");
    h.cleanup().await;
}

/// §49.5: "A declined sample is recorded as a negative with its reason, not
/// discarded", and §49.7: "No sample is discarded for being a surprise."
///
/// The decline must be visible in the reader's own profile surface **and** must
/// have moved the weight it named. A decline recorded and then invisible is the
/// discarding the clause forbids wearing a database row.
#[tokio::test]
async fn a_declined_sample_is_recorded_and_trains_the_profile() {
    let h = Harness::new("tasting-decline").await;
    let (mut client, account) = h.reader("decliner").await;
    let _work = h
        .work(&account, "Not for me", Some(("prose", "reader")))
        .await;

    let (_, body) = get(&mut client, "/api/v1/tasting/queue?session_id=s1").await;
    let sample_id = first_sample_id(&body);

    let (status, body) = post(
        &mut client,
        "/api/v1/tasting/respond",
        json!({"sample_id": sample_id, "verdict": "dislike", "reason": "prose",
               "session_id": "s1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a decline must be accepted: {body}");
    assert_eq!(body["recorded"], json!(true), "{body}");

    let (status, profile) = get(&mut client, "/api/v1/tasting/responses").await;
    assert_eq!(status, StatusCode::OK, "profile failed: {profile}");
    assert_eq!(
        profile["declines"],
        json!(1),
        "a decline must be counted as a decline, not dropped: {profile}"
    );
    let responses = profile["responses"].as_array().expect("responses");
    assert_eq!(responses.len(), 1, "{profile}");
    assert_eq!(responses[0]["verdict"], json!("dislike"), "{profile}");
    assert_eq!(responses[0]["reason"], json!("prose"), "{profile}");

    // The **sign** is the assertion, and it is a separate one from the existence
    // check on purpose. A mutation that makes a dislike raise the weight instead
    // of lowering it still leaves a weight for `prose`, so "a weight exists"
    // survives it; only a negative value distinguishes a negative from a positive.
    // §49.5's clause is that a decline is "recorded as a negative", and a
    // positive weight is not a negative however present it is.
    let weights = lorehaven_db::ranking::TagWeights::for_reader(&h.db, &account)
        .await
        .expect("weights readable");
    let weight = weights.weight_of("prose");
    assert!(
        weight < 0.0,
        "a decline on `prose` must leave a NEGATIVE weight; got {weight}"
    );

    // And a like on the same dimension moves it the other way, so the two
    // verdicts are distinguishable rather than both being "a weight appeared".
    let _liked = h
        .work(&account, "Liked instead", Some(("pacing", "reader")))
        .await;
    let (_, queue) = get(&mut client, "/api/v1/tasting/queue?session_id=s2").await;
    let sample = queue["samples"]
        .as_array()
        .expect("samples")
        .iter()
        .find(|s| s["work_id"] == _liked)
        .expect("the liked work must be offered");
    let (status, _) = post(
        &mut client,
        "/api/v1/tasting/respond",
        json!({"sample_id": sample["sample_id"], "verdict": "like", "reason": "pacing",
               "session_id": "s2"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let after = lorehaven_db::ranking::TagWeights::for_reader(&h.db, &account)
        .await
        .expect("weights readable")
        .weight_of("pacing");
    assert!(
        after > 0.0,
        "a like must leave a POSITIVE weight; got {after} — so a decline and a \
         like are not the same signal"
    );
    h.cleanup().await;
}

/// §49.5: "Sampling is bounded per session. A calibration queue that can consume
/// the whole of a reading session is a chore, and a chore gets abandoned."
///
/// Two halves: the request cannot raise the cap, and an exhausted session gets
/// nothing on a second call.
#[tokio::test]
async fn a_session_gets_at_most_its_quota() {
    let h = Harness::new("tasting-quota").await;
    let (mut client, account) = h.reader("quota").await;
    for i in 0..8 {
        let _ = h
            .work(&account, &format!("Work {i}"), Some(("prose", "reader")))
            .await;
    }

    let (status, body) = get(&mut client, "/api/v1/tasting/queue?session_id=s1&limit=99").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let limit = body["session_limit"].as_u64().expect("session_limit") as usize;
    let offered = body["samples"].as_array().expect("samples").len();
    assert!(
        offered <= limit && limit <= lorehaven_db::tasting::SESSION_LIMIT,
        "the queue must clamp to the per-session cap; offered {offered}, limit {limit}"
    );

    for sample in body["samples"].as_array().expect("samples") {
        let _ = post(
            &mut client,
            "/api/v1/tasting/respond",
            json!({"sample_id": sample["sample_id"], "verdict": "like",
                   "reason": "prose", "session_id": "s1"}),
        )
        .await;
    }
    let (status, body) = get(&mut client, "/api/v1/tasting/queue?session_id=s1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["samples"].as_array().expect("samples").len(),
        0,
        "an exhausted session gets nothing more, got {body}"
    );
    assert!(
        body["answered_in_session"].as_i64().unwrap_or(0) > 0,
        "the response must report the session's answered count, got {body}"
    );
    h.cleanup().await;
}

/// A sample is answered once, and only by the reader it was drawn for. Both
/// refusals matter: a second rating would make the recorded signal depend on how
/// many times a button was pressed, and another reader's sample must be
/// indistinguishable from one that does not exist.
#[tokio::test]
async fn a_sample_is_answered_once_and_only_by_its_owner() {
    let h = Harness::new("tasting-once").await;
    let (mut owner, account) = h.reader("owner1").await;
    let _w = h.work(&account, "One", Some(("prose", "reader"))).await;
    let _w2 = h.work(&account, "Two", Some(("angst", "reader"))).await;

    let (_, body) = get(&mut owner, "/api/v1/tasting/queue?session_id=s1").await;
    let sample_id = first_sample_id(&body);

    // Another reader, signed in on their own client. `TestClient::new` on the same
    // router is a distinct cookie jar, so this is a genuinely separate session.
    let (mut stranger, _stranger_account) = h.reader("owner2").await;
    let (status, _) = post(
        &mut stranger,
        "/api/v1/tasting/respond",
        json!({"sample_id": sample_id, "verdict": "like", "reason": "prose",
               "session_id": "s1"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another reader's sample must answer 404, not 403 — a 403 confirms it exists"
    );

    let (status, _) = post(
        &mut owner,
        "/api/v1/tasting/respond",
        json!({"sample_id": sample_id, "verdict": "like", "reason": "prose",
               "session_id": "s1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = post(
        &mut owner,
        "/api/v1/tasting/respond",
        json!({"sample_id": sample_id, "verdict": "dislike", "reason": "prose",
               "session_id": "s1"}),
    )
    .await;
    assert!(
        status.is_client_error(),
        "a second rating of one sample must be refused, got {status}"
    );

    let rows = h
        .count(
            "SELECT COUNT(*) FROM tasting_responses WHERE sample_id = ?",
            &sample_id,
        )
        .await;
    assert_eq!(rows, 1, "exactly one response row per sample, got {rows}");
    h.cleanup().await;
}

/// Migration 0100's partial unique index, exercised on the path it actually
/// protects.
///
/// A work that is **offered but not yet answered** is still open, and it is
/// reachable again: `candidate_works` filters on *answered* works, so an open
/// sample does not remove its work from the pool. Without the UNIQUE index,
/// `build_queue`'s `INSERT OR IGNORE` has no constraint to bite on, silently
/// inserts a duplicate, and the reader is holding two open samples of one work.
///
/// The existing "not offered again" test cannot see this, which is exactly why
/// dropping the index to a plain `CREATE INDEX` left the suite green.
///
/// Pinned at the **database** level as well as the API level: a duplicate open
/// sample is a schema violation whether or not a route produced it.
#[tokio::test]
async fn one_work_cannot_have_two_open_samples() {
    let h = Harness::new("tasting-one-open").await;
    let (mut client, account) = h.reader("one-open").await;
    let work = h
        .work(&account, "Offered twice", Some(("prose", "reader")))
        .await;

    // Session 1: drawn, left unanswered.
    let (status, first) = get(&mut client, "/api/v1/tasting/queue?session_id=s1").await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(
        first["samples"].as_array().expect("samples").len(),
        1,
        "expected exactly one sample: {first}"
    );

    // Session 2: the same work is still a candidate, because it was never
    // answered -- and the index must refuse a second open row.
    let (status, second) = get(&mut client, "/api/v1/tasting/queue?session_id=s2").await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let reoffered = second["samples"]
        .as_array()
        .expect("samples")
        .iter()
        .filter(|s| s["work_id"] == work)
        .count();
    assert_eq!(
        reoffered, 0,
        "a work with an open sample must not be offered again in another session: \
         {second}"
    );

    // And at the schema level: exactly one open sample for this (account, work).
    //
    // `work_id` is **UUID** on PostgreSQL and TEXT on SQLite (0099), and
    // `count_by` binds a text value — so the value is cast per engine here. The
    // first draft omitted the cast and this query failed on PostgreSQL with
    // "operator does not exist: uuid = text" while passing on SQLite, which is the
    // dialect trap in its purest form: a test assertion is as engine-dependent as
    // the code it checks.
    let open_rows = match h.db.backend() {
        Backend::Sqlite => h
            .count(
                "SELECT COUNT(*) FROM tasting_samples WHERE work_id = ? AND answered_at IS NULL",
                &work,
            )
            .await,
        Backend::Postgres => h
            .count(
                "SELECT COUNT(*) FROM tasting_samples WHERE work_id = ?::uuid AND answered_at IS NULL",
                &work,
            )
            .await,
    };
    assert_eq!(
        open_rows, 1,
        "there must be exactly one open sample for this work, got {open_rows}"
    );
    h.cleanup().await;
}

/// The one-open-sample invariant, which is what migration 0100's partial unique
/// index exists to express and which `record_response` maintains by stamping
/// `answered_at` in the same statement that inserts the response.
///
/// A response whose sample is still open means the index is not seeing the
/// queue's own state, and a second draw of the same work becomes possible.
#[tokio::test]
async fn a_response_and_its_samples_answered_flag_never_disagree() {
    let h = Harness::new("tasting-flag").await;
    let (mut client, account) = h.reader("flagger").await;
    let _w = h.work(&account, "Flagged", Some(("prose", "reader"))).await;

    let (_, body) = get(&mut client, "/api/v1/tasting/queue?session_id=s1").await;
    let sample_id = first_sample_id(&body);
    let (status, _) = post(
        &mut client,
        "/api/v1/tasting/respond",
        json!({"sample_id": sample_id, "verdict": "like", "reason": "prose",
               "session_id": "s1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let still_open = h
        .count(
            "SELECT COUNT(*) FROM tasting_samples WHERE id = ? AND answered_at IS NOT NULL",
            &sample_id,
        )
        .await;
    assert_eq!(
        still_open, 1,
        "an answered sample must be marked answered, which is what frees the work \
         for a later session"
    );
    h.cleanup().await;
}

/// §49.2 meets §49.5: a work whose only tag is **unconfirmed** has no countable
/// dimension, so the response is recorded and trains nothing.
///
/// This is what makes the "recorded but trains nothing" response an honest
/// outcome rather than a hedge — and it is the exact result 0099's
/// `unconfirmed` default was chosen to produce.
#[tokio::test]
async fn a_work_whose_only_tag_is_unconfirmed_trains_nothing() {
    let h = Harness::new("tasting-unconfirmed").await;
    let (mut client, account) = h.reader("unconfirmed").await;
    let _w = h
        .work(&account, "Unconfirmed tag", Some(("prose", "unconfirmed")))
        .await;

    let (_, body) = get(&mut client, "/api/v1/tasting/queue?session_id=s1").await;
    let sample_id = first_sample_id(&body);
    let (status, body) = post(
        &mut client,
        "/api/v1/tasting/respond",
        json!({"sample_id": sample_id, "verdict": "like", "reason": "prose",
               "session_id": "s1"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the rating is still recorded even when it trains nothing: {body}"
    );
    assert!(
        body["contribution"]
            .as_str()
            .is_some_and(|c| c.contains("no countable tag")),
        "the response must say the rating trained nothing, got {body}"
    );

    let weights = lorehaven_db::ranking::TagWeights::for_reader(&h.db, &account)
        .await
        .expect("weights readable");
    assert!(
        !weights.has("prose"),
        "an unconfirmed tag must not move any weight, got {:?}",
        weights.weights
    );
    h.cleanup().await;
}

/// A work the reader has already answered is not offered again — in this session
/// or any later one, so the second request uses a **different** session id and
/// the per-session bound cannot be what stops it.
#[tokio::test]
async fn an_answered_work_is_not_offered_again() {
    let h = Harness::new("tasting-no-repeat").await;
    let (mut client, account) = h.reader("repeater").await;
    for i in 0..3 {
        let _ = h
            .work(&account, &format!("Once {i}"), Some(("prose", "reader")))
            .await;
    }

    let (_, body) = get(&mut client, "/api/v1/tasting/queue?session_id=s1").await;
    let mut answered = Vec::new();
    for sample in body["samples"].as_array().expect("samples") {
        answered.push(sample["work_id"].as_str().unwrap_or_default().to_owned());
        let _ = post(
            &mut client,
            "/api/v1/tasting/respond",
            json!({"sample_id": sample["sample_id"], "verdict": "like",
                   "reason": "prose", "session_id": "s1"}),
        )
        .await;
    }
    assert!(!answered.is_empty(), "expected samples to answer: {body}");

    let (_, body) = get(&mut client, "/api/v1/tasting/queue?session_id=s2").await;
    let fresh: Vec<String> = body["samples"]
        .as_array()
        .expect("samples")
        .iter()
        .map(|s| s["work_id"].as_str().unwrap_or_default().to_owned())
        .collect();
    for work in &answered {
        assert!(
            !fresh.contains(work),
            "work {work} was already answered and must not come back: {body}"
        );
    }
    h.cleanup().await;
}

/// The selector's own uncertainty travels with the sample and is inherited by
/// the response — the number §49.5's acceptance criterion is checked against.
///
/// Equality, not a range: a re-derivation from the reader's weights at answer
/// time would be very close and would pass a tolerance, which is precisely the
/// bug this rules out.
#[tokio::test]
async fn the_recorded_uncertainty_is_the_selectors_own() {
    let h = Harness::new("tasting-uncertainty-recorded").await;
    let (mut client, account) = h.reader("recorded").await;
    let _w = h
        .work(&account, "Recorded", Some(("vampires", "reader")))
        .await;
    h.weigh(&account, "prose", 0.7).await;

    let (_, body) = get(&mut client, "/api/v1/tasting/queue?session_id=s1").await;
    let sample = &body["samples"][0];
    let offered = sample["uncertainty"]
        .as_f64()
        .expect("the card must carry the selector's uncertainty");
    let sample_id = first_sample_id(&body);

    let (status, _) = post(
        &mut client,
        "/api/v1/tasting/respond",
        json!({"sample_id": sample_id, "verdict": "like", "reason": "prose",
               "session_id": "s1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Read the response's own number, and the sample's, and require all three to
    // be the same value. Both are read as **text** — `fetch_text_by` decodes into
    // `Option<String>` and the column is REAL, so selecting it raw is a decode
    // error on both engines. The cast is written into the projection, which is
    // where the engine-specific spelling belongs; `test_support::sql` rewrites the
    // placeholder but never the cast, so both arms are given their own.
    let responses = h
        .uncertainty_in("tasting_responses", "sample_id", &sample_id)
        .await
        .expect("the response's recorded uncertainty must be readable");
    let on_sample = h
        .uncertainty_in("tasting_samples", "id", &sample_id)
        .await
        .expect("the sample's recorded uncertainty must be readable");
    assert_eq!(
        on_sample, offered,
        "the sample must store the number the card advertised"
    );
    assert_eq!(
        responses, offered,
        "the response must inherit the selector's number, not re-derive it"
    );
    h.cleanup().await;
}

/// Nothing on this surface serves prose. §7.7 forbids a surface that varies body
/// visibility, and a calibration card carrying the body would be one — the
/// passage is fetched through the normal read path, under the work's own gates.
#[tokio::test]
async fn the_queue_never_carries_prose() {
    let h = Harness::new("tasting-no-prose").await;
    let (mut client, account) = h.reader("prose-free").await;
    let _w = h
        .work(&account, "No prose here", Some(("prose", "reader")))
        .await;

    let (status, body) = get(&mut client, "/api/v1/tasting/queue?session_id=s1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rendered = body.to_string();
    for forbidden in ["plain_text", "sanitized_html", "document_json", "body_text"] {
        assert!(
            !rendered.contains(forbidden),
            "the calibration queue must not carry {forbidden}: {rendered}"
        );
    }
    h.cleanup().await;
}

/// The queue is authenticated. A calibration state is a profile fact, so an
/// anonymous caller must be refused on both doors — and the POST is checked
/// separately because "the GET is behind a session" says nothing about the write.
#[tokio::test]
async fn the_queue_is_authenticated() {
    let h = Harness::new("tasting-auth").await;
    // A fresh client with no cookies, which is what "anonymous" means here.
    let mut anonymous = h.client();

    let (status, _) = get(&mut anonymous, "/api/v1/tasting/queue?session_id=s1").await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "an anonymous reader of the queue must be refused, got {status}"
    );

    // The write door is checked separately, because "the GET is behind a session"
    // says nothing about the POST -- and the POST is the one that moves weights.
    let (status, _) = post(
        &mut anonymous,
        "/api/v1/tasting/respond",
        json!({"sample_id": "x", "verdict": "like", "reason": "prose",
               "session_id": "s"}),
    )
    .await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "an anonymous POST must be refused, got {status}"
    );
    h.cleanup().await;
}

/// A missing or empty `session_id` is refused rather than defaulted.
///
/// §49.5's per-session bound is enforced against a session the client names. A
/// server-minted one would be a session the client cannot resume, so a reload
/// would reset the reader's quota — and the bound would be bypassed by pressing
/// refresh, which is the cheapest possible bypass.
#[tokio::test]
async fn a_session_id_is_required_and_may_not_be_blank() {
    let h = Harness::new("tasting-session-id").await;
    let (mut client, account) = h.reader("needs-session").await;
    let _w = h
        .work(&account, "Needs a session", Some(("prose", "reader")))
        .await;

    // Two different failures, and the distinction is the point. A **missing**
    // `session_id` never reaches the handler: axum's `Query` extractor rejects it
    // with 400 before any code of ours runs. A **present but blank** one reaches
    // the handler and is refused with 422 by the field check. Both are refusals,
    // and neither may serve a queue — which is what this asserts, without
    // pretending the two paths are the same.
    for (uri, expected) in [
        ("/api/v1/tasting/queue", StatusCode::BAD_REQUEST),
        (
            "/api/v1/tasting/queue?session_id=",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/api/v1/tasting/queue?session_id=%20%20",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let (status, body) = get(&mut client, uri).await;
        assert_eq!(
            status, expected,
            "{uri} must be refused without a usable session id, got {status}: {body}"
        );
        assert!(
            body["samples"].is_null(),
            "{uri} must not serve a queue, got {body}"
        );
    }
    h.cleanup().await;
}
