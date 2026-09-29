//! Acceptance: the metadata exchange endpoint (spec §11.17, §15.17, §19.14, M57).
//!
//! §11.17's own acceptance list, plus the four properties that the schema and
//! the trust bars exist to guarantee. Each test below names the spec sentence it
//! pins, because a test that does not say which sentence it enforces is a test
//! that cannot be argued with when the spec changes.
//!
//! The load-bearing claims, in order of how badly they would be violated:
//!
//! 1. **The opt-in is independent and real.** A server that did not enable the
//!    exchange answers 404 on every door. Not 403 — a 403 confirms the resource
//!    exists, and §11.17 says "no request shape turns on a server that did not
//!    enable it".
//! 2. **The schema refuses by name.** A payload carrying `reader_id` or
//!    `reading_progress` is rejected, and the error says which field. A silently
//!    trimmed payload is the failure mode §11.17 names explicitly, and it is
//!    worse than a rejection: the sender still believes it sent the field.
//! 3. **Trust is asymmetric.** TL1 submits, TL3 curates, and neither bar is
//!    reachable by configuration.
//! 4. **Unverified is usable.** A new name from a signal is immediately
//!    attachable, searchable and visible — carrying `unverified`, never
//!    presented as curated. §15.17 is explicit that the alternative stalls
//!    organic growth.

mod router {
    use std::path::PathBuf;

    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use lorehaven_app::config::Config;
    use lorehaven_app::server;
    use lorehaven_app::state::AppState;
    use lorehaven_db::{Database, DatabaseConfig};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    const GOOD_PASSWORD: &str = "correct-horse-battery-staple-42";

    pub fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lorehaven-exchange-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    pub struct Harness {
        _dir: PathBuf,
        db: Database,
        app: axum::Router,
        cookies: Vec<(String, String)>,
    }

    impl Harness {
        pub async fn new(tag: &str) -> Self {
            let dir = scratch_dir(tag);
            let mut config = Config::development_defaults();
            config.storage.root = dir.to_path_buf();
            config.database = DatabaseConfig::new(format!(
                "sqlite://{}/lorehaven.sqlite?mode=rwc",
                dir.display()
            ));
            let db = Database::connect(&config.database)
                .await
                .expect("db connect");
            db.migrate().await.expect("migrations");
            let app = server::build_router(AppState::new(config.clone(), db.clone()));
            Self {
                _dir: dir,
                db,
                app,
                cookies: Vec::new(),
            }
        }

        pub fn db(&self) -> &Database {
            &self.db
        }

        /// Flip the operator's opt-in. §11.17: the server opts in separately
        /// from each client.
        pub async fn enable(&self) {
            lorehaven_db::exchange::set_enabled(&self.db, true)
                .await
                .expect("enable the exchange");
        }

        /// Enable the exchange *and* give this instance an exchange identity.
        ///
        /// The two are separate settings and the demand tests need both: without
        /// an `instance_id` there is no deduplication key for latent demand, and
        /// the route correctly declines to record any.
        pub async fn enable_as(&self, instance_id: &str) {
            // The id first: `set_instance_id` creates the settings row with
            // `enabled = 0`, so enabling afterwards is what makes it stick. In the
            // other order the id is set and then the switch is flipped, which also
            // works — but only because `set_enabled`'s update arm leaves
            // `instance_id` alone. Setting it first does not depend on that.
            lorehaven_db::exchange::set_instance_id(&self.db, instance_id)
                .await
                .expect("set the exchange instance id");
            self.enable().await;
        }

        fn cookie_header(&self) -> String {
            self.cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ")
        }

        fn cookie(&self, name: &str) -> Option<&String> {
            self.cookies.iter().find(|(k, _)| k == name).map(|(_, v)| v)
        }

        fn capture_cookies(&mut self, response: &axum::response::Response) {
            for value in response.headers().get_all(header::SET_COOKIE) {
                let Ok(text) = value.to_str() else { continue };
                let pair = text.split(';').next().unwrap_or("");
                if let Some((k, v)) = pair.split_once('=') {
                    self.cookies.retain(|(ek, _)| ek != k);
                    self.cookies.push((k.to_string(), v.to_string()));
                }
            }
        }

        pub async fn request(
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
                if let Some(token) = self.cookie("lorehaven_csrf").cloned() {
                    builder = builder.header("x-csrf-token", token);
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

        /// Register, sign in, and set the account's trust level.
        ///
        /// The level is set explicitly because a **new account is TL0**
        /// (`TL_NEW`), and §19.14's submit bar is TL1. Trust is earned, not
        /// granted at registration — so a test that registers and submits
        /// without this line is testing a 403, not the exchange. That is the
        /// spec working: a brand-new account cannot contribute until it has
        /// any standing at all.
        pub async fn signed_in_at(&mut self, email: &str, handle: &str, trust: i64) {
            self.signed_in(email, handle).await;
            let account_id = self.sole_account_id().await;
            lorehaven_db::governance::set_trust(self.db(), &account_id, trust, "test")
                .await
                .expect("set trust");
        }

        /// The id of the only account on this database.
        ///
        /// Read from the database rather than an API response on purpose: §7
        /// forbids an endpoint that hands a client its own account id, so there
        /// is deliberately no such door to reach for.
        pub(crate) async fn sole_account_id(&self) -> String {
            // **Not the first row in `accounts`.** Migration 0094 inserts the
            // instance's own system account with `created_at = 2026-01-01`, earlier
            // than any registration, so `ORDER BY created_at LIMIT 1` — and a bare
            // `LIMIT 1` — return *it*. The symptom then surfaces three files away:
            // a 404 on an endpoint that exists, or `RowNotFound` on a pseud.
            //
            // The system account is excluded explicitly rather than by relying on it
            // sorting later, because it is identifiable on purpose: a fixed literal
            // id, a `system` status, a `.invalid` email (migration 0094, which says
            // so). `lorehaven_db::is_system_account` is the same fact in Rust.
            sqlx::query_scalar(
                "SELECT id FROM accounts
                 WHERE id != ?1   -- not the instance's system account
                 ORDER BY created_at LIMIT 1",
            )
            .bind(lorehaven_db::SYSTEM_ACCOUNT.to_string())
            .fetch_one(self.db.sqlite_pool().expect("this harness is sqlite"))
            .await
            .expect("an account row exists after registration")
        }

        /// Register and sign in, leaving the account at its earned trust level.
        pub async fn signed_in(&mut self, email: &str, handle: &str) {
            let (status, body) = self
                .request(
                    "POST",
                    "/api/v1/auth/register",
                    Some(json!({
                        "email": email,
                        "password": GOOD_PASSWORD,
                        "handle": handle,
                        "display_name": handle,
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

        /// A well-formed batch carrying one signal.
        pub fn batch(title: &str, tags: &[&str]) -> Value {
            json!({
                "version": 1,
                "signals": [{
                    "site_ids": [],
                    "title": title,
                    "author_names": ["An Author"],
                    "fandom": "Some Fandom",
                    "tags": tags,
                    "characters": [],
                    "relationships": [],
                    "content_rating": "general",
                    "language": "en",
                }],
            })
        }
    }
}

use axum::http::StatusCode;
use router::Harness;
use serde_json::json;

// ---------------------------------------------------------------------------
// The opt-in (§11.17)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_server_that_did_not_enable_the_exchange_answers_404_on_every_door() {
    // §11.17: "the server exposes the endpoint only when its operator enabled
    // it. No request shape turns on a server that did not enable it."
    //
    // The version door is excluded, deliberately and for a stated reason: it
    // returns no instance data, only a range and a boolean, and a client has to
    // be able to discover whether negotiation is possible before it tries. It
    // is covered on its own by `the_version_door_answers_even_with_the_exchange_disabled`.
    let mut h = Harness::new("optin-off").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    for (method, uri, body) in [
        ("GET", "/api/v1/exchange/canonical?entity=tag:x", None),
        (
            "POST",
            "/api/v1/exchange/signals",
            Some(Harness::batch("A Work", &["x"])),
        ),
    ] {
        let (status, _) = h.request(method, uri, body).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {uri} answered {status} with the exchange disabled"
        );
    }
}

#[tokio::test]
async fn a_disabled_exchange_answers_404_rather_than_403() {
    // The distinction is the test. A 403 confirms the door exists, which tells a
    // prober this instance runs an exchange at all — the opposite of "no request
    // shape turns on a server that did not enable it".
    let h = Harness::new("optin-403").await;
    let settings = lorehaven_db::exchange::get_settings(h.db())
        .await
        .expect("read");
    assert!(
        settings.is_none(),
        "a fresh instance has no settings row, which means disabled"
    );
    let mut h = h;
    let (status, _) = h
        .request("GET", "/api/v1/exchange/canonical?entity=tag:x", None)
        .await;
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "403 would confirm the door exists"
    );
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn enabling_the_exchange_opens_the_doors() {
    let mut h = Harness::new("optin-on").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    let (status, body) = h
        .request("GET", "/api/v1/exchange/canonical?entity=tag:nothing", None)
        .await;
    assert_eq!(status, StatusCode::OK, "canonical: {body}");
    let (status, body) = h
        .request(
            "POST",
            "/api/v1/exchange/signals",
            Some(Harness::batch("A Work", &["x"])),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "signals: {body}");
}

// ---------------------------------------------------------------------------
// Version negotiation (§11.17 / the wire contract)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_version_door_reports_the_supported_range() {
    let h = Harness::new("version").await;
    let mut h = h;
    let (status, body) = h.request("GET", "/api/v1/exchange/version", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["current"], 1);
    assert_eq!(body["supported"]["min"], 1);
    assert_eq!(body["supported"]["max"], 1);
}

#[tokio::test]
async fn the_version_door_answers_even_with_the_exchange_disabled() {
    // A client has to be able to discover whether negotiation is possible before
    // it tries. This door returns no instance data — a range and a boolean — so
    // answering it does not turn on anything.
    let mut h = Harness::new("version-off").await;
    let (status, body) = h.request("GET", "/api/v1/exchange/version", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], false, "it says so rather than pretending");
}

#[tokio::test]
async fn an_out_of_range_version_is_refused() {
    let mut h = Harness::new("version-bad").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    let mut batch = Harness::batch("A Work", &["x"]);
    batch["version"] = json!(99);
    let (status, body) = h
        .request("POST", "/api/v1/exchange/signals", Some(batch))
        .await;
    assert!(
        !status.is_success(),
        "an unsupported version is refused: {status} {body}"
    );
}

// ---------------------------------------------------------------------------
// The schema prohibition (§11.17) — the deliverable of this milestone
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_signal_carrying_a_prohibited_field_is_refused_by_name() {
    // §11.17: reading history, progress, position, reading status, ratings,
    // notes, kudos, private library membership, pseud linkage, draft content,
    // source credentials, session identifiers, IP addresses and file paths "are
    // rejected by name rather than having its payload quietly trimmed".
    let mut h = Harness::new("schema-reader-id").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    let mut batch = Harness::batch("A Work", &["x"]);
    batch["signals"][0]["reader_id"] = json!("reader-1");
    let (status, body) = h
        .request("POST", "/api/v1/exchange/signals", Some(batch))
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a payload carrying `reader_id` is refused, not trimmed: {body}"
    );
    let text = body.to_string();
    assert!(
        text.contains("reader_id"),
        "the refusal names the offending field: {text}"
    );
}

#[tokio::test]
async fn every_prohibited_field_is_refused_rather_than_ignored() {
    // One field proves the mechanism; the list proves the coverage. §0.3 is a
    // prohibition on a category, and a category is only enforced if every member
    // of it is.
    let mut h = Harness::new("schema-all").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    for field in [
        "reader_id",
        "reading_progress",
        "reading_position",
        "reading_status",
        "rating",
        "notes",
        "kudos",
        "library_membership",
        "pseud_id",
        "draft_content",
        "source_credentials",
        "session_id",
        "ip_address",
        "file_path",
    ] {
        let mut batch = Harness::batch("A Work", &["x"]);
        batch["signals"][0][field] = json!("leaked");
        let (status, body) = h
            .request("POST", "/api/v1/exchange/signals", Some(batch))
            .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "`{field}` must be refused, not ignored: {body}"
        );
        assert!(
            body.to_string().contains(field),
            "`{field}` must be named in the refusal: {body}"
        );
    }
}

#[tokio::test]
async fn a_refused_signal_stores_nothing() {
    // The refusal has to happen before the write, or the payload has already
    // been persisted by the time it is rejected.
    let mut h = Harness::new("schema-no-write").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    let mut batch = Harness::batch("Leaky", &["x"]);
    batch["signals"][0]["reader_id"] = json!("reader-1");
    let (status, _) = h
        .request("POST", "/api/v1/exchange/signals", Some(batch))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let stored = lorehaven_db::exchange::count_provenance_signals(h.db(), "unused")
        .await
        .expect("count");
    assert_eq!(stored, 0, "nothing was written for a refused payload");
    let (status, body) = h
        .request("GET", "/api/v1/exchange/canonical?entity=tag:x", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["works"].as_array().map(|a| a.len()),
        Some(0),
        "and nothing became a canonical entity: {body}"
    );
}

// ---------------------------------------------------------------------------
// Trust bars (§19.14)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_anonymous_caller_cannot_submit() {
    let mut h = Harness::new("trust-anon").await;
    h.enable().await;
    let (status, body) = h
        .request(
            "POST",
            "/api/v1/exchange/signals",
            Some(Harness::batch("A Work", &["x"])),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

#[tokio::test]
async fn a_tl1_account_may_submit() {
    // §19.14: "Submitting a signal requires TL1 ... Gating submission above TL1
    // would gate participation, not quality."
    let mut h = Harness::new("trust-tl1").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    let (status, body) = h
        .request(
            "POST",
            "/api/v1/exchange/signals",
            Some(Harness::batch("A Work", &["quiet"])),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "a TL1 account may submit: {body}");
    assert_eq!(body["accepted"], 1);
}

#[tokio::test]
async fn a_brand_new_account_at_tl0_may_not_submit() {
    // Found while building this suite: a freshly registered account is `TL0`
    // (`TL_NEW`), and §19.14's submit bar is TL1. So a new account is refused
    // until it has some standing — which is the spec working, not a defect, and
    // is worth pinning because "a TL1 account may submit" on its own would pass
    // against a build where *every* account could submit.
    let mut h = Harness::new("trust-tl0").await;
    h.signed_in("newcomer@example.test", "newcomer").await;
    h.enable().await;
    assert_eq!(
        lorehaven_db::governance::trust_for(
            h.db(),
            // Not `LIMIT 1`: see the note above. The system account is the
            // only row that exists before this harness registers, so a bare
            // `LIMIT 1` returns it.
            &sqlx::query_scalar::<_, String>("SELECT id FROM accounts WHERE id != ?1 LIMIT 1",)
                .bind(lorehaven_db::SYSTEM_ACCOUNT.to_string())
                .fetch_one(h.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("an account")
        )
        .await
        .expect("trust"),
        0,
        "a new account starts at TL0"
    );
    let (status, body) = h
        .request(
            "POST",
            "/api/v1/exchange/signals",
            Some(Harness::batch("A Work", &["x"])),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

#[tokio::test]
async fn the_trust_bars_are_not_configuration() {
    // §19.14: "No configuration lowers either bar." There is nothing to set, so
    // this asserts the constants and the absence of any key a lowering could be
    // read from. `Config` is not `Serialize`, which is itself the point: a
    // serialisable config could be introspected for exchange keys, and one that
    // is not is a weaker guarantee than it looks, so the assertion is on the
    // constants plus a grep-visible absence in the source.
    assert_eq!(
        lorehaven_domain::exchange::TRUST_LEVEL_TO_SUBMIT,
        1,
        "§19.14: TL1 submits"
    );
    assert_eq!(
        lorehaven_domain::exchange::TRUST_LEVEL_TO_CURATE,
        3,
        "§19.14: TL3 curates"
    );
    assert!(!may_submit_below(1), "TL0 may not submit");
    assert!(!may_curate_below(3), "no level below TL3 may curate");
}

/// TL0 is the floor an account starts at; asserted through the domain rules so
/// the "not below TL1" claim is a claim about the rule, not about a constant.
fn may_submit_below(tl: i64) -> bool {
    tl < 1 && lorehaven_domain::exchange::may_submit(tl)
}

fn may_curate_below(tl: i64) -> bool {
    tl < 3 && lorehaven_domain::exchange::may_curate(tl)
}

// ---------------------------------------------------------------------------
// Deduplication (§11.17)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_reimport_costs_nothing_and_creates_no_second_record() {
    // §11.17: "A signal batch is deduplicated by content hash before it is
    // stored, so a re-import costs a submitter nothing and creates no second
    // record."
    let mut h = Harness::new("dedup").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    let batch = Harness::batch("The Same Work", &["quiet"]);
    let (first_status, first) = h
        .request("POST", "/api/v1/exchange/signals", Some(batch.clone()))
        .await;
    assert_eq!(first_status, StatusCode::OK, "{first}");
    assert_eq!(first["duplicates"], 0);
    let (second_status, second) = h
        .request("POST", "/api/v1/exchange/signals", Some(batch))
        .await;
    assert_eq!(second_status, StatusCode::OK);
    assert_eq!(
        second["duplicates"], 1,
        "the second one is a duplicate: {second}"
    );
    assert_eq!(second["results"][0]["duplicate"], true);
}

#[tokio::test]
async fn a_duplicate_does_not_reinforce_the_entity_count() {
    // A duplicate is not new evidence. If it counted, a submitter who re-imports
    // nightly would push their own tag up the review queue forever, which is
    // exactly the "a count of submitters read as a count of readers" problem
    // §15.17 forbids.
    let mut h = Harness::new("dedup-count").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    let batch = Harness::batch("The Same Work", &["quiet"]);
    h.request("POST", "/api/v1/exchange/signals", Some(batch.clone()))
        .await;
    h.request("POST", "/api/v1/exchange/signals", Some(batch))
        .await;
    let (status, body) = h
        .request("GET", "/api/v1/exchange/canonical?entity=tag:quiet", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["works"][0]["signal_count"], 1,
        "one signal, one count: {body}"
    );
}

// ---------------------------------------------------------------------------
// Auto-created canonical entities (§15.17)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_signal_naming_an_unknown_tag_creates_a_usable_unverified_entity() {
    // §15.17's first acceptance line.
    let mut h = Harness::new("entity-new").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    h.request(
        "POST",
        "/api/v1/exchange/signals",
        Some(Harness::batch("A Work", &["Slow Burn"])),
    )
    .await;
    let (status, body) = h
        .request(
            "GET",
            "/api/v1/exchange/canonical?entity=tag:slow+burn",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["works"].as_array().map(|a| a.len()), Some(1), "{body}");
    assert_eq!(body["works"][0]["review_status"], "unverified");
    assert_eq!(
        body["works"][0]["title"], "Slow Burn",
        "the submitter's spelling"
    );
}

#[tokio::test]
async fn an_unverified_entity_is_never_rendered_as_curated() {
    // §15.17's second acceptance line: "An unverified entity is never rendered
    // as curated."
    let mut h = Harness::new("entity-unverified").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    h.request(
        "POST",
        "/api/v1/exchange/signals",
        Some(Harness::batch("A Work", &["Slow Burn"])),
    )
    .await;
    let (_, body) = h
        .request(
            "GET",
            "/api/v1/exchange/canonical?entity=tag:slow+burn",
            None,
        )
        .await;
    assert_ne!(body["works"][0]["review_status"], "verified");
    assert!(
        body["works"][0]["curated_at"].is_null(),
        "an uncurated entity has no curation timestamp: {body}"
    );
}

#[tokio::test]
async fn distinct_signals_reinforce_one_entity_rather_than_creating_duplicates() {
    // §15.17: "A signal naming a known tag adds an alias rather than creating a
    // duplicate." Three spellings, one entity.
    let mut h = Harness::new("entity-alias").await;
    h.signed_in_at("a@example.test", "alpha", 1).await;
    h.enable().await;
    h.request(
        "POST",
        "/api/v1/exchange/signals",
        Some(Harness::batch("Work One", &["Slow Burn"])),
    )
    .await;
    let (_, body) = h
        .request(
            "GET",
            "/api/v1/exchange/canonical?entity=tag:slow+burn",
            None,
        )
        .await;
    assert_eq!(body["works"][0]["signal_count"], 1);
    // The same name in a different work is a different signal, so it
    // reinforces the same entity.
    h.request(
        "POST",
        "/api/v1/exchange/signals",
        Some(Harness::batch("Work Two", &["slow burn"])),
    )
    .await;
    let (_, body) = h
        .request(
            "GET",
            "/api/v1/exchange/canonical?entity=tag:slow+burn",
            None,
        )
        .await;
    assert_eq!(
        body["works"].as_array().map(|a| a.len()),
        Some(1),
        "one entity, not two: {body}"
    );
    assert_eq!(body["works"][0]["signal_count"], 2, "two distinct signals");
}

#[tokio::test]
async fn the_canonical_response_never_carries_a_submitter_or_a_holder_count() {
    // §11.17: "It never reveals who submitted a signal, and it never reveals how
    // many accounts hold a work: a count of holders is not computable from the
    // data the instance holds, and an endpoint that appeared to offer one would
    // be offering a guess."
    let mut h = Harness::new("entity-noleak").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    h.request(
        "POST",
        "/api/v1/exchange/signals",
        Some(Harness::batch("A Work", &["quiet"])),
    )
    .await;
    let (_, body) = h
        .request("GET", "/api/v1/exchange/canonical?entity=tag:quiet", None)
        .await;
    let text = body.to_string();
    for forbidden in ["account", "holder", "reader", "submitter", "email"] {
        assert!(
            !text.contains(forbidden),
            "the canonical response must not carry `{forbidden}`: {text}"
        );
    }
}

// ---------------------------------------------------------------------------
// Validation refusals, by name (§11.17)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_empty_batch_is_refused_by_name() {
    let mut h = Harness::new("validate-empty").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    let (status, body) = h
        .request(
            "POST",
            "/api/v1/exchange/signals",
            Some(json!({ "version": 1, "signals": [] })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body.to_string().contains("signals"), "{body}");
}

#[tokio::test]
async fn a_signal_without_a_title_is_refused_by_index() {
    let mut h = Harness::new("validate-title").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    let (status, body) = h
        .request(
            "POST",
            "/api/v1/exchange/signals",
            Some(json!({
                "version": 1,
                "signals": [
                    { "title": "Fine" },
                    { "title": "   " }
                ]
            })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body.to_string().contains("signals[1].title"),
        "the refusal points at the offending index: {body}"
    );
}

#[tokio::test]
async fn an_unknown_entity_kind_is_refused_rather_than_invented() {
    let mut h = Harness::new("validate-kind").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    let (status, body) = h
        .request("GET", "/api/v1/exchange/canonical?entity=mood:cozy", None)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body.to_string().contains("mood"), "{body}");
}

#[tokio::test]
async fn a_malformed_entity_pair_is_refused() {
    let mut h = Harness::new("validate-pair").await;
    h.enable().await;
    let (status, body) = h
        .request("GET", "/api/v1/exchange/canonical?entity=justaname", None)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body.to_string().contains("kind:norm"), "{body}");
}

// ---------------------------------------------------------------------------
// Latent demand (§16.16.1)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn latent_demand_is_once_per_work_per_instance_and_never_a_headcount() {
    // §16.16.1: the demand item is "created or reinforced once per work per
    // submitting instance, and re-signalling the same work adds no weight. The
    // count of submitters is never a count of readers."
    let h = Harness::new("demand").await;
    for _ in 0..3 {
        lorehaven_db::exchange::reinforce_latent_demand(h.db(), "work-1", "sister-a")
            .await
            .expect("reinforce");
    }
    let items = lorehaven_db::exchange::list_latent_demand(h.db(), 10)
        .await
        .expect("list");
    assert_eq!(items.len(), 1, "three signals, one demand item: {items:?}");
    // A second instance is a genuinely distinct piece of demand, so it is a
    // second row rather than a heavier first row.
    lorehaven_db::exchange::reinforce_latent_demand(h.db(), "work-1", "sister-b")
        .await
        .expect("reinforce");
    let items = lorehaven_db::exchange::list_latent_demand(h.db(), 10)
        .await
        .expect("list");
    assert_eq!(items.len(), 2, "two instances, two rows: {items:?}");
}

// ---------------------------------------------------------------------------
// M11-17a — §15.17's usable-while-unverified claim
//
// The claim is not "the exchange records the name". It is that the name is
// usable *in the instance's own taxonomy* — searchable, browsable, attachable —
// while visibly not curated. A name that lives only in the exchange's own table
// satisfies GET /canonical and is invisible everywhere else, which is the stall
// §15.17 says the split exists to prevent.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_name_from_a_signal_is_searchable_in_the_instance_taxonomy() {
    let mut h = Harness::new("m1717a-search").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    h.request(
        "POST",
        "/api/v1/exchange/signals",
        Some(Harness::batch("A Work", &["Slow Burn"])),
    )
    .await;
    // `search_nodes` is the instance's own taxonomy search. If the entity is not
    // findable here, it is not usable, whatever GET /canonical says.
    let found = lorehaven_db::taxonomy::search_nodes(h.db(), Some("tag"), "slow burn", 10)
        .await
        .expect("search nodes");
    assert_eq!(
        found.len(),
        1,
        "the signal's tag is in the taxonomy: {found:?}"
    );
    assert_eq!(found[0].canonical, "Slow Burn");
    assert_eq!(
        found[0].review_status, "unverified",
        "§15.17: usable, and visibly not curated"
    );
    assert_eq!(found[0].signal_count, 1, "one signal, one count");
}

#[tokio::test]
async fn an_unverified_node_is_listed_by_the_instance_tag_browser() {
    // §15.17: it "may be attached to a work, appear in the tag browser, and be
    // searched". A separate assertion from the search one, because the tag
    // browser is a different query and a name can be findable by prefix search
    // while missing from a listing.
    let mut h = Harness::new("m1717a-browser").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    h.request(
        "POST",
        "/api/v1/exchange/signals",
        Some(Harness::batch("A Work", &["Cozy Fantasy"])),
    )
    .await;
    // `list_tags` returns `(id, canonical, kind, count)` tuples.
    let listed = lorehaven_db::taxonomy::list_tags(h.db(), 50, 0)
        .await
        .expect("list tags");
    assert!(
        listed
            .iter()
            .any(|(_, canonical, _, _)| canonical == "Cozy Fantasy"),
        "the tag browser shows it: {listed:?}"
    );
}

#[tokio::test]
async fn a_node_made_by_a_person_starts_curated_and_a_signal_does_not_demote_it() {
    // Two claims in one test, because they are the same column and a build that
    // satisfied only the second would still be wrong.
    let h = Harness::new("m1717a-curated").await;
    let made = lorehaven_db::taxonomy::create_node(h.db(), "tag", "Handmade")
        .await
        .expect("a person creates a tag");
    assert_eq!(
        made.review_status, "curated",
        "§19.4 quorum is what makes a node curated, and create_node is the person path"
    );
    let after = lorehaven_db::taxonomy::ensure_node_from_signal(h.db(), "tag", "Handmade")
        .await
        .expect("a signal names the same tag");
    assert_eq!(
        after.review_status, "curated",
        "agreement is not curation: a signal must not demote a curated node"
    );
    assert_eq!(after.id, made.id, "and must not create a second node");
    assert_eq!(after.signal_count, 1, "but the count still rises");
}

#[tokio::test]
async fn distinct_signals_reinforce_one_taxonomy_node() {
    let h = Harness::new("m1717a-reinforce").await;
    let first = lorehaven_db::taxonomy::ensure_node_from_signal(h.db(), "tag", "Slow Burn")
        .await
        .expect("first signal");
    let second = lorehaven_db::taxonomy::ensure_node_from_signal(h.db(), "tag", "slow burn")
        .await
        .expect("second signal");
    assert_eq!(first.id, second.id, "one node");
    assert_eq!(second.signal_count, 2, "two distinct signals reinforced it");
    assert_eq!(
        second.canonical, "Slow Burn",
        "a curator's display form is not rewritten by a later spelling"
    );
}

#[tokio::test]
async fn a_differing_spelling_is_recorded_as_an_alias_not_a_duplicate() {
    // §15.17: "A signal naming a known tag adds an alias rather than creating a
    // duplicate."
    let mut h = Harness::new("m1717a-alias").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    h.request(
        "POST",
        "/api/v1/exchange/signals",
        Some(Harness::batch("A Work", &["SLOW   BURN"])),
    )
    .await;
    let nodes = lorehaven_db::taxonomy::search_nodes(h.db(), Some("tag"), "slow burn", 10)
        .await
        .expect("search");
    assert_eq!(
        nodes.len(),
        1,
        "one node despite the messy spelling: {nodes:?}"
    );
    assert_eq!(
        nodes[0].canonical, "SLOW   BURN",
        "the submitter's spelling is kept"
    );
}

#[tokio::test]
async fn the_review_queue_is_ordered_by_signal_count() {
    // §15.17: "the name with forty distinct signals is examined before the name
    // with one." The ordering is the function's whole point, so it is asserted
    // rather than left to the index.
    let h = Harness::new("m1717a-queue").await;
    for _ in 0..3 {
        lorehaven_db::taxonomy::ensure_node_from_signal(h.db(), "tag", "Busy")
            .await
            .expect("busy");
    }
    lorehaven_db::taxonomy::ensure_node_from_signal(h.db(), "tag", "Quiet")
        .await
        .expect("quiet");
    let queue = lorehaven_db::taxonomy::list_unverified_nodes(h.db(), 10)
        .await
        .expect("queue");
    assert_eq!(
        queue.len(),
        2,
        "only unverified nodes are queued: {queue:?}"
    );
    assert_eq!(
        queue[0].canonical, "Busy",
        "the reinforced name comes first"
    );
    assert_eq!(queue[0].signal_count, 3);
}

// ---------------------------------------------------------------------------
// M11-17b — §19.14's asymmetric trust bars, over the real routes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_tl1_account_may_not_curate() {
    // §19.14: the asymmetry. If this passed for TL1, the whole design collapses
    // into "cheap to be believed", and the canonical layer stops being worth
    // reading.
    let mut h = Harness::new("m1717b-curate").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    h.request(
        "POST",
        "/api/v1/exchange/signals",
        Some(Harness::batch("A Work", &["Slow Burn"])),
    )
    .await;
    let (status, body) = h
        .request(
            "GET",
            "/api/v1/exchange/review-queue",
            None::<serde_json::Value>,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "TL1 may not read the queue: {body}"
    );
    let (status, body) = h
        .request(
            "POST",
            "/api/v1/exchange/entities/curate",
            Some(json!({ "kind": "tag", "norm": "slow burn", "canonical": "Slow Burn" })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "TL1 may not curate: {body}");
}

#[tokio::test]
async fn a_tl3_account_may_curate_and_the_provenance_survives() {
    // §15.17: "Curating an entity retains the originating signals."
    let mut h = Harness::new("m1717b-tl3").await;
    h.signed_in_at("curator@example.test", "curator", 3).await;
    h.enable().await;
    h.request(
        "POST",
        "/api/v1/exchange/signals",
        Some(Harness::batch("A Work", &["Slow Burn"])),
    )
    .await;
    let (status, body) = h
        .request(
            "POST",
            "/api/v1/exchange/entities/curate",
            Some(json!({ "kind": "tag", "norm": "slow burn", "canonical": "Slowburn" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "TL3 may curate: {body}");
    let nodes = lorehaven_db::taxonomy::search_nodes(h.db(), Some("tag"), "slow burn", 10)
        .await
        .expect("search");
    assert_eq!(nodes[0].review_status, "curated");
    assert_eq!(nodes[0].canonical, "Slowburn", "the curator's form wins");
    assert_eq!(
        nodes[0].signal_count, 1,
        "curation does not erase the count that justified it"
    );
}

#[tokio::test]
async fn a_tl2_account_may_neither_submit_nor_curate() {
    // The gap between the bars is the design: §19.14's "cheap to participate,
    // expensive to be believed".
    let mut h = Harness::new("m1717b-tl2").await;
    h.signed_in_at("mid@example.test", "mid", 2).await;
    h.enable().await;
    let (status, body) = h
        .request(
            "POST",
            "/api/v1/exchange/signals",
            Some(Harness::batch("A Work", &["x"])),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "TL2 is above the submit bar: {body}"
    );
    let (status, body) = h
        .request(
            "POST",
            "/api/v1/exchange/entities/curate",
            Some(json!({ "kind": "tag", "norm": "x", "canonical": "X" })),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "and below the curate bar: {body}"
    );
}

#[tokio::test]
async fn curation_does_not_rewrite_the_originating_signals() {
    // §15.17: signals are "retained as provenance, never rewritten". Asserted
    // by content, not by row count, because a rewrite that kept the count would
    // pass a weaker version of this test.
    let mut h = Harness::new("m1717b-provenance").await;
    h.signed_in_at("curator@example.test", "curator", 3).await;
    h.enable().await;
    let batch = Harness::batch("Original Title", &["Slow Burn"]);
    h.request("POST", "/api/v1/exchange/signals", Some(batch.clone()))
        .await;
    let before = lorehaven_db::exchange::count_provenance_signals(h.db(), "unused")
        .await
        .expect("count");
    h.request(
        "POST",
        "/api/v1/exchange/entities/curate",
        Some(json!({ "kind": "tag", "norm": "slow burn", "canonical": "Slowburn" })),
    )
    .await;
    let after = lorehaven_db::exchange::count_provenance_signals(h.db(), "unused")
        .await
        .expect("count");
    assert_eq!(
        before, after,
        "curating neither deletes nor rewrites a signal"
    );
    // And the original spelling is still the one the signal carried.
    let nodes = lorehaven_db::taxonomy::search_nodes(h.db(), Some("tag"), "slow burn", 10)
        .await
        .expect("search");
    assert_eq!(
        nodes[0].signal_count, 1,
        "the count that justified curation is intact"
    );
}

#[tokio::test]
async fn curating_an_unknown_entity_is_a_404_rather_than_a_silent_creation() {
    // A curation act on a name nobody has ever signalled would be an operator
    // inventing canonical metadata, which is a different thing with a different
    // authority. It must not quietly succeed.
    let mut h = Harness::new("m1717b-unknown").await;
    h.signed_in_at("curator@example.test", "curator", 3).await;
    h.enable().await;
    let (status, body) = h
        .request(
            "POST",
            "/api/v1/exchange/entities/curate",
            Some(json!({ "kind": "tag", "norm": "nothing here", "canonical": "Nothing" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

// ---------------------------------------------------------------------------
// M11-17c — §19.15's latent demand
//
// "a signal through the exchange for a work the instance does not hold" — and
// "the demand item is created or reinforced once per work per submitting
// instance, and re-signalling the same work adds no weight."
//
// Both halves are load-bearing. The first stops the instance asking for works
// it already has; the second stops a chatty sibling (or a retrying client)
// from buying the same demand several times over.
// ---------------------------------------------------------------------------

/// A batch whose single signal names an external work the instance does not hold.
fn signal_for(site: &str, id: &str) -> serde_json::Value {
    json!({
        "version": 1,
        "signals": [{
            "site_ids": [{ "site": site, "id": id }],
            "title": "A Stranger's Work",
            "author_names": ["An Author"],
            "fandom": "Some Fandom",
            "tags": ["Slow Burn"],
            "characters": [],
            "relationships": [],
            "content_rating": "general",
            "language": "en",
        }],
    })
}

/// Seed a `library_items` row, optionally materialised into a local work.
///
/// `work_id = None` is the *private-library* case the migration itself calls
/// out: the reader has it in their own shelves and the instance has no work.
async fn seed_library_item(h: &Harness, site: &str, source_work_key: &str, work_id: Option<&str>) {
    let now = lorehaven_db::identity::now_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    // `account_id` is NOT NULL: a library item is *somebody's* shelf, not the
    // instance's. So the row is owned by the signed-in reader, which is also the
    // only actor in §19.15's story who would hold a copy.
    let account_id = h.sole_account_id().await;
    match h.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO library_items (id, account_id, work_id, source_key, source_work_key, \
                 title, author_text, status, source_url, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 'Held', '', 'complete', 'https://example.test/held', ?6, ?6)",
            )
            .bind(&id)
            .bind(&account_id)
            .bind(work_id)
            .bind(site)
            .bind(source_work_key)
            .bind(&now)
            .execute(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("seed a library item");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO library_items (id, account_id, work_id, source_key, source_work_key, \
                 title, author_text, status, source_url, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, 'Held', '', 'complete', 'https://example.test/held', $6, $6)",
            )
            .bind(&id)
            .bind(&account_id)
            .bind(work_id)
            .bind(site)
            .bind(source_work_key)
            .bind(&now)
            .execute(h.db().postgres_pool().expect("postgres"))
            .await
            .expect("seed a library item");
        }
    }
}

/// Create a real `works` row and return its id.
///
/// A real row rather than a bare uuid because `library_items.work_id` is a
/// foreign key and enforcement is on. `owner_pseud_id` is NOT NULL (it is what
/// makes a work attributable), so the fixture inserts a pseud to own it — an
/// inserted pseud rather than a signed-in one keeps this independent of which
/// account the test happens to use.
async fn real_work_id(h: &Harness) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let now = lorehaven_db::identity::now_rfc3339();
    // The signed-in account's own pseud: `works.owner_pseud_id` is NOT NULL, and
    // using the account the harness already created avoids a second fixture that
    // could fail for unrelated reasons.
    let account_id = h.sole_account_id().await;
    let pseud_sql = h.db().sql(
        "SELECT id FROM pseuds WHERE account_id = ?1 LIMIT 1",
        "SELECT id FROM pseuds WHERE account_id = ?1::uuid LIMIT 1",
    );
    let pseud_id: String = match h.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query_as::<_, (String,)>(&pseud_sql)
                .bind(&account_id)
                .fetch_one(h.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("the registered account has a pseud")
                .0
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query_as::<_, (String,)>(&pseud_sql)
                .bind(&account_id)
                .fetch_one(h.db().postgres_pool().expect("postgres"))
                .await
                .expect("the registered account has a pseud")
                .0
        }
    };
    let sql = h.db().sql(
        "INSERT INTO works (id, owner_pseud_id, title, summary, language, rating, visibility, \
         lifecycle, completion, show_public_ratings, created_at, updated_at, version) \
         VALUES (?1, ?2, 'Held Work', '', 'en', 'general', 'public', 'published', 'complete', 1, ?3, ?3, 1)",
        "INSERT INTO works (id, owner_pseud_id, title, summary, language, rating, visibility, \
         lifecycle, completion, show_public_ratings, created_at, updated_at, version) \
         VALUES (?1::uuid, ?2::uuid, 'Held Work', '', 'en', 'general', 'public', 'published', 'complete', 1, $3, $3, 1)",
    );
    match h.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(&pseud_id)
                .bind(&now)
                .execute(h.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("insert a work");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(&pseud_id)
                .bind(&now)
                .execute(h.db().postgres_pool().expect("postgres"))
                .await
                .expect("insert a work");
        }
    }
    id
}

#[tokio::test]
async fn a_signal_for_a_work_the_instance_does_not_hold_creates_a_demand_item() {
    let mut h = Harness::new("m1717c-demand").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable_as("instance-a").await;
    h.request(
        "POST",
        "/api/v1/exchange/signals",
        Some(signal_for("ao3", "12345")),
    )
    .await;
    let demand = lorehaven_db::exchange::list_latent_demand(h.db(), 10)
        .await
        .expect("list demand");
    assert_eq!(
        demand.len(),
        1,
        "a work we do not have is demand: {demand:?}"
    );
    assert_eq!(
        demand[0].work_id, "ao3:12345",
        "keyed by the external identity"
    );
}

#[tokio::test]
async fn a_signal_for_a_work_the_instance_already_holds_is_not_demand() {
    // §19.15's condition, and the reason the check exists: a demand item here
    // would ask the instance to acquire a work it has.
    let mut h = Harness::new("m1717c-held").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable_as("instance-a").await;
    seed_library_item(&h, "ao3", "12345", Some(&real_work_id(&h).await)).await;
    h.request(
        "POST",
        "/api/v1/exchange/signals",
        Some(signal_for("ao3", "12345")),
    )
    .await;
    let demand = lorehaven_db::exchange::list_latent_demand(h.db(), 10)
        .await
        .expect("list demand");
    assert!(
        demand.is_empty(),
        "we hold it, so it is not demand: {demand:?}"
    );
}

#[tokio::test]
async fn the_same_work_signalled_twice_by_one_instance_is_one_demand() {
    // §16.16.1: "re-signalling the same work adds no weight." The primary key
    // makes the row unique; this asserts the *weight* does not move, which a
    // key alone would not guarantee.
    let mut h = Harness::new("m1717c-repeat").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable_as("instance-a").await;
    h.request(
        "POST",
        "/api/v1/exchange/signals",
        Some(signal_for("ao3", "777")),
    )
    .await;
    // A second, differently-spelled signal for the same work: the demand key is
    // the site+id, not the content hash, so this must not add weight.
    let mut second = signal_for("ao3", "777");
    second["signals"][0]["title"] = json!("A Stranger's Work (2nd ed)");
    h.request("POST", "/api/v1/exchange/signals", Some(second))
        .await;
    let demand = lorehaven_db::exchange::list_latent_demand(h.db(), 10)
        .await
        .expect("list demand");
    assert_eq!(demand.len(), 1, "one work, one demand item: {demand:?}");
    assert_eq!(
        demand[0].signal_count, 1,
        "§16.16.1: re-signalling the same work adds no weight"
    );
}

#[tokio::test]
async fn a_cross_posted_work_held_under_any_identity_is_not_demand() {
    // A work cross-posted to three sites is one work and three identities. If the
    // instance holds it under the third, a signal naming the first is not
    // demand — which is why the route tests *every* site id rather than the
    // first.
    let mut h = Harness::new("m1717c-cross").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable_as("instance-a").await;
    seed_library_item(&h, "ffnet", "crossover-9", Some(&real_work_id(&h).await)).await;
    h.request(
        "POST",
        "/api/v1/exchange/signals",
        Some(json!({
            "version": 1,
            "signals": [{
                "site_ids": [
                    { "site": "ao3", "id": "aaa" },
                    { "site": "ffnet", "id": "crossover-9" },
                    { "site": "wattpad", "id": "bbb" },
                ],
                "title": "A Stranger's Work",
                "author_names": ["An Author"],
                "fandom": "Some Fandom",
                "tags": ["Slow Burn"],
                "characters": [],
                "relationships": [],
                "content_rating": "general",
                "language": "en",
            }],
        })),
    )
    .await;
    let demand = lorehaven_db::exchange::list_latent_demand(h.db(), 10)
        .await
        .expect("list demand");
    assert!(
        demand.is_empty(),
        "held under one of its three identities: {demand:?}"
    );
}

#[tokio::test]
async fn a_signal_carrying_no_external_identifier_creates_no_demand() {
    // There is no identity to deduplicate on, and a title is not an identity:
    // two different works share a title, and one work has a translated one.
    // Guessing would put phantom demand in the queue under a key we cannot
    // later prove wrong.
    let mut h = Harness::new("m1717c-noidentity").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable().await;
    h.request(
        "POST",
        "/api/v1/exchange/signals",
        Some(Harness::batch("A Work", &["Slow Burn"])),
    )
    .await;
    let demand = lorehaven_db::exchange::list_latent_demand(h.db(), 10)
        .await
        .expect("list demand");
    assert!(demand.is_empty(), "no identity, no demand: {demand:?}");
}

#[tokio::test]
async fn a_private_library_copy_does_not_count_as_the_instance_holding_the_work() {
    // A `library_items` row with a NULL `work_id` is a reader's private copy: the
    // instance has no work for it and cannot acquire one on the instance's
    // behalf. Asserted because the row is present and the check must still miss
    // it — the naive query omits the `work_id IS NOT NULL` guard.
    let mut h = Harness::new("m1717c-private").await;
    h.signed_in_at("reader@example.test", "reader", 1).await;
    h.enable_as("instance-a").await;
    seed_library_item(&h, "ao3", "9999", None).await;
    h.request(
        "POST",
        "/api/v1/exchange/signals",
        Some(signal_for("ao3", "9999")),
    )
    .await;
    let demand = lorehaven_db::exchange::list_latent_demand(h.db(), 10)
        .await
        .expect("list demand");
    assert_eq!(
        demand.len(),
        1,
        "a private copy is not the instance holding it: {demand:?}"
    );
}
