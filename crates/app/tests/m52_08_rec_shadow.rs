//! Acceptance: shadow-mode evaluation for the recommender (spec §16.1a, M52-08).
//!
//! The spec requires, in one sentence, that switching rankers "is preceded by
//! shadow-mode evaluation on the same candidate sets". These tests pin what that
//! has to mean in practice, and in particular the property that makes shadow
//! mode safe at all:
//!
//! **A reader in shadow mode receives exactly what a reader in legacy mode
//! receives.** Not "something close", not "the legacy ranking recomputed under
//! new settings" — the identical feed. The evaluation is an observation made
//! alongside the served result, never an input to it.
//!
//! The unit tests at the top pin the comparison arithmetic, which is where a
//! wrong answer is most plausible and least visible: an overlap score that
//! flatters itself, a strategy that looks like it contributed when it did not.
//! The router tests below pin the wiring, because "shadow does not change the
//! feed" is a claim about the route and not about a function.
use std::path::Path;
use std::str::FromStr;

use lorehaven_app::rec_shadow::{self, Agreement};
use lorehaven_db::rec_strategy::{RecRunReport, StrategyContribution};

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// A traced registry run with one named strategy, for `compare` inputs.
fn run_with(name: &str, produced: usize, ranked: &[&str]) -> RecRunReport {
    RecRunReport {
        blended: ranked.iter().map(|s| s.to_string()).collect(),
        per_strategy: vec![StrategyContribution {
            name: name.to_owned(),
            produced,
            ranked: ranked
                .iter()
                .enumerate()
                .map(|(i, id)| (id.to_string(), 1.0 / (60.0 + i as f64 + 1.0)))
                .collect(),
        }],
    }
}

// ---------------------------------------------------------------------------
// The comparison arithmetic
// ---------------------------------------------------------------------------

#[test]
fn identical_rankings_are_neutral() {
    let list = ids(&["a", "b", "c"]);
    let report = rec_shadow::compare(&list, &list, &run_with("s", 3, &["a", "b", "c"]), 1);
    assert_eq!(report.agreement, Agreement::Identical);
    assert!(report.is_neutral());
    assert_eq!(report.overlap, 1.0);
    assert!(report.displaced.is_empty());
    assert!(report.promoted.is_empty());
}

#[test]
fn reordered_only_is_same_set_not_identical() {
    // "Same works, different order" is a distinct finding from "identical":
    // an operator switching would not change what is offered, only the order.
    let served = ids(&["a", "b", "c"]);
    let shadow = ids(&["c", "b", "a"]);
    let report = rec_shadow::compare(&served, &shadow, &run_with("s", 3, &["c", "b", "a"]), 1);
    assert_eq!(report.agreement, Agreement::SameSet);
    assert!(!report.is_neutral());
    assert!(
        report.displaced.is_empty(),
        "nothing is lost, only reordered"
    );
    assert!(report.promoted.is_empty());
}

#[test]
fn overlap_counts_positions_not_membership() {
    // Same three works, only the first position shared. Membership overlap here
    // would be 1.0 and would understate a reordering that changes the top
    // result, which is the position that matters most.
    let served = ids(&["a", "b", "c"]);
    let shadow = ids(&["a", "c", "b"]);
    let report = rec_shadow::compare(&served, &shadow, &run_with("s", 3, &["a", "c", "b"]), 1);
    assert!((report.overlap - 1.0 / 3.0).abs() < f64::EPSILON);
}

#[test]
fn a_longer_shadow_list_cannot_inflate_overlap() {
    // The trap this guards: scoring over the longer list would let a shadow
    // ranking twice the size of the legacy one score 1.0 on the positions it
    // happens to share, and an operator would read "perfect agreement".
    let served = ids(&["a"]);
    let shadow = ids(&["a", "b", "c", "d"]);
    let report = rec_shadow::compare(
        &served,
        &shadow,
        &run_with("s", 4, &["a", "b", "c", "d"]),
        1,
    );
    // Scored over the shorter list (1 position), and that position agrees.
    assert_eq!(report.overlap, 1.0);
    // But the membership is plainly different, and 3 works are new.
    assert_eq!(report.agreement, Agreement::Different);
    assert_eq!(report.promoted, ids(&["b", "c", "d"]));
}

#[test]
fn empty_lists_agree_and_one_sided_emptyness_does_not() {
    let empty: Vec<String> = Vec::new();
    let some = ids(&["a"]);
    assert_eq!(Agreement::overlap(&empty, &empty), 1.0);
    assert_eq!(Agreement::overlap(&empty, &some), 0.0);
    assert_eq!(Agreement::overlap(&some, &empty), 0.0);
    // And the classification agrees with that, rather than reporting two
    // different "agreeing" empty rankings as merely Different.
    assert_eq!(Agreement::classify(&empty, &empty), Agreement::Identical);
    assert_eq!(Agreement::classify(&some, &empty), Agreement::Different);
}

#[test]
fn displaced_and_promoted_are_the_switch_consequences() {
    let served = ids(&["a", "b", "c"]);
    let shadow = ids(&["a", "c", "d"]);
    let report = rec_shadow::compare(&served, &shadow, &run_with("s", 3, &["a", "c", "d"]), 7);
    assert_eq!(report.displaced, ids(&["b"]), "the reader would lose b");
    assert_eq!(report.promoted, ids(&["d"]), "and gain d");
    assert_eq!(report.sample, 7, "the count travels with the report");
}

#[test]
fn a_strategy_that_produced_nothing_is_visible() {
    // The finding an operator most needs: a strategy contributing nothing is
    // invisible in the blended output, because a blend cannot show you a
    // strategy that was silent.
    let run = RecRunReport {
        blended: ids(&["a", "b"]),
        per_strategy: vec![
            StrategyContribution {
                name: "collab".into(),
                produced: 2,
                ranked: vec![("a".into(), 0.9), ("b".into(), 0.4)],
            },
            StrategyContribution {
                name: "time_decay".into(),
                produced: 0,
                ranked: vec![],
            },
        ],
    };
    let report = rec_shadow::compare(&ids(&["a", "b"]), &ids(&["a", "b"]), &run, 1);
    let silent = report
        .strategies
        .iter()
        .find(|s| s.name == "time_decay")
        .expect("time_decay reported");
    assert_eq!(silent.produced, 0);
    assert!(!silent.reached_blend);
    let live = report
        .strategies
        .iter()
        .find(|s| s.name == "collab")
        .expect("collab reported");
    assert!(live.reached_blend);
}

#[test]
fn producing_results_but_not_reaching_the_blend_is_a_distinct_finding() {
    // A strategy can return fifty works and change nothing, because every one
    // of them is also ranked by another strategy and none reaches the cap.
    // "produced 50" and "reached the blend" are different questions and the
    // report must not conflate them into a single green tick.
    let run = RecRunReport {
        blended: ids(&["a"]),
        per_strategy: vec![StrategyContribution {
            name: "collab".into(),
            produced: 50,
            ranked: (0..50).map(|i| (format!("w{i}"), 0.01)).collect(),
        }],
    };
    let report = rec_shadow::compare(&ids(&["a"]), &ids(&["a"]), &run, 1);
    assert_eq!(report.strategies[0].produced, 50);
    assert!(
        !report.strategies[0].reached_blend,
        "50 results none of which survived the cap is not 'reached the blend'"
    );
}

#[test]
fn summary_names_the_things_an_operator_acts_on() {
    let report = rec_shadow::compare(
        &ids(&["a", "b", "c"]),
        &ids(&["a", "c", "d"]),
        &run_with("collab", 2, &["a", "c", "d"]),
        42,
    );
    let line = report.summary();
    assert!(
        line.contains("42"),
        "the sample count is in the line: {line}"
    );
    assert!(line.contains("displaced"), "{line}");
    assert!(line.contains("promoted"), "{line}");
}

#[test]
fn the_latest_report_is_readable_and_survives_a_poisoned_lock() {
    // A panic in one request's logging path must not turn every later
    // evaluation into a 500, and must not lose the report either.
    rec_shadow::record(rec_shadow::compare(
        &ids(&["a"]),
        &ids(&["a"]),
        &run_with("s", 1, &["a"]),
        1,
    ));
    assert!(rec_shadow::latest().is_some());

    // Poison the mutex the way a panic would: a panic must escape the closure
    // while the lock is held, which is exactly what poisons it.
    let _ = std::panic::catch_unwind(|| {
        rec_shadow::poison_the_report_slot_for_tests();
    });

    // A later evaluation must still be storable and readable.
    rec_shadow::record(rec_shadow::compare(
        &ids(&["x"]),
        &ids(&["x"]),
        &run_with("s", 1, &["x"]),
        2,
    ));
    let latest = rec_shadow::latest().expect("report readable after poisoning");
    assert_eq!(latest.served, ids(&["x"]));
    assert_eq!(latest.sample, 2);
}

// ---------------------------------------------------------------------------
// The wiring: shadow mode must not change what a reader receives
// ---------------------------------------------------------------------------

mod router {
    use super::*;

    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use lorehaven_app::config::Config;
    use lorehaven_app::server;
    use lorehaven_app::state::AppState;
    use lorehaven_db::{Database, DatabaseConfig};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    const GOOD_PASSWORD: &str = "a-long-enough-passphrase";

    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lorehaven-m5208-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn config_for(dir: &Path, mode: &str) -> Config {
        let mut config = Config::development_defaults();
        config.storage.root = dir.to_path_buf();
        config.database = DatabaseConfig::new(format!(
            "sqlite://{}/lorehaven.sqlite?mode=rwc",
            dir.display()
        ));
        // These tests share the process-global rate-limit buckets at 127.0.0.1.
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
        config.discovery.rec_mode = mode.to_owned();
        config
    }

    struct Client {
        app: axum::Router,
        cookies: Vec<(String, String)>,
    }

    impl Client {
        fn cookie(&self, name: &str) -> Option<&str> {
            self.cookies
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        }

        fn capture_cookies(&mut self, response: &axum::response::Response) {
            for value in response.headers().get_all(header::SET_COOKIE) {
                let Ok(raw) = value.to_str() else { continue };
                let pair = raw.split(';').next().unwrap_or(raw);
                let Some((name, val)) = pair.split_once('=') else {
                    continue;
                };
                let (name, val) = (name.to_owned(), val.to_owned());
                self.cookies.retain(|(key, _)| key != &name);
                if !val.is_empty() {
                    self.cookies.push((name, val));
                }
            }
        }

        fn cookie_header(&self) -> String {
            self.cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ")
        }

        async fn request(
            &mut self,
            method: &str,
            uri: &str,
            body: Option<Value>,
        ) -> (StatusCode, Value) {
            let mut builder = Request::builder().method(method).uri(uri);
            let cookies = self.cookie_header();
            if !cookies.is_empty() {
                builder = builder.header(header::COOKIE, cookies);
            }
            if !matches!(method, "GET" | "HEAD" | "OPTIONS") {
                if let Some(token) = self.cookie("lorehaven_csrf") {
                    builder = builder.header("x-csrf-token", token.to_owned());
                }
            }
            let request = match body {
                Some(ref value) => builder
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(value).expect("serialise")))
                    .expect("request"),
                None => builder.body(Body::empty()).expect("request"),
            };
            let response = self.app.clone().oneshot(request).await.expect("response");
            let status = response.status();
            self.capture_cookies(&response);
            let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
                .await
                .expect("body");
            let value = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap_or(Value::Null)
            };
            (status, value)
        }

        /// Register, verify and sign in. Returns the pseudonym, which is what
        /// the recommender keys on.
        async fn signed_in_reader(&mut self) {
            let email = "reader@example.test";
            let (status, body) = self
                .request(
                    "POST",
                    "/api/v1/auth/register",
                    Some(json!({
                        "email": email,
                        "password": GOOD_PASSWORD,
                        "handle": "reader",
                        "display_name": "Reader",
                        "age_band": "adult",
                    })),
                )
                .await;
            assert!(
                status == StatusCode::CREATED || status == StatusCode::CONFLICT,
                "register: {status} {body}"
            );
            let (status, body) = self
                .request(
                    "POST",
                    "/api/v1/auth/login",
                    Some(json!({ "email": email, "password": GOOD_PASSWORD })),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "login: {body}");
        }
    }

    /// The account id of the first account on this database.
    ///
    /// Read from the database rather than an API response, on purpose: §7's
    /// acceptance criterion is that pseud linkage is *absent* from public API
    /// responses, so there is deliberately no endpoint that hands a client its
    /// own account id. Reaching for one to make a test pass would be asking for
    /// the very leak §7 forbids. `require_operator` compares against the same
    /// column, so the database is also the authoritative source here.
    async fn sole_account_id(db: &Database) -> String {
        sqlx::query_scalar("SELECT id FROM accounts ORDER BY created_at LIMIT 1")
            .fetch_one(db.sqlite_pool().expect("this file's harness is sqlite"))
            .await
            .expect("an account row exists after registration")
    }

    /// Build a router on a fresh database in the given rec mode.
    async fn harness(tag: &str, mode: &str) -> (std::path::PathBuf, Database, Config) {
        let dir = scratch_dir(tag);
        let config = config_for(&dir, mode);
        let db = Database::connect(&config.database)
            .await
            .expect("db connect");
        db.migrate().await.expect("migrations");
        (dir, db, config)
    }

    /// Publish `n` works so the feed has something in it.
    ///
    /// Without this the comparison below would run against two *empty* feeds,
    /// which agree trivially and prove nothing about whether shadow mode
    /// changes what is served. `create_work` writes a draft, and the feed's own
    /// predicate is `lifecycle = 'published' AND visibility = 'public'`, so the
    /// promote step is what makes the works actually candidates.
    async fn seed_published_works(db: &Database, tag: &str, n: usize) {
        for i in 0..n {
            // The owner is a foreign key, so a made-up id would only produce a
            // constraint failure. Create a real account and pseud, once, and
            // hang every work off it.
            let account = lorehaven_db::identity::create_account(
                db,
                &format!("{tag}-{i}@example.test"),
                lorehaven_domain::policy::AgeState::DeclaredAdult,
                lorehaven_db::identity::AccountStatus::Active,
            )
            .await
            .expect("create account");
            let owner = lorehaven_db::identity::create_pseud(
                db,
                account,
                &format!("{tag}-p{i}"),
                "Seeded Author",
            )
            .await
            .expect("create pseud");
            let work = lorehaven_db::content::create_work(
                db,
                owner,
                &format!("{tag} work {i}"),
                Some("public"),
            )
            .await
            .expect("create work");
            sqlx::query(
                "UPDATE works SET lifecycle = 'published', published_at = '2026-01-01 00:00:00' \
                 WHERE id = ?",
            )
            .bind(work.id.to_string())
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("publish work");
        }
    }

    #[tokio::test]
    async fn shadow_mode_serves_the_legacy_feed_unchanged() {
        // The safety property, pinned end to end. Same reader, same empty
        // instance, two modes: the served feed must be byte-identical, because
        // the whole point of shadow is that the reader cannot tell.
        let (_d1, db1, c1) = harness("feed-legacy", "legacy").await;
        seed_published_works(&db1, "feed-legacy", 4).await;
        let mut legacy = Client {
            app: server::build_router(AppState::new(c1, db1)),
            cookies: vec![],
        };
        let legacy_feed = legacy.request("GET", "/api/v1/discovery", None).await;

        let (_d2, db2, c2) = harness("feed-shadow", "shadow").await;
        seed_published_works(&db2, "feed-legacy", 4).await;
        let mut shadow = Client {
            app: server::build_router(AppState::new(c2, db2)),
            cookies: vec![],
        };
        let shadow_feed = shadow.request("GET", "/api/v1/discovery", None).await;

        assert_eq!(legacy_feed.0, shadow_feed.0, "status differs");
        // Compare the feed, not the whole body: `request_id` is unique per
        // response by design, so asserting whole-body equality would compare
        // two identifiers that are *meant* to differ and would pass or fail
        // for the wrong reason. What must match is what a reader is shown.
        // Compare the *titles in order*, not whole items. The two instances are
        // separate databases and `create_work` mints a random work id, so item
        // ids cannot match by construction — comparing them would be comparing
        // two random values and would fail for a reason that has nothing to do
        // with shadow mode. The served title sequence is the reader-facing
        // result, and it is fully determined by the seed.
        let titles = |feed: &Value| -> Vec<String> {
            feed["items"]
                .as_array()
                .expect("items array")
                .iter()
                .map(|item| item["title"].as_str().expect("item title").to_owned())
                .collect()
        };
        assert_eq!(
            titles(&legacy_feed.1),
            titles(&shadow_feed.1),
            "shadow mode served a different ranking to the reader"
        );
        assert_eq!(legacy_feed.1["sort"], shadow_feed.1["sort"], "sort differs");
        // Guard against a vacuous pass: two empty feeds agree, which would make
        // this test true without shadow mode ever being exercised.
        assert!(
            !legacy_feed.1["items"]
                .as_array()
                .expect("items array")
                .is_empty(),
            "the legacy feed was empty, so this comparison proves nothing"
        );
    }

    /// Register a reader, then re-serve the same database with that account
    /// named as the operator.
    ///
    /// `require_operator` matches `administration.operator_account_id` against
    /// the session's account, so being an operator in a test means naming an
    /// account in config — not escalating a trust level. That is the same lever
    /// a real deployment pulls, so the test exercises the real one.
    ///
    /// The session cookie is a server-side record, so a router rebuilt on the
    /// same `Database` still honours it — which is what makes this cheap: no
    /// second login, no forged cookie, just a different `Config`.
    async fn harness_with_operator(
        tag: &str,
        mode: &str,
    ) -> (std::path::PathBuf, Database, Config, Client) {
        let (dir, db, mut config) = harness(tag, mode).await;
        let mut first = Client {
            app: server::build_router(AppState::new(config.clone(), db.clone())),
            cookies: vec![],
        };
        first.signed_in_reader().await;
        let account_id = sole_account_id(&db).await;
        config.administration.operator_account_id = Some(
            lorehaven_domain::ids::AccountId::from_str(&account_id).expect("account id parses"),
        );
        let client = Client {
            app: server::build_router(AppState::new(config.clone(), db.clone())),
            // Carry the session cookies across the router swap.
            cookies: first.cookies,
        };
        (dir, db, config, client)
    }

    #[tokio::test]
    async fn shadow_mode_evaluates_for_a_signed_in_reader() {
        let (_dir, _db, _config, mut client) = harness_with_operator("evaluates", "shadow").await;

        let (status, body) = client.request("GET", "/api/v1/discovery", None).await;
        assert_eq!(status, StatusCode::OK, "discovery: {body}");

        // The evaluation ran and was recorded.
        let (status, report) = client
            .request("GET", "/api/v1/operator/rec/shadow", None)
            .await;
        assert_eq!(status, StatusCode::OK, "shadow report: {report}");
        assert_eq!(report["mode"], "shadow");
        assert_eq!(report["shadow_active"], true);
        assert_eq!(report["evaluated"], true, "an evaluation should have run");
        assert!(
            report["evaluations_this_process"].as_u64().unwrap_or(0) >= 1,
            "the evaluation count is reported: {report}"
        );
        let latest = &report["latest"];
        assert!(latest.is_object(), "latest: {report}");
        // The report records the served ranking, so it can be checked against
        // something without re-running the evaluation.
        assert!(
            latest["served"].is_array(),
            "served is recorded so the report is self-contained: {latest}"
        );
        assert!(latest["agreement"].is_string(), "agreement: {latest}");
        assert!(latest["sample"].as_u64().expect("sample") >= 1);
    }

    #[tokio::test]
    async fn a_non_shadow_instance_reports_that_it_has_not_evaluated() {
        // "Not running" and "ran and found nothing" are different answers, and
        // an operator switching modes needs the difference.
        let (_dir, _db, _config, mut client) = harness_with_operator("not-run", "legacy").await;

        let (status, report) = client
            .request("GET", "/api/v1/operator/rec/shadow", None)
            .await;
        assert_eq!(status, StatusCode::OK, "{report}");
        assert_eq!(report["shadow_active"], false);
        assert_eq!(
            report["evaluated"], false,
            "legacy mode must not claim an evaluation happened"
        );
    }

    #[tokio::test]
    async fn the_shadow_report_is_not_in_the_reader_facing_feed() {
        // The discovery response is served to signed-out readers. A field
        // describing how the instance's ranker would differ is tuning
        // information that does not belong in an anonymous payload.
        let (_dir, db, config) = harness("not-leaked", "shadow").await;
        let mut client = Client {
            app: server::build_router(AppState::new(config, db)),
            cookies: vec![],
        };
        let (_, feed) = client.request("GET", "/api/v1/discovery", None).await;
        let text = serde_json::to_string(&feed).expect("serialise feed");
        for leak in ["shadow", "displaced", "promoted", "agreement"] {
            assert!(
                !text.contains(leak),
                "the reader-facing feed leaked {leak:?}: {text}"
            );
        }
    }

    #[tokio::test]
    async fn the_shadow_report_is_not_reachable_by_a_plain_reader() {
        // A signed-in reader on an instance that has *no* configured operator.
        // `require_operator` answers 404 rather than 403 so the route's existence
        // is not itself disclosed; the assertion accepts 403/401 as well so it
        // fails on *access* rather than on a future change of that code.
        let (dir, db, config) = harness("authz", "shadow").await;
        let mut reader = Client {
            app: server::build_router(AppState::new(config.clone(), db.clone())),
            cookies: vec![],
        };
        reader.signed_in_reader().await;
        assert!(
            config.administration.operator_account_id.is_none(),
            "this instance must name no operator for the test to mean anything"
        );

        let (status, _) = reader
            .request("GET", "/api/v1/operator/rec/shadow", None)
            .await;
        assert!(
            status == StatusCode::NOT_FOUND
                || status == StatusCode::FORBIDDEN
                || status == StatusCode::UNAUTHORIZED,
            "a plain reader reached the shadow report: {status}"
        );
        drop(dir);
    }

    #[tokio::test]
    async fn an_anonymous_visitor_cannot_read_the_shadow_report() {
        let (_dir, db, config) = harness("anon", "shadow").await;
        let mut client = Client {
            app: server::build_router(AppState::new(config, db)),
            cookies: vec![],
        };
        let (status, _) = client
            .request("GET", "/api/v1/operator/rec/shadow", None)
            .await;
        assert!(
            status == StatusCode::UNAUTHORIZED
                || status == StatusCode::FORBIDDEN
                || status == StatusCode::NOT_FOUND,
            "an anonymous visitor reached the shadow report: {status}"
        );
    }
}
