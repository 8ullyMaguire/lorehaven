//! Acceptance: the decision disclosure on `/api/v1/meta` (amendment §3.4).
//!
//! §0.4.3 asks an instance to disclose what it is that decides; §11.15's
//! retention setting is already reported by `/meta` and
//! `body_audience_indistinguishability.rs` pins that it stays. The decision
//! provider is the same kind of fact and belongs in the same place.
//!
//! The three claims here, and the reason each needs its own test:
//!
//! - **A deterministic instance says `"deterministic"`.** Not an omitted field.
//!   A missing field is indistinguishable from an old server, a client that
//!   renders "unknown" for it has learned nothing, and a client that renders
//!   "the instance does not filter" has been told something false about an
//!   instance that filters with its own classifiers.
//! - **A calibrated instance says `"calibrated"`, to every reader, signed in or
//!   not.** A reader whose comments are filtered by a local model is owed the
//!   knowledge that a model did it, and `/meta` is the one endpoint an
//!   integration reads before it renders anything.
//! - **The thresholds travel with the provider.** An operator who has moved
//!   `accept_threshold` has changed what the instance holds to be a work, and
//!   that is a setting a reader is being graded against.

use std::path::Path;

use axum::http::StatusCode;
use lorehaven_app::config::{Config, DecisionProvider};
use lorehaven_app::server;
use lorehaven_app::state::AppState;

/// A router whose `[decisions]` section is whatever this test sets.
///
/// Not `Config::development_defaults()` alone, because the point of two of
/// these tests is a *configured* provider, and a config built in code cannot
/// reach the file loader that is the other half of the feature.
fn router_with(config: Config, tdb: &test_support::TestDb) -> axum::Router {
    server::build_router(AppState::new(config, tdb.db().clone()))
}

fn scratch(tag: &str) -> std::path::PathBuf {
    test_support::scratch_dir(tag)
}

fn config_for(dir: &Path) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config
}

/// An instance nobody has opted in discloses the deterministic provider, with
/// its thresholds, rather than saying nothing.
///
/// The `body.is_null()` half is the load-bearing one. `/meta` is a public,
/// unauthenticated endpoint that an integration reads before it renders
/// anything, and a client that receives no `decisions` block has no way to
/// distinguish "this instance grades with its own rules" from "this server is
/// too old to tell me" — and the second reading is a reason to guess, which is
/// the failure a disclosure exists to prevent.
#[tokio::test]
async fn an_instance_that_configured_nothing_still_discloses_a_provider() {
    let dir = scratch("decisions-disclosure-default");
    let tdb = test_support::TestDb::connect_with_dir("decisions-disclosure-default", &dir).await;
    let config = config_for(&dir);

    // The premise: the default really is deterministic. Asserted rather than
    // assumed, because every assertion below is about the default case.
    assert_eq!(
        config.decisions.provider,
        DecisionProvider::Deterministic,
        "an instance that has not opted in must behave as it did before the \
         feature existed"
    );

    let mut client = test_support::TestClient::new(router_with(config, &tdb));
    let (status, body) = client.get("/api/v1/meta").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert!(
        !body["decisions"].is_null(),
        "the field is absent rather than reported: {body}"
    );
    assert_eq!(
        body["decisions"]["provider"],
        serde_json::json!("deterministic"),
        "a reader must be told the instance grades with its own classifiers, \
         not left to infer it: {body}"
    );
    assert_eq!(
        body["decisions"]["accept_threshold"]
            .as_f64()
            .expect("a number"),
        0.90,
        "the threshold is reported even under `deterministic`, because a \
         client that special-cases its absence would otherwise imply a number \
         that is not in force: {body}"
    );
    tdb.cleanup().await;
}

/// A calibrated instance says so, to an anonymous reader.
///
/// The claim is about *who* can see it. An operator-only disclosure would leave
/// the readers it exists for — the ones being graded — exactly as uninformed as
/// before, because a reader who is not an operator never visits an operator
/// surface. `MaybeSession` on the handler is what makes this test pass: the
/// field is in the response for a caller with no session at all.
#[tokio::test]
async fn a_calibrated_instance_says_so_to_a_reader_who_is_not_signed_in() {
    let dir = scratch("decisions-disclosure-calibrated");
    let tdb = test_support::TestDb::connect_with_dir("decisions-disclosure-calibrated", &dir).await;

    let mut config = config_for(&dir);
    config.decisions.provider = DecisionProvider::Calibrated;
    // An operator who has tuned it, because the tuned value is what a reader is
    // being graded against and the default would not prove the field is live.
    config.decisions.accept_threshold = 0.75;
    config.decisions.consult_floor = 0.05;

    // No registration, no login: this client has never been signed in.
    let mut anonymous = test_support::TestClient::new(router_with(config, &tdb));
    let (status, body) = anonymous.get("/api/v1/meta").await;
    assert_eq!(status, StatusCode::OK, "{status}: {body}");
    assert_eq!(
        body["decisions"]["provider"],
        serde_json::json!("calibrated"),
        "a reader being graded by a model is owed the knowledge, and /meta is \
         the one endpoint they are guaranteed to read: {body}"
    );
    assert_eq!(
        body["decisions"]["accept_threshold"]
            .as_f64()
            .expect("a number"),
        0.75,
        "the operator's tuned threshold is what the instance is holding works \
         to, so it is what a reader is told: {body}"
    );
    assert_eq!(
        body["decisions"]["consult_floor"]
            .as_f64()
            .expect("a number"),
        0.05,
        "{body}"
    );
    tdb.cleanup().await;
}

/// The disclosure never carries a model name, a host, or a key.
///
/// A disclosure of *which provider* is public information an instance owes its
/// readers. A disclosure of *where it runs* is an internal detail, and one that
/// would help a reader probe a service the operator never meant to expose. The
/// audit endpoint's `base_url` is for the operator's own logs; `/meta` is for
/// everybody, and "everybody" includes a reader who would like to know which
/// loopback port answers.
///
/// The key is the sharper half: a key in a public, unauthenticated response is
/// a published credential, and the test asserts on the whole serialised body
/// rather than on named fields precisely so a future field cannot slip past.
#[tokio::test]
async fn the_disclosure_names_the_provider_and_nothing_about_where_it_runs() {
    let dir = scratch("decisions-disclosure-no-leak");
    let tdb = test_support::TestDb::connect_with_dir("decisions-disclosure-no-leak", &dir).await;

    let mut config = config_for(&dir);
    config.decisions.provider = DecisionProvider::Calibrated;
    // A distinctive base URL and a distinctive key, so the test fails if either
    // reaches the response rather than if some generic substring appears.
    config.decisions.base_url = "http://198.51.100.7:9999".to_owned();
    config.decisions.api_key = Some("sk-unsloth-MUST-NOT-APPEAR".to_owned());

    let mut client = test_support::TestClient::new(router_with(config, &tdb));
    let (status, body) = client.get("/api/v1/meta").await;
    assert_eq!(status, StatusCode::OK, "{status}: {body}");

    assert_eq!(
        body["decisions"]["provider"],
        serde_json::json!("calibrated"),
        "naming the provider IS the disclosure: {body}"
    );

    // Scoped to the `decisions` block, and that scoping is the point rather than
    // a convenience. `/meta` ALREADY carries a top-level `base_url` — the
    // instance's own public address, which is the frontend's to render. A test
    // that scanned the whole body for the word "base_url" fails against a
    // pre-existing, correct field, and a test that scans for "localhost" fails
    // against the development default for the same reason. Both would be
    // wrong, and both would be fixed by deleting the assertion -- so the
    // assertion is scoped to the block this feature added, and the distinctive
    // values from the config are what it looks for.
    let decisions = body["decisions"].to_string();
    for forbidden in [
        "MUST-NOT-APPEAR", // the configured key
        "sk-unsloth",      // any key at all
        "198.51.100.7",    // the configured model host
        "9999",            // its port
        "base_url",        // the field name itself
        "api_key",         // and the key field's name
        "model",           // the model name is an operator's business
        "timeout",         // and so is its timeout
    ] {
        assert!(
            !decisions.contains(forbidden),
            "the `decisions` block names the provider and nothing about where it \
             runs, so it must carry no `{forbidden}`: {decisions}"
        );
    }

    // The three fields it DOES carry, pinned. A disclosure that grew a fourth
    // field would pass every assertion above.
    let object = body["decisions"]
        .as_object()
        .expect("the disclosure is an object");
    let mut keys: Vec<&String> = object.keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        vec!["accept_threshold", "consult_floor", "provider"],
        "exactly these three, no more: {body}"
    );
    tdb.cleanup().await;
}

/// The `body_mode` disclosure is untouched by this one.
///
/// Stated as its own test because `body_audience_indistinguishability.rs` pins
/// `body_mode` on `/meta` as an operator-and-public configuration fact that
/// "must stay". Adding a `decisions` block next to it is a change to a
/// response other tests assert on, so the neighbour is checked rather than
/// assumed to have survived.
#[tokio::test]
async fn the_retention_disclosure_beside_it_is_untouched() {
    let dir = scratch("decisions-disclosure-neighbour");
    let tdb = test_support::TestDb::connect_with_dir("decisions-disclosure-neighbour", &dir).await;

    let mut config = config_for(&dir);
    config.decisions.provider = DecisionProvider::Calibrated;

    let mut client = test_support::TestClient::new(router_with(config, &tdb));
    let (status, body) = client.get("/api/v1/meta").await;
    assert_eq!(status, StatusCode::OK, "{status}: {body}");
    assert!(
        !body["policy"].is_null(),
        "the policy block is still here: {body}"
    );
    assert!(
        !body["site"].is_null() || !body["name"].is_null(),
        "the rest of the response is unchanged: {body}"
    );
    tdb.cleanup().await;
}
