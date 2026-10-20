//! M53-03: the three §11.6 vault properties the 0006 schema did not carry —
//! origin binding, explicit consent, and an audit trail without secret contents.
//!
//! ## What is actually being tested
//!
//! Spec §11.6 lists eleven vault behaviours. Eight were already built and are
//! pinned by `crates/app/tests/milestone_6.rs`: pseud-scoping, encryption at
//! rest, plaintext never returned, brief decryption, default 30-day expiry,
//! shorter configurable expiry, revocation, and "deleting a credential does not
//! delete already imported copies". The three this suite covers are the ones with
//! **no schema behind them at all** — `kind`, `origin_host` and `consent_at` did
//! not exist, and neither did `credential_audit_events`.
//!
//! They also did not exist because of a defect worth recording. The first draft
//! of migration 0096 declared `CREATE TABLE IF NOT EXISTS source_credentials`
//! with the full plan schema. 0006 already creates that table, so `IF NOT
//! EXISTS` swallowed the conflict, the migration applied cleanly, and not one
//! column appeared. The plan's verification for that step — `sqlite3 :memory: <
//! 0096.sql`, "no error" — passed on a file that had done nothing at all.
//!
//! **So every assertion here is behavioural, and the first test is specifically
//! the one that would have failed against the old migration.** A syntax check on
//! a migration file is a gate that cannot fail, and this repo has now paid for
//! two of them (`docs/verification.md` records the first, a `cargo clippy
//! 2>/dev/null | grep -c` that counted an empty stdout).
//!
//! ## Both engines
//!
//! Everything here runs on SQLite and, when `DARKHOLD_TEST_PG_URL` is set, on
//! PostgreSQL. That is not ceremony: `credential_audit_events` takes a `UUID` in
//! the Postgres arm and a `TEXT` in the SQLite one, and the whole
//! pseud-scoping guarantee is a `WHERE pseud_id = ?` that would bind differently
//! if the cast were missing. The single-backend blind spot in `docs/goal.md` is
//! exactly this shape.

use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use darkhold_app::config::Config;
use darkhold_app::server::set_trust_proxy;
use darkhold_app::state::AppState;
use darkhold_db::imports::CredentialEvent;
use darkhold_db::{imports, Backend, Database, DatabaseConfig};
use darkhold_scrapers::registry::Registry;
use darkhold_scrapers::{
    AuthKind, Credentials, Fetcher, SourceAdapter, SourceCapabilities, SourceKey, SourceResult,
    SourceUnit, SourceWork,
};
use serde_json::json;
use test_support::TestClient;

/// The gated source's key. Not `ao3`, because this suite needs an adapter that
/// declares an authentication kind and all twelve shipped adapters declare
/// `AuthKind::None` — see `the_shipped_adapters_declare_no_authentication_at_all`.
const GATED: &str = "gated-source";

/// The host the gated adapter serves. Origin binding is checked against this
/// string, so it is also the value a reader's credential gets bound to.
const GATED_HOST: &str = "gated.example";

/// The source whose credential is tested against a *different* host.
const OTHER_HOST: &str = "lookalike.example";

// ---------------------------------------------------------------------------
// The gated adapter
// ---------------------------------------------------------------------------

/// A login-gated adapter that never touches the network.
///
/// Its only job is to declare an authentication kind and a host, so every
/// assertion here is about what the vault does with those two facts rather than
/// about parsing. A test adapter that reached a network would be a test that
/// fails on a plane.
struct GatedSource {
    kind: AuthKind,
}

#[async_trait::async_trait]
impl SourceAdapter for GatedSource {
    fn key(&self) -> SourceKey {
        SourceKey::new(GATED)
    }

    fn display_name(&self) -> &'static str {
        "A Gated Source"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            authentication: self.kind,
            ..SourceCapabilities::public_read()
        }
    }

    fn hosts(&self) -> Vec<String> {
        vec![GATED_HOST.to_owned()]
    }

    fn can_handle(&self, url: &url::Url) -> bool {
        url.host_str() == Some(GATED_HOST)
    }

    async fn preview(
        &self,
        _fetch: &dyn Fetcher,
        url: &url::Url,
        _creds: Option<&Credentials>,
    ) -> SourceResult<SourceWork> {
        Ok(SourceWork {
            source_key: self.key(),
            source_work_key: url.path().to_owned(),
            source_url: url.to_string(),
            title: "T".to_owned(),
            author_text: "A".to_owned(),
            author_url: None,
            summary: String::new(),
            word_count: None,
            language: None,
            status: darkhold_scrapers::WorkStatus::Unknown,
            published_at: None,
            updated_at: None,
            units: vec![darkhold_scrapers::UnitRef {
                ordinal: 1,
                source_unit_key: "1".to_owned(),
                title: "One".to_owned(),
            }],
            rating_text: None,
            warning_texts: Vec::new(),
            tags: Vec::new(),
        })
    }

    async fn fetch_units(
        &self,
        _fetch: &dyn Fetcher,
        _work: &SourceWork,
        _creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceUnit>> {
        Ok(Vec::new())
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    // `test_support::scratch_dir` rather than a local copy: it is keyed per *call*
    // rather than per tag, because it opens by removing the directory, and a
    // per-tag key means a second harness with one tag deletes the first
    // harness's database out from under a live connection. That defect is real
    // and is pinned in that crate's own test suite.
    test_support::scratch_dir(&format!("m53-{tag}"))
}

/// A `Config` whose `database` points at the database `TestDb` actually opened.
///
/// This used to hardcode `sqlite://{dir}/darkhold.sqlite?mode=rwc`, which
/// `find-single-backend-suites.py` flagged as "hardcoded sqlite:// in a file that
/// claims both backends" -- correctly, and for a reason worth stating rather than
/// silencing.
///
/// `TestDb::connect_with_dir` HONOURS the backend selector: with
/// `DARKHOLD_TEST_PG_URL` set it creates and migrates a scratch PostgreSQL
/// DATABASE and ignores `dir` entirely (`let _ = &dir;`). So on the PostgreSQL leg
/// this file handed `AppState::new` a config pointing at a SQLite file that does not
/// exist, next to a live PostgreSQL pool. The suite was therefore SQLite-only in
/// fact while its header claimed both engines, and nothing failed -- `AppState` takes
/// the pool as a separate argument and never opens the URL itself, so the wrong URL
/// sat there unread.
///
/// Reading the backend off the live `Database` is the fix: one source of truth, and
/// no way for the two to disagree.
fn config_for(dir: &Path, db: &Database) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.join("storage");
    // Built from the LIVE backend rather than hardcoded, so the config cannot name
    // a database that was never opened.
    //
    // Not `db.redacted_url()`: that replaces the password with `***`, which is right
    // for a log line and useless for connecting. What is wanted is the same URL with
    // its credentials intact -- the PG leg's scratch database name and the SQLite
    // leg's file both live in the harness, and the admin URL came from the
    // environment in the first place.
    config.database = DatabaseConfig::new(match db.backend() {
        Backend::Sqlite => format!("sqlite://{}/darkhold.sqlite?mode=rwc", dir.display()),
        Backend::Postgres => std::env::var("DARKHOLD_TEST_PG_URL").unwrap_or_default(),
    });
    config.rate_limits.auth = darkhold_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.write = darkhold_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.search = darkhold_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.default = darkhold_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config
}

struct Harness {
    dir: PathBuf,
    tdb: test_support::TestDb,
}

impl Harness {
    async fn new(tag: &str, kind: AuthKind) -> Self {
        set_trust_proxy(false);
        let _ = darkhold_app::logging::init(&darkhold_app::config::LoggingConfig {
            filter: "error".to_owned(),
            format: darkhold_app::config::LogFormat::Pretty,
        });

        let dir = scratch_dir(tag);
        let tdb = test_support::TestDb::connect_with_dir(tag, &dir).await;
        let harness = Self { dir, tdb };
        harness.seed_source(kind).await;
        harness
    }

    async fn seed_source(&self, kind: AuthKind) {
        imports::upsert_source(self.tdb.db(), GATED, "A Gated Source", "0.1.0", "{}")
            .await
            .expect("seed source");
        let capability_json = json!({
            "authentication": match kind {
                AuthKind::None => "none",
                AuthKind::Token => "token",
                AuthKind::Password => "password",
                AuthKind::SessionCookie => "session_cookie",
            },
            "hosts": [GATED_HOST],
        })
        .to_string();
        imports::upsert_source(
            self.tdb.db(),
            GATED,
            "A Gated Source",
            "0.1.0",
            &capability_json,
        )
        .await
        .expect("record the source's capabilities");
    }

    fn state(&self) -> AppState {
        let mut registry = Registry::new();
        registry.register(Box::new(GatedSource {
            kind: AuthKind::Password,
        }));
        AppState::new(config_for(&self.dir, self.tdb.db()), self.tdb.db().clone())
            .with_registry(registry)
    }

    async fn cleanup(self) {
        self.tdb.cleanup().await;
        let _ = std::fs::remove_dir_all(self.dir);
    }
}

/// A signed-in reader, and the pseud the credential routes are scoped to.
///
/// The pseud rather than the account is returned because spec §11.6 scopes the
/// vault to the pseud, and a test that used the account id would be testing a
/// different reader than the one the route resolves. `test_support::register`
/// goes through the real registration route, so the session cookie and CSRF
/// token here are the ones the app issued.
async fn signed_in(harness: &Harness, email: &str, handle: &str) -> (TestClient, String) {
    let mut client = TestClient::new(darkhold_app::server::build_router(harness.state()));
    test_support::register(&mut client, email, handle).await;

    let (status, me) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let pseud = me["pseud"]["id"]
        .as_str()
        .or_else(|| me["pseuds"][0]["id"].as_str())
        .expect("/auth/me names the pseud the credential routes resolve")
        .to_owned();

    (client, pseud)
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

/// The test that would have failed against the first draft of migration 0096.
///
/// It asserts the three columns *behave* — a stored credential round-trips with
/// the values it was given — rather than asserting the migration file parses.
/// `CREATE TABLE IF NOT EXISTS` on a table 0006 already created satisfies every
/// syntax-level check and produces a database where all three of these columns
/// do not exist, which is what this test exists to have caught.
#[tokio::test]
async fn the_vault_columns_exist_and_round_trip() {
    let harness = Harness::new("columns", AuthKind::Password).await;
    let (_client, pseud) = signed_in(&harness, "columns@example.org", "columns").await;

    let secret_id = store_secret(&harness, &pseud, "the-secret-value").await;
    let (row, _previous) = imports::upsert_source_credential(
        harness.tdb.db(),
        &pseud,
        GATED,
        &secret_id,
        "a label",
        None,
        Some("password"),
        Some(GATED_HOST),
        Some("2026-09-30T12:00:00Z"),
    )
    .await
    .expect("store a credential with every M53-03 column");

    let read_back = imports::get_source_credential(harness.tdb.db(), &row.id)
        .await
        .expect("read it back")
        .expect("it exists");

    assert_eq!(
        read_back.kind.as_deref(),
        Some("password"),
        "`kind` did not round-trip: the column may not exist"
    );
    assert_eq!(
        read_back.origin_host.as_deref(),
        Some(GATED_HOST),
        "`origin_host` did not round-trip: the column may not exist"
    );
    assert_eq!(
        read_back.consent_at.as_deref(),
        Some("2026-09-30T12:00:00Z"),
        "`consent_at` did not round-trip: the column may not exist"
    );

    harness.cleanup().await;
}

/// §11.6: "Origin-bound credential use." A credential stored for one host is
/// refused for another, and the refusal names both hosts.
///
/// The second host is the one this property exists for: a cookie pasted for a
/// lookalike domain. Without the check the secret is simply attached to the
/// request, which is how one site's credential becomes another site's problem.
#[test]
fn a_credential_bound_to_one_host_is_refused_for_another() {
    let credential = credential_row(
        Some("password"),
        Some(GATED_HOST),
        Some("2026-09-30T12:00:00Z"),
    );

    let refusal = credential
        .usable_for(OTHER_HOST, NOW)
        .expect_err("a credential for another host must be refused");
    assert_eq!(
        refusal,
        imports::CredentialRefusal::WrongHost {
            bound: GATED_HOST.to_owned(),
            asked: OTHER_HOST.to_owned(),
        }
    );

    // §11.6 wants an *actionable* status, and the message is the requirement.
    let message = refusal.message();
    assert!(
        message.contains(GATED_HOST) && message.contains(OTHER_HOST),
        "the refusal must name both hosts, or the reader cannot tell which is \
         wrong: {message}"
    );
    assert!(
        !message.contains("hunter2"),
        "and it must not carry the secret: {message}"
    );
}

/// A host is case-insensitive by definition, so a credential for `GATED.EXAMPLE`
/// is a credential for `gated.example`. A comparison that called those different
/// would refuse a legitimate credential over a spelling.
#[test]
fn host_comparison_is_case_insensitive() {
    let credential = credential_row(
        Some("password"),
        Some("GATED.Example"),
        Some("2026-09-30T12:00:00Z"),
    );
    assert!(
        credential.usable_for("gated.example", NOW).is_ok(),
        "a host differs from itself only by case"
    );
}

/// §11.6: "Password or session-cookie storage requires explicit consent."
#[test]
fn a_password_without_a_consent_record_is_refused() {
    for kind in ["password", "session_cookie"] {
        let credential = credential_row(Some(kind), Some(GATED_HOST), None);

        let refusal = credential
            .usable_for(GATED_HOST, NOW)
            .expect_err("a {kind} with no consent record must be refused");
        assert_eq!(
            refusal,
            imports::CredentialRefusal::Unconsented {
                kind: kind.to_owned()
            },
            "{kind} needs consent; a token does not"
        );
        assert!(
            refusal.message().to_lowercase().contains("consent"),
            "the refusal must name the fix: {}",
            refusal.message()
        );
    }
}

/// A token needs no consent under §11.6 — "prefer source-issued tokens" is the
/// same sentence that requires consent for passwords — so gating one would gate
/// the method the spec recommends.
#[test]
fn a_token_needs_no_consent_record() {
    let credential = credential_row(Some("token"), Some(GATED_HOST), None);
    assert!(
        credential.usable_for(GATED_HOST, NOW).is_ok(),
        "a token is served without a consent record"
    );
}

/// An unbound credential is refused rather than tidied into a bound one. We
/// cannot reconstruct which host it was created for, and guessing would be
/// exactly the failure the requirement prevents.
#[test]
fn an_unbound_credential_is_refused_for_every_host() {
    let credential = credential_row(Some("token"), None, None);

    let refusal = credential
        .usable_for(GATED_HOST, NOW)
        .expect_err("a credential with no recorded host must be refused");
    assert!(matches!(
        refusal,
        imports::CredentialRefusal::Unbound { .. }
    ));
    assert!(
        refusal
            .message()
            .contains("POST /api/v1/source-credentials"),
        "the fix is to store it again, and the message has to say so: {}",
        refusal.message()
    );
}

/// An unknown kind requires consent. This is the direction a new `AuthKind` has
/// to earn: the alternative is that adding a fourth variant silently starts
/// serving secrets nobody consented to.
#[test]
fn an_unrecognised_kind_requires_consent() {
    for kind in [None, Some("something-new")] {
        let credential = credential_row(kind, Some(GATED_HOST), None);
        assert!(
            credential.usable_for(GATED_HOST, NOW).is_err(),
            "{kind:?} must be treated as consent-requiring"
        );
    }
}

/// A revoked or expired credential is refused on the way in, before origin or
/// consent are even considered — otherwise the refusal a reader sees depends on
/// which column happened to be checked first.
#[test]
fn an_unusable_credential_is_refused_before_anything_else() {
    let mut credential = credential_row(Some("password"), Some(OTHER_HOST), None);
    credential.status = "revoked".to_owned();
    assert_eq!(
        credential.usable_for(GATED_HOST, NOW),
        Err(imports::CredentialRefusal::Unusable {
            status: "revoked".to_owned()
        }),
        "a revoked credential must not reach the origin or consent checks"
    );
}

/// §11.6: "Audit events without secret contents."
///
/// The table is checked for a place to put a secret rather than for the absence
/// of a known secret, because the second can only ever find the secrets we
/// already know about. There is no column whose name suggests free text.
#[tokio::test]
async fn the_audit_table_has_nowhere_to_put_a_secret() {
    let harness = Harness::new("audit-schema", AuthKind::Password).await;
    let (_client, _pseud) = signed_in(&harness, "schema@example.org", "schema").await;

    let columns = table_columns(harness.tdb.db(), "credential_audit_events").await;
    assert_eq!(
        columns,
        vec!["credential_id", "event", "id", "occurred_at", "pseud_id"],
        "the audit table's columns changed; a new column is a new chance to \
         store a secret in it and must be argued for"
    );

    harness.cleanup().await;
}

/// An audit row is written when a credential is stored, and it names no secret.
#[tokio::test]
async fn storing_a_credential_records_a_created_event() {
    let harness = Harness::new("audit-created", AuthKind::Password).await;
    let (_client, pseud) = signed_in(&harness, "created@example.org", "created").await;

    let secret_id = store_secret(&harness, &pseud, "hunter2-do-not-log").await;
    let (row, _previous) = imports::upsert_source_credential(
        harness.tdb.db(),
        &pseud,
        GATED,
        &secret_id,
        "a label",
        None,
        Some("password"),
        Some(GATED_HOST),
        Some("2026-09-30T12:00:00Z"),
    )
    .await
    .expect("store");

    imports::record_credential_event(harness.tdb.db(), &row.id, &pseud, CredentialEvent::Created)
        .await
        .expect("record the creation");

    let events = imports::list_credential_events(harness.tdb.db(), &row.id)
        .await
        .expect("list");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event, "created");
    assert_eq!(events[0].pseud_id, pseud);

    // No arm of this row can carry the secret, and the check is against the
    // serialised row rather than a field access, so a *new* field holding it
    // would be caught.
    let serialised = serde_json::to_string(&events[0]).expect("serialise");
    assert!(
        !serialised.contains("hunter2"),
        "an audit row carried the secret: {serialised}"
    );

    harness.cleanup().await;
}

/// The revocation trail outlives the credential it is about. §11.6 asks for both
/// "immediate revocation" and "audit events", and a cascade would satisfy the
/// first by destroying the evidence for the second — the `revoked` row is
/// deleted by the very act it records.
#[tokio::test]
async fn a_revocation_is_still_on_record_after_the_credential_is_gone() {
    let harness = Harness::new("audit-revoked", AuthKind::Password).await;
    let (_client, pseud) = signed_in(&harness, "revoked@example.org", "revoked").await;

    let secret_id = store_secret(&harness, &pseud, "the-secret").await;
    let (row, _previous) = imports::upsert_source_credential(
        harness.tdb.db(),
        &pseud,
        GATED,
        &secret_id,
        "a label",
        None,
        Some("password"),
        Some(GATED_HOST),
        Some("2026-09-30T12:00:00Z"),
    )
    .await
    .expect("store");

    imports::record_credential_event(harness.tdb.db(), &row.id, &pseud, CredentialEvent::Revoked)
        .await
        .expect("record the revocation");

    assert!(
        imports::delete_source_credential(harness.tdb.db(), &row.id, &pseud)
            .await
            .expect("revoke")
            .is_some(),
        "the credential itself is gone"
    );

    let events = imports::list_credential_events(harness.tdb.db(), &row.id)
        .await
        .expect("the trail is still readable");
    assert_eq!(
        events.iter().map(|e| e.event.as_str()).collect::<Vec<_>>(),
        vec!["revoked"],
        "revoking a credential must not delete the record that it was revoked"
    );

    harness.cleanup().await;
}

/// One reader cannot write an audit row against another's credential. §11.6's
/// audit clause would leak the existence and timing of somebody else's login
/// otherwise.
#[tokio::test]
async fn an_audit_event_cannot_be_written_against_another_readers_credential() {
    let harness = Harness::new("audit-cross-pseud", AuthKind::Password).await;
    let (_c1, mine) = signed_in(&harness, "mine@example.org", "mine").await;
    let (_c2, theirs) = signed_in(&harness, "theirs@example.org", "theirs").await;

    let secret_id = store_secret(&harness, &mine, "the-secret").await;
    let (row, _previous) = imports::upsert_source_credential(
        harness.tdb.db(),
        &mine,
        GATED,
        &secret_id,
        "a label",
        None,
        Some("password"),
        Some(GATED_HOST),
        Some("2026-09-30T12:00:00Z"),
    )
    .await
    .expect("store");

    let result =
        imports::record_credential_event(harness.tdb.db(), &row.id, &theirs, CredentialEvent::Used)
            .await;
    assert!(
        result.is_err(),
        "a reader wrote an audit row against another reader's credential"
    );
    assert!(
        imports::list_credential_events(harness.tdb.db(), &row.id)
            .await
            .expect("list")
            .is_empty(),
        "and the refused write left a row behind"
    );

    harness.cleanup().await;
}

/// An audit row cannot be invented for a credential that does not exist.
///
/// `credential_id` is deliberately not a foreign key (see migration 0096), so
/// nothing but the store stops a row naming a credential that never was — and a
/// permanent audit row for something that never happened is a false record an
/// operator would later have to disprove.
#[tokio::test]
async fn an_audit_event_cannot_be_written_for_a_credential_that_does_not_exist() {
    let harness = Harness::new("audit-ghost", AuthKind::Password).await;
    let (_client, pseud) = signed_in(&harness, "ghost@example.org", "ghost").await;

    let result = imports::record_credential_event(
        harness.tdb.db(),
        "00000000-0000-0000-0000-000000000000",
        &pseud,
        CredentialEvent::Used,
    )
    .await;
    assert!(
        result.is_err(),
        "an audit row was written for a credential that was never stored"
    );

    harness.cleanup().await;
}

/// Every event the store can name is one the schema accepts. The store's enum
/// and the migration's `CHECK` are two spellings of the same list, and nothing
/// makes them agree except this.
#[tokio::test]
async fn every_credential_event_the_store_accepts_is_accepted_by_the_schema() {
    let harness = Harness::new("audit-events", AuthKind::Password).await;
    let (_client, pseud) = signed_in(&harness, "events@example.org", "events").await;

    let secret_id = store_secret(&harness, &pseud, "the-secret").await;
    let (row, _previous) = imports::upsert_source_credential(
        harness.tdb.db(),
        &pseud,
        GATED,
        &secret_id,
        "a label",
        None,
        Some("password"),
        Some(GATED_HOST),
        Some("2026-09-30T12:00:00Z"),
    )
    .await
    .expect("store");

    for event in CredentialEvent::ALL {
        imports::record_credential_event(harness.tdb.db(), &row.id, &pseud, event)
            .await
            .unwrap_or_else(|error| panic!("the schema refused {}: {error}", event.as_str()));
    }

    let events = imports::list_credential_events(harness.tdb.db(), &row.id)
        .await
        .expect("list");
    assert_eq!(events.len(), CredentialEvent::ALL.len());

    harness.cleanup().await;
}

/// §11.6: "Re-consent for renewal." A renewal is a fresh consent, so the
/// timestamp is rewritten rather than carried forward — a rotation that kept the
/// old value would satisfy the word "consent" and none of its intent.
#[tokio::test]
async fn a_renewal_rewrites_the_consent_rather_than_carrying_it_forward() {
    let harness = Harness::new("renew", AuthKind::Password).await;
    let (_client, pseud) = signed_in(&harness, "renew@example.org", "renew").await;

    let first = store_secret(&harness, &pseud, "first-secret").await;
    let (row, previous) = imports::upsert_source_credential(
        harness.tdb.db(),
        &pseud,
        GATED,
        &first,
        "a label",
        None,
        Some("password"),
        Some(GATED_HOST),
        Some("2026-01-01T00:00:00Z"),
    )
    .await
    .expect("the first store");
    assert!(previous.is_none(), "nothing was there to replace");

    let second = store_secret(&harness, &pseud, "second-secret").await;
    let (_row, _previous) = imports::upsert_source_credential(
        harness.tdb.db(),
        &pseud,
        GATED,
        &second,
        "a label",
        None,
        Some("password"),
        Some(GATED_HOST),
        Some("2026-09-30T00:00:00Z"),
    )
    .await
    .expect("the renewal");

    let after = imports::get_source_credential(harness.tdb.db(), &row.id)
        .await
        .expect("read")
        .expect("it exists");
    assert_eq!(
        after.consent_at.as_deref(),
        Some("2026-09-30T00:00:00Z"),
        "the renewal carried the old consent forward instead of re-recording it"
    );

    harness.cleanup().await;
}

/// §11.6: "No automatic copying across pseuds." Two identities on one account
/// are separate people as far as the vault is concerned, so a credential stored
/// for one is not available to the other — and the schema's uniqueness is on
/// `(pseud_id, source_key, label)`, which permits exactly this.
///
/// This is the assertion the plan's `UNIQUE (pseud_id, source_key)` would have
/// made *impossible to express*: under that constraint the second insert fails,
/// and a reader with two logins for one source could not have both. The property
/// is a store-layer refusal, not a schema constraint, and this pins which it is.
#[tokio::test]
async fn a_credential_is_not_copied_to_another_pseud_on_its_own() {
    let harness = Harness::new("cross-pseud", AuthKind::Password).await;
    let (_c1, first_pseud) = signed_in(&harness, "first@example.org", "first").await;
    let (_c2, second_pseud) = signed_in(&harness, "second@example.org", "second").await;

    let secret_id = store_secret(&harness, &first_pseud, "the-secret").await;
    let (row, _previous) = imports::upsert_source_credential(
        harness.tdb.db(),
        &first_pseud,
        GATED,
        &secret_id,
        "a label",
        None,
        Some("password"),
        Some(GATED_HOST),
        Some("2026-09-30T00:00:00Z"),
    )
    .await
    .expect("store under the first pseud");

    // The second pseud's listing does not include it. Nothing copied it: there
    // is no code path from one store to another, and this is the observable
    // consequence of there not being one.
    let others = imports::list_source_credentials(harness.tdb.db(), &second_pseud, None)
        .await
        .expect("list the second pseud's credentials");
    assert!(
        others.is_empty(),
        "a credential appeared under a pseud that never asked for one: {others:?}"
    );

    // And the store refuses to record an audit event against it under the other
    // pseud, which is the write-side half of the same property.
    assert!(
        imports::record_credential_event(
            harness.tdb.db(),
            &row.id,
            &second_pseud,
            CredentialEvent::Used
        )
        .await
        .is_err(),
        "the second pseud could write against the first's credential"
    );

    harness.cleanup().await;
}

/// The consent sweep finds exactly the pseuds holding an un-consented credential.
///
/// §11.6's "re-consent for renewal" is only enforceable if something asks the
/// question; this is that something, and a sweep that returned nothing would
/// make the partial index worthless in the one case it exists for.
#[tokio::test]
async fn the_consent_sweep_finds_the_pseudonyms_that_need_re_consenting() {
    let harness = Harness::new("sweep", AuthKind::Password).await;
    // Handles are 3-32 characters; a two-character one is refused at
    // registration and the failure looks like a sweep bug rather than a typo.
    let (_c1, consented) = signed_in(&harness, "okay@example.org", "consented-one").await;
    let (_c2, unconsented) = signed_in(&harness, "gap@example.org", "unconsented-one").await;

    let ok_secret = store_secret(&harness, &consented, "s").await;
    imports::upsert_source_credential(
        harness.tdb.db(),
        &consented,
        GATED,
        &ok_secret,
        "a label",
        None,
        Some("password"),
        Some(GATED_HOST),
        Some("2026-09-30T00:00:00Z"),
    )
    .await
    .expect("store consented");

    let gap_secret = store_secret(&harness, &unconsented, "s").await;
    imports::upsert_source_credential(
        harness.tdb.db(),
        &unconsented,
        GATED,
        &gap_secret,
        "a label",
        None,
        Some("password"),
        Some(GATED_HOST),
        None,
    )
    .await
    .expect("store un-consented");

    let needing = imports::pseuds_with_unconsented_credentials(harness.tdb.db())
        .await
        .expect("the sweep");
    assert_eq!(
        needing,
        vec![unconsented],
        "the sweep must find the pseud with no consent record and nothing else"
    );

    harness.cleanup().await;
}

/// The un-consented token row is still swept, because the column is the record
/// and a missing record is the sweep's business whatever the kind was. A token
/// is *usable* without consent; it is not *recorded* as consented.
#[tokio::test]
async fn a_token_is_usable_without_consent_but_still_has_none_recorded() {
    let credential = credential_row(Some("token"), Some(GATED_HOST), None);
    assert!(
        credential.usable_for(GATED_HOST, NOW).is_ok(),
        "a token is served without a consent record"
    );
    assert!(
        !credential.is_consented(),
        "and it must not claim a consent record it was never given"
    );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// An instant inside every credential's 30-day window in this file.
const NOW: &str = "2026-09-30T12:00:00Z";

/// A credential row with the three M53-03 fields set as given, everything else
/// healthy. Built here rather than read from a database because these tests are
/// about the *decision*, and a database round trip would only add a way for them
/// to pass for the wrong reason.
fn credential_row(
    kind: Option<&str>,
    origin_host: Option<&str>,
    consent_at: Option<&str>,
) -> imports::SourceCredential {
    imports::SourceCredential {
        id: "00000000-0000-0000-0000-000000000001".to_owned(),
        pseud_id: "00000000-0000-0000-0000-000000000002".to_owned(),
        source_key: GATED.to_owned(),
        secret_id: "00000000-0000-0000-0000-000000000003".to_owned(),
        label: "a label".to_owned(),
        status: "active".to_owned(),
        expires_at: Some("2026-12-31T00:00:00Z".to_owned()),
        last_checked_at: None,
        created_at: "2026-09-01T00:00:00Z".to_owned(),
        updated_at: "2026-09-01T00:00:00Z".to_owned(),
        version: 1,
        kind: kind.map(str::to_owned),
        origin_host: origin_host.map(str::to_owned),
        consent_at: consent_at.map(str::to_owned),
    }
}

/// Seal a secret the way the route does, so the store's foreign key is satisfied.
async fn store_secret(harness: &Harness, pseud: &str, plaintext: &str) -> String {
    let cipher = darkhold_app::secrets::load_cipher(&harness.dir.join("storage"), None, false)
        .expect("the development key");
    darkhold_app::secrets::seal_secret(
        harness.tdb.db(),
        &cipher,
        "source_credential",
        &format!("{pseud}\u{1f}{GATED}\u{1f}a label"),
        "secret",
        &darkhold_app::secrets::Secret::new(plaintext),
    )
    .await
    .expect("seal")
}

/// A table's column names, sorted, from whichever engine the test is running on.
///
/// Written as a query rather than a `PRAGMA` so the same assertion runs on
/// PostgreSQL, where `information_schema` is the only spelling. A SQLite-only
/// version of this test would leave the Postgres arm of the schema unexamined,
/// which is the blind spot `docs/goal.md` names.
async fn table_columns(db: &darkhold_db::Database, table: &str) -> Vec<String> {
    let mut names = raw_table_columns(db, table).await;
    // Sorted here rather than in the assertion, because `pragma_table_info` and
    // `information_schema` do not agree on ordinal order — the first is column
    // order, the second is alphabetical — and a test that compared them
    // positionally would fail on whichever engine happened to run second.
    names.sort();
    names
}

/// A table's columns, in whatever order the engine reports them.
async fn raw_table_columns(db: &darkhold_db::Database, table: &str) -> Vec<String> {
    match db.backend() {
        darkhold_db::Backend::Sqlite => {
            let sql = format!("SELECT name FROM pragma_table_info('{table}')");
            let rows: Vec<(String,)> = sqlx::query_as(&sql)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await
                .expect("read the column list");
            rows.into_iter().map(|(name,)| name).collect()
        }
        darkhold_db::Backend::Postgres => {
            let rows: Vec<(String,)> = sqlx::query_as(
                "SELECT column_name FROM information_schema.columns
                 WHERE table_name = $1 ORDER BY column_name",
            )
            .bind(table)
            .fetch_all(db.postgres_pool().expect("postgres handle"))
            .await
            .expect("read the column list");
            rows.into_iter().map(|(name,)| name).collect()
        }
    }
}
