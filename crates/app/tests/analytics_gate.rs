//! The gate at the HTTP boundary.
//!
//! The domain tests prove `allowed_under` is a correct pure function. This file
//! proves the *route* calls it, which is a different claim and the one that
//! actually matters: a gate nothing consults is a gate that protects nothing,
//! and the failure is a 200 where a 403 belongs.
//!
//! Three ways that goes wrong, all of them quiet:
//!
//! * the route hardcodes a level instead of asking the registry,
//! * the route checks the trust level but not the role, so a trustee-level
//!   reader reaches the financial dashboard,
//! * the route returns 404 rather than 403, so a caller cannot distinguish
//!   "denied" from "not implemented" and cannot tell whether a capability
//!   exists at all.
//!
//! The last one matters for privacy as much as for UX: a 404 that varies by
//! trust level is an oracle for what the instance has implemented.

use axum::http::StatusCode;
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;

/// A fresh router per request.
///
/// `Router` is consumed by `oneshot`, and `TestClient` re-uses its own copy
/// across several calls, so each request in these tests gets its own.
/// A router on an *archive* instance.
///
/// `development_defaults()` configures `curated_boutique`, which maps to
/// `Preset::Gallery` and withholds every `community.*` capability. That is the
/// gate working — a gallery has no community dashboards — but it means a test
/// written against the development preset is testing the wrong instance. The
/// preset is pinned here so the trust-ladder tests are about the ladder.
fn router_for(tdb: &test_support::TestDb, dir: &std::path::Path) -> axum::Router {
    router_for_preset(tdb, dir, "open_library")
}

fn router_for_preset(
    tdb: &test_support::TestDb,
    dir: &std::path::Path,
    preset: &str,
) -> axum::Router {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config.instance.preset = preset.to_owned();
    server::build_router(AppState::new(config, tdb.db().clone()))
}

/// Register a reader, then set their trust level.
///
/// The level is set *after* registration because registration is what a real
/// new account looks like, and a gate that only works for accounts inserted
/// by a fixture is not a gate. `register` returns the account id, so this does
/// not need to seed `accounts` itself.
async fn client_at(
    tdb: &test_support::TestDb,
    dir: &std::path::Path,
    tag: &str,
    trust: i64,
) -> test_support::TestClient {
    let mut client = test_support::TestClient::new(router_for(tdb, dir));
    // `validate_handle` rejects hyphens, so the tag is used as-is. A 422 here
    // reads like a broken auth flow rather than a broken fixture.
    let handle = tag.replace(['.', '-'], "_");
    let account = test_support::register(&mut client, &format!("{tag}@test.dev"), &handle).await;

    // `set_trust` upserts, and registration creates no trust row at all --
    // so an UPDATE here would match nothing and every reader would look like
    // TL0. That is exactly the bug this test suite would otherwise have
    // "passed" with: a gate that refuses everyone looks like a gate that
    // works.
    lorehaven_db::governance::set_trust(tdb.db(), &account, trust, "{}")
        .await
        .expect("set the trust level");
    client
}

/// Register a reader at `trust` and GET one capability.
async fn get_analytics(
    tdb: &test_support::TestDb,
    dir: &std::path::Path,
    tag: &str,
    trust: i64,
    capability: &str,
) -> (StatusCode, serde_json::Value) {
    let mut client = client_at(tdb, dir, tag, trust).await;
    client
        .get(format!("/api/v1/me/analytics/{capability}"))
        .await
}

// --- the gate ----------------------------------------------------------------

#[tokio::test]
async fn an_own_metric_is_reachable_at_its_own_level() {
    let dir = test_support::scratch_dir("gate_ok");
    let tdb = test_support::TestDb::connect_with_dir("gate-ok", &dir).await;

    let (status, body) = get_analytics(&tdb, &dir, "gate-ok-a", 0, "own.reading.basic").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    tdb.cleanup().await;
}

#[tokio::test]
async fn a_capability_above_the_viewers_level_is_refused() {
    // TL0 asking for a TL3 community capability. The refusal must be a
    // permission error, not a 404: a 404 that varies by trust level tells a
    // caller which capabilities this instance has implemented, and that is
    // the first half of an enumeration of what an operator is running.
    let dir = test_support::scratch_dir("gate_denied");
    let tdb = test_support::TestDb::connect_with_dir("gate-denied", &dir).await;

    let (status, body) =
        get_analytics(&tdb, &dir, "gate-denied-a", 0, "community.query_analysis").await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "expected 403, got {status}: {body}"
    );
    // And the body must not contain the number it refused to show.
    assert!(
        !body.to_string().contains("retention"),
        "the refusal echoed the metric: {body}"
    );

    tdb.cleanup().await;
}

#[tokio::test]
async fn the_same_capability_is_reachable_one_level_up() {
    // The denial above is only meaningful if the capability *works* for
    // someone. A gate that refuses everyone is not a gate.
    let dir = test_support::scratch_dir("gate_then");
    let tdb = test_support::TestDb::connect_with_dir("gate-then", &dir).await;

    let (status, body) =
        get_analytics(&tdb, &dir, "gate-then-a", 3, "community.query_analysis").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    tdb.cleanup().await;
}

#[tokio::test]
async fn an_unknown_capability_is_a_404_and_a_known_one_is_not() {
    // The distinction matters and is easy to collapse by accident: an unknown
    // name is a client bug, a known-but-denied name is a permission outcome.
    // Returning 403 for both would tell a prober that a name is real.
    let dir = test_support::scratch_dir("gate_unknown");
    let tdb = test_support::TestDb::connect_with_dir("gate-unknown", &dir).await;

    let mut client = client_at(&tdb, &dir, "gate-unknown-a", 6).await;
    let (status, _body) = client.get("/api/v1/me/analytics/not.a.capability").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an unknown name must be a 404"
    );

    tdb.cleanup().await;
}

#[tokio::test]
async fn a_forbidden_name_in_the_never_shown_list_is_a_404_at_every_level() {
    // The forbidden scopes are not capabilities, so no trust level reaches
    // them -- including a trustee and including admin. If any of these ever
    // returned 200, the anti-list would have stopped being structural.
    for name in [
        "ab.variant_assignment",
        "ab.signal_weights",
        "ab.resonance_numeric",
        "ab.shadowban_state",
        "ab.pseud_linkage",
        "ab.other_reading_history",
    ] {
        let dir = test_support::scratch_dir(&format!("gate_forb_{}", name.replace('.', "_")));
        let tdb = test_support::TestDb::connect_with_dir(&format!("gate-forb-{name}"), &dir).await;
        let mut client =
            client_at(&tdb, &dir, &format!("forb-{}", name.replace('.', "_")), 6).await;
        let (status, _body) = client.get(format!("/api/v1/me/analytics/{name}")).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{name} is reachable at TL6: the never-shown list is not structural"
        );
        tdb.cleanup().await;
    }
}

#[tokio::test]
async fn an_admin_only_capability_is_refused_to_a_trustee() {
    // A role grant, not a score. The trustee is the most senior non-admin
    // and must still not read the admin taste profile.
    let dir = test_support::scratch_dir("gate_admin");
    let tdb = test_support::TestDb::connect_with_dir("gate-admin", &dir).await;

    let mut client = client_at(&tdb, &dir, "gate-admin-a", 6).await;
    let (status, _body) = client.get("/api/v1/me/analytics/admin.taste_profile").await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a trustee reached the admin panel"
    );

    tdb.cleanup().await;
}

#[tokio::test]
async fn a_trustee_capability_is_refused_to_a_senior_reader() {
    // TL5 is not TL6 and does not carry the fiduciary role. The financial
    // dashboard is trustee business, not a reward for seniority.
    let dir = test_support::scratch_dir("gate_fid");
    let tdb = test_support::TestDb::connect_with_dir("gate-fid", &dir).await;

    let mut client = client_at(&tdb, &dir, "gate-fid-a", 5).await;
    let (status, _body) = client.get("/api/v1/me/analytics/trustee.financials").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "TL5 read aggregate revenue");

    tdb.cleanup().await;
}

#[tokio::test]
async fn the_response_states_the_floor_and_the_definition() {
    // A reader who is refused should be able to learn what would change the
    // answer, and a reader who is served should be able to check the number.
    // Both come from the registry, so neither is a per-route string.
    let dir = test_support::scratch_dir("gate_docs");
    let tdb = test_support::TestDb::connect_with_dir("gate-docs", &dir).await;

    let (status, body) = get_analytics(&tdb, &dir, "gate-docs-a", 0, "own.reading.basic").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let meta = &body["meta"];
    assert!(meta["definition"].is_string(), "{body}");
    assert!(meta["freshness"].is_string(), "{body}");
    assert!(meta["approximation"].is_string(), "{body}");
    assert!(meta["minimum_trust_level"].is_number(), "{body}");
    assert!(meta["subject"].is_string(), "{body}");

    tdb.cleanup().await;
}

#[tokio::test]
async fn the_own_reading_endpoint_lists_only_what_the_viewer_may_see() {
    // The dashboard renders by iterating this list. If it returned every
    // capability, the client would be the thing deciding what is allowed, and
    // the registry would be decoration.
    let dir = test_support::scratch_dir("gate_list");
    let tdb = test_support::TestDb::connect_with_dir("gate-list", &dir).await;

    let mut client = client_at(&tdb, &dir, "gate-list-a", 0).await;
    let (status, body) = client.get("/api/v1/me/analytics").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let names: Vec<String> = body["capabilities"]
        .as_array()
        .expect("capabilities")
        .iter()
        .map(|c| c["name"].as_str().unwrap_or_default().to_string())
        .collect();

    assert!(
        names.contains(&"own.reading.basic".to_string()),
        "{names:?}"
    );
    assert!(
        !names.contains(&"community.query_analysis".to_string()),
        "a TL0 list contains a TL3 capability: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.starts_with("ab.")),
        "a forbidden name reached the list: {names:?}"
    );
    // Every listed name must be a real capability, so a client can key on it.
    for n in &names {
        assert!(
            lorehaven_domain::analytics::Scope::parse(n).is_some(),
            "{n} is listed but is not a capability"
        );
    }

    tdb.cleanup().await;
}

#[tokio::test]
async fn a_higher_level_sees_a_superset() {
    // The ladder is a partial order and a reader can check it.
    let dir = test_support::scratch_dir("gate_super");
    let tdb = test_support::TestDb::connect_with_dir("gate-super", &dir).await;

    let mut names_by_level = Vec::new();
    for trust in [0i64, 2, 4] {
        let tag = format!("gate-super-{trust}");
        let mut client = client_at(&tdb, &dir, &tag, trust).await;
        let (status, body) = client.get("/api/v1/me/analytics").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let set: std::collections::BTreeSet<String> = body["capabilities"]
            .as_array()
            .expect("capabilities")
            .iter()
            .map(|c| c["name"].as_str().unwrap_or_default().to_string())
            .collect();
        names_by_level.push(set);
    }

    for pair in names_by_level.windows(2) {
        for name in &pair[0] {
            assert!(
                pair[1].contains(name),
                "{name} is visible at a lower level and not at a higher one"
            );
        }
    }
    assert!(
        names_by_level[2].len() > names_by_level[0].len(),
        "a steward sees no more than a new account"
    );

    tdb.cleanup().await;
}

#[tokio::test]
async fn the_response_never_contains_a_credential_or_an_identifier() {
    // The whole capability response is asserted key by key rather than by
    // "does it look right": a stray field is exactly how a session token or a
    // pseud id ends up in an analytics payload nobody reads.
    let dir = test_support::scratch_dir("gate_keys");
    let tdb = test_support::TestDb::connect_with_dir("gate-keys", &dir).await;

    let (status, body) = get_analytics(&tdb, &dir, "gate-keys-a", 0, "own.reading.basic").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    fn check(value: &serde_json::Value, path: &str) {
        match value {
            serde_json::Value::Object(o) => {
                for (k, v) in o {
                    let lower = k.to_ascii_lowercase();
                    for banned in [
                        "token", "secret", "password", "session", "email", "ip", "device",
                        "account", "pseud", "handle", "csrf", "key",
                    ] {
                        assert!(
                            !lower.contains(banned),
                            "{path}.{k} looks like an identifier or a credential"
                        );
                    }
                    check(v, &format!("{path}.{k}"));
                }
            }
            serde_json::Value::Array(a) => {
                for (i, v) in a.iter().enumerate() {
                    check(v, &format!("{path}[{i}]"));
                }
            }
            _ => {}
        }
    }
    check(&body, "$");

    tdb.cleanup().await;
}

#[tokio::test]
async fn a_gallery_instance_withholds_the_community_surface_to_everyone() {
    // Found by a test failing for the right reason: `development_defaults()`
    // configures `curated_boutique`, and a gallery has no community
    // dashboards. The TL3 reader was refused because the *instance* is
    // curated, not because the reader is untrusted.
    //
    // Worth its own test because it is the one case where a preset overrides a
    // trust level in the restrictive direction, and the amendment requires
    // exactly that and nothing more.
    let dir = test_support::scratch_dir("gate_gallery");
    let tdb = test_support::TestDb::connect_with_dir("gate-gallery", &dir).await;

    for trust in [0i64, 3, 6] {
        let tag = format!("gate-gallery-{trust}");
        let mut client =
            test_support::TestClient::new(router_for_preset(&tdb, &dir, "curated_boutique"));
        let account = test_support::register(&mut client, &format!("{tag}@test.dev"), &tag).await;
        lorehaven_db::governance::set_trust(tdb.db(), &account, trust, "{}")
            .await
            .unwrap();

        let (status, body) = client
            .get("/api/v1/me/analytics/community.query_analysis")
            .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a gallery served a community capability at TL{trust}: {body}"
        );

        // The personal surface stays: a reader's own history is theirs
        // regardless of how curated the instance is.
        let (status, body) = client.get("/api/v1/me/analytics/own.reading.basic").await;
        assert_eq!(
            status,
            StatusCode::OK,
            "a gallery refused personal stats: {body}"
        );
    }

    tdb.cleanup().await;
}

#[tokio::test]
async fn an_unrecognised_preset_does_not_silently_withhold_everything() {
    // A new preset name on an upgraded instance should not cost the operator
    // their analytics. The trust ladder still applies; only the preset's own
    // ceiling is skipped.
    let dir = test_support::scratch_dir("gate_preset_unknown");
    let tdb = test_support::TestDb::connect_with_dir("gate-preset-unknown", &dir).await;

    let mut client =
        test_support::TestClient::new(router_for_preset(&tdb, &dir, "a_preset_from_the_future"));
    let account = test_support::register(&mut client, "future@test.dev", "future_reader").await;
    lorehaven_db::governance::set_trust(tdb.db(), &account, 3, "{}")
        .await
        .unwrap();

    let (status, body) = client
        .get("/api/v1/me/analytics/community.query_analysis")
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an unknown preset withheld analytics: {body}"
    );

    // And the ladder is untouched.
    let mut tl0 =
        test_support::TestClient::new(router_for_preset(&tdb, &dir, "a_preset_from_the_future"));
    let a0 = test_support::register(&mut tl0, "future0@test.dev", "future_reader0").await;
    lorehaven_db::governance::set_trust(tdb.db(), &a0, 0, "{}")
        .await
        .unwrap();
    let (status, _body) = tl0
        .get("/api/v1/me/analytics/community.query_analysis")
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a preset raised a trust ceiling"
    );

    tdb.cleanup().await;
}
