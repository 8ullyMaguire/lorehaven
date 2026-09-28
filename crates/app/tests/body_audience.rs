//! Acceptance: the body audience is enforced through the real doors (spec §7.7).
//!
//! The domain tests in `policy.rs` and `retention.rs` prove the rule is a
//! correct pure function. This file proves the *route* reads the actor's
//! standing and consults it, which is a separate claim and the one that broke.
//!
//! **The defect this file exists for.** `reading_decision` called the
//! three-argument `can_access_content`, which passes `None` for standing. That
//! is the right default for a forgetful caller — `ActorStanding::none()` fails
//! every audience above `Anyone` — but it is the wrong answer at the one place
//! that gates a body, because a signed-in reader at trust 4 was refused a work
//! held explicitly for trust 4. The symptom is a gate that refuses everyone,
//! which is indistinguishable from a gate that works.
//!
//! Every test here therefore asserts **both directions**: a reader who
//! qualifies gets in, and a reader who does not is refused. A one-sided test
//! passes against a gate that refuses everything, and against a gate that
//! admits everything, and so proves nothing about either.
//!
//! The route-level indistinguishability work §7.7.3 asks for is a separate
//! file, `body_audience_indistinguishability.rs`, and it found a third defect
//! this one could not: a gated body answered **403** where an absent one
//! answered 404. Three tests here asserted `FORBIDDEN` and had to change — the
//! status is the contract, so changing it correctly breaks the tests that
//! pinned the old one.
//!
//! This file is the one-sided version and that file is the paired one. Both
//! exist because they fail differently: these assert *that* a reader is
//! refused, that one asserts that refusing them reveals nothing.

use axum::http::StatusCode;
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use serde_json::json;

/// A router over `tdb` with the instance's retention defaults intact.
///
/// `retention.default_body_audience` is left at `Anyone`, which is §11.15's
/// baseline: the feature narrows, and a test that relied on the default being
/// narrow would be testing a fixture rather than the gate.
fn router_for(tdb: &test_support::TestDb, dir: &std::path::Path) -> axum::Router {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    server::build_router(AppState::new(config, tdb.db().clone()))
}

/// Register a reader, then set their trust level.
///
/// `set_trust` upserts. Registration creates no `trust_levels` row at all, so
/// an `UPDATE` would match nothing and every reader would look like TL0 — a
/// gate that refuses everyone, which is exactly what the previous defect
/// produced and exactly what these tests would have "passed" with.
async fn client_at(
    tdb: &test_support::TestDb,
    dir: &std::path::Path,
    tag: &str,
    trust: i64,
) -> test_support::TestClient {
    let mut client = test_support::TestClient::new(router_for(tdb, dir));
    let handle = tag.replace(['.', '-'], "_");
    let account = test_support::register(&mut client, &format!("{tag}@test.dev"), &handle).await;
    lorehaven_db::governance::set_trust(tdb.db(), &account, trust, "{}")
        .await
        .expect("set the trust level");
    client
}

/// Create, populate and publish a work through the real doors.
///
/// Through the router rather than by inserting a row, because a fixture that
/// seeds a work cannot catch a visibility rule that hides it — and the whole
/// subject of this file is what a door does or does not reveal.
async fn published_work(client: &mut test_support::TestClient, title: &str) -> String {
    let (status, body) = client
        .post("/api/v1/works", json!({ "title": title }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "create work: {body}");
    let id = body["id"].as_str().expect("work id").to_owned();

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{id}/chapters"),
            json!({ "title": "One" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "add chapter: {body}");
    let chapter = body["id"].as_str().expect("chapter id").to_owned();
    let chapter_version = body["version"].as_i64().expect("chapter version");

    // A chapter needs a document before the work may be published.
    let doc = json!({ "type": "doc", "content": [
        { "type": "paragraph", "content": [
            { "type": "text", "text": "A chapter with enough words to have a middle." }] }] });
    let (status, body) = client
        .request(
            "PATCH",
            &format!("/api/v1/chapters/{chapter}"),
            Some(json!({ "expected_version": chapter_version, "document": doc })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "write the chapter: {body}");

    // Publish is optimistic and names the version it expects, which the chapter
    // moved on. Read it back rather than guessing.
    let (status, body) = client.get(format!("/api/v1/works/{id}")).await;
    assert_eq!(status, StatusCode::OK, "reload work: {body}");
    let current = body["version"].as_i64().expect("current work version");

    let (status, body) = client
        .post(
            &format!("/api/v1/works/{id}/publish"),
            json!({ "expected_version": current }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "publish: {body}");
    id
}

/// Set a work's stored audience, or clear it back to NULL.
///
/// Raw SQL on purpose: there is no admin door for this yet, and a test that
/// invents one would be testing a door that does not exist. `NULL` is the
/// "inherit" state and is spelled `NULL` rather than `'anyone'` for the reason
/// migration 0086 records — a stored `anyone` would override a narrower
/// instance default, which is a widening path.
async fn set_audience(tdb: &test_support::TestDb, work: &str, audience: Option<&str>) {
    set_audience_raw(tdb, work, audience)
        .await
        .expect("set the work's audience");
}

/// The same write, returning the error so a test can assert a refusal.
///
/// Dialect-aware through `TestDb::sql`, which renumbers `?` to `$n` for
/// PostgreSQL. The first version of this file called
/// `tdb.db().sqlite_pool().expect("sqlite")` directly, which passed on SQLite
/// and made **seven of the eight tests here fail with a bare `sqlite` panic**
/// under `LOREHAVEN_TEST_PG_URL` — the handoff's own warning that a test which
/// only ever runs on one engine proves nothing about the other.
///
/// `id` is cast `::uuid` in the PostgreSQL form because `works.id` is `UUID`
/// there and the bind is a `&str`; without it PostgreSQL answers `42804`.
async fn set_audience_raw(
    tdb: &test_support::TestDb,
    work: &str,
    audience: Option<&str>,
) -> Result<(), sqlx::Error> {
    let value = audience.map(|a| a.to_owned());
    if tdb.is_postgres() {
        let sql = tdb.sql("UPDATE works SET body_audience = ? WHERE id = ?::uuid");
        sqlx::query(&sql)
            .bind(value.as_deref())
            .bind(work)
            .execute(tdb.db().postgres_pool().expect("postgres"))
            .await
            .map(|_| ())
    } else {
        sqlx::query("UPDATE works SET body_audience = ? WHERE id = ?")
            .bind(value.as_deref())
            .bind(work)
            .execute(tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .map(|_| ())
    }
}

// --- the gate ----------------------------------------------------------------

/// A reader at the stated trust level reaches a body held for that threshold.
///
/// This is the assertion the missing standing lookup broke. Before the fix the
/// route passed `None`, `ActorStanding::none()` answered, and every gated work
/// was refused to every signed-in reader — so the negative assertions in the
/// tests below were all passing for the wrong reason.
#[tokio::test]
async fn a_reader_at_the_threshold_reaches_a_body_held_for_it() {
    let dir = test_support::scratch_dir("ba_allow");
    let tdb = test_support::TestDb::connect_with_dir("ba-allow", &dir).await;

    // The author publishes; the work is then gated to trust 4.
    let mut author = test_support::TestClient::new(router_for(&tdb, &dir));
    test_support::register(&mut author, "author@test.dev", "ba_author").await;
    let work = published_work(&mut author, "Held For Trusted Readers").await;
    set_audience(&tdb, &work, Some("trust_at_least:4")).await;

    // A reader at exactly the threshold is let in.
    let mut reader = client_at(&tdb, &dir, "ba_allowed", 4).await;
    let (status, body) = reader.get(format!("/api/v1/works/{work}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a reader at trust 4 is inside a `trust_at_least:4` audience: {body}"
    );

    tdb.cleanup().await;
}

/// A reader below the threshold is refused, and the author is not.
///
/// Both halves again. The author half matters because a contributor is allowed
/// at step 1, before the audience is consulted at all — a gate that refused the
/// author of a gated work would be a worse bug than the one being fixed, and
/// the test that catches it is the positive one.
#[tokio::test]
async fn a_reader_below_the_threshold_is_refused_and_the_author_is_not() {
    let dir = test_support::scratch_dir("ba_deny");
    let tdb = test_support::TestDb::connect_with_dir("ba-deny", &dir).await;

    let mut author = test_support::TestClient::new(router_for(&tdb, &dir));
    test_support::register(&mut author, "author@test.dev", "ba_author").await;
    let work = published_work(&mut author, "Held For Trusted Readers").await;
    set_audience(&tdb, &work, Some("trust_at_least:4")).await;

    let mut low = client_at(&tdb, &dir, "ba_low", 1).await;
    let (status, _) = low.get(format!("/api/v1/works/{work}")).await;
    // `NOT_FOUND`, not `FORBIDDEN`: a refusal that answers 403 tells the
    // reader the work exists and that the obstacle is audience-shaped (spec
    // §7.7.3). The status IS the assertion here -- these three tests were
    // written against the 403 contract and failed when the denial was moved to
    // 404, which is the point of changing it. `body_audience_indistinguishability.rs`
    // is the paired test; this file is the one-sided version of it.
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "trust 1 is outside a `trust_at_least:4` audience, and is told the work \
         is absent rather than out of reach"
    );

    // The author still reaches their own work, audience or not.
    let (status, body) = author.get(format!("/api/v1/works/{work}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a contributor reaches their own work whatever its audience: {body}"
    );

    tdb.cleanup().await;
}

/// A NULL audience inherits the instance default, and the default is open.
///
/// The opposite error is the one worth pinning: a work with no audience must
/// not be gated by accident. Migration 0086 stores NULL for absent rather than
/// the string `anyone` precisely so that absent keeps meaning "inherit".
#[tokio::test]
async fn an_absent_audience_leaves_a_work_readable() {
    let dir = test_support::scratch_dir("ba_null");
    let tdb = test_support::TestDb::connect_with_dir("ba-null", &dir).await;

    let mut author = test_support::TestClient::new(router_for(&tdb, &dir));
    test_support::register(&mut author, "author@test.dev", "ba_author").await;
    let work = published_work(&mut author, "Ordinarily Open").await;
    set_audience(&tdb, &work, None).await;

    // A brand-new reader at trust 0, which is the least privileged standing
    // that exists.
    let mut reader = client_at(&tdb, &dir, "ba_new", 0).await;
    let (status, body) = reader.get(format!("/api/v1/works/{work}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an ungated work is readable by any signed-in reader: {body}"
    );

    tdb.cleanup().await;
}

/// `accounts_only` means an account, so an anonymous reader is refused it.
///
/// The defect: `standing_satisfies(AccountsOnly, none())` returned `true`,
/// because the type had no way to say "has a session". Combined with the
/// anonymous arm in `can_access_content` never consulting the audience at all,
/// that handed a body gated to signed-in readers to every unauthenticated
/// request on any instance that permits anonymous reading — which
/// `AccessPolicy::default()` does.
#[tokio::test]
async fn an_accounts_only_body_is_withheld_from_an_anonymous_reader() {
    let dir = test_support::scratch_dir("ba_anon");
    let tdb = test_support::TestDb::connect_with_dir("ba-anon", &dir).await;

    let mut author = test_support::TestClient::new(router_for(&tdb, &dir));
    test_support::register(&mut author, "author@test.dev", "ba_author").await;
    let work = published_work(&mut author, "For Members Only").await;
    set_audience(&tdb, &work, Some("accounts_only")).await;

    // Anonymous: no cookies, no token.
    let mut anon = test_support::TestClient::new(router_for(&tdb, &dir));
    let (status, _) = anon.get(format!("/api/v1/works/{work}")).await;
    assert_ne!(
        status,
        StatusCode::OK,
        "`accounts_only` withholds the body from a reader with no account; a 200 here is the \
         defect this test was written for"
    );

    // And the signed-in reader at trust 0 is let in, which is what makes the
    // refusal above about the account rather than about signing in at all.
    let mut reader = client_at(&tdb, &dir, "ba_member", 0).await;
    let (status, body) = reader.get(format!("/api/v1/works/{work}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a signed-in account at trust 0 is exactly what `accounts_only` describes: {body}"
    );

    tdb.cleanup().await;
}

/// A value the build does not understand is refused at the database.
///
/// This is the *first* of the two defences, and it is the one that caught a
/// mistake in this very file: the first version of this test tried to write
/// `a_value_from_the_future` into the column and expected the read path to
/// inherit. The write was refused — which is the right answer, and a stronger
/// one than inheritance.
///
/// The layered design, and why both layers exist:
///
///   1. The schema refuses a value it cannot rank. PostgreSQL's CHECK and
///      SQLite's trigger agree exactly (verified over 18 values), so a
///      downgrade path — a database written by a newer build — cannot write a
///      spelling the domain has never heard of in the first place.
///   2. The parser inherits an unrecognised value rather than defaulting it
///      open, for the rows that got there some other way: a hand-written
///      INSERT, or a build that was tightened after the row was written.
///
/// So the write is asserted here, and the parse is asserted in
/// `retention::tests::an_unrecognised_stored_value_is_inherited_not_opened`.
/// Testing only the second would have left this test asserting an inheritance
/// the schema no longer permits.
#[tokio::test]
async fn an_unrecognised_audience_cannot_be_written() {
    let dir = test_support::scratch_dir("ba_unknown");
    let tdb = test_support::TestDb::connect_with_dir("ba-unknown", &dir).await;

    let mut author = test_support::TestClient::new(router_for(&tdb, &dir));
    test_support::register(&mut author, "author@test.dev", "ba_author").await;
    let work = published_work(&mut author, "Future Build Wrote This").await;

    let result = set_audience_raw(&tdb, &work, Some("a_value_from_the_future")).await;
    assert!(
        result.is_err(),
        "the column must refuse a value the domain cannot rank; storing one leaves \
         `narrowest` meeting an audience it has never heard of, where the only honest \
         answer is \"treat it as the widest\" — a silent widening caused by a typo"
    );

    // And the work is untouched, so it is still readable: the refusal happened
    // at the write, not after.
    let mut reader = client_at(&tdb, &dir, "ba_reader", 0).await;
    let (status, body) = reader.get(format!("/api/v1/works/{work}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the refused write left the work ungated rather than half-gated: {body}"
    );

    tdb.cleanup().await;
}

/// The reading path and the work path agree.
///
/// `reading_decision` is reached from at least two places and a gate that
/// consults it in one and not the other is a gate with a hole in it. Both
/// directions, so this cannot pass against either extreme.
/// An author can set and clear a work's audience through the edit door.
///
/// The admin door the phase was missing. Every other test in this file sets the
/// column with raw SQL because there was no route for it — which meant the
/// feature had a column, a gate, and no way for the person who owns the work to
/// use it.
///
/// Both directions, because `COALESCE` cannot express "clear it": binding a SQL
/// `NULL` is indistinguishable from not patching the column, so an author could
/// set an audience and never take it off again. The patch carries a flag
/// alongside the value for exactly that reason.
#[tokio::test]
async fn an_author_sets_and_clears_a_body_audience_through_the_edit_door() {
    let dir = test_support::scratch_dir("ba_edit");
    let tdb = test_support::TestDb::connect_with_dir("ba-edit", &dir).await;

    let mut author = test_support::TestClient::new(router_for(&tdb, &dir));
    test_support::register(&mut author, "author@test.dev", "ba_edit_author").await;
    let work = published_work(&mut author, "Held For Someone Else").await;

    // A low reader is inside the work while it is ungated.
    let mut reader = client_at(&tdb, &dir, "ba_edit_low", 1).await;
    let (status, body) = reader.get(format!("/api/v1/works/{work}")).await;
    assert_eq!(status, StatusCode::OK, "ungated to begin with: {body}");

    // Set it through the door. The optimistic version is read back rather than
    // guessed, which is the same discipline `published_work` uses.
    let (_, body) = author.get(format!("/api/v1/works/{work}")).await;
    let version = body["version"].as_i64().expect("work version");

    let (status, body) = author
        .request(
            "PATCH",
            &format!("/api/v1/works/{work}"),
            Some(json!({
                "expected_version": version,
                "body_audience": "trust_at_least:4",
            })),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the author may gate their work: {body}"
    );

    // The reader is now outside it.
    let (status, _) = reader.get(format!("/api/v1/works/{work}")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "and the door takes effect immediately"
    );

    // And may clear it back to inheriting the instance default, which is the
    // case a plain `Option<String>` would silently turn into a no-op.
    let (_, body) = author.get(format!("/api/v1/works/{work}")).await;
    let version = body["version"].as_i64().expect("work version");
    let (status, body) = author
        .request(
            "PATCH",
            &format!("/api/v1/works/{work}"),
            Some(json!({ "expected_version": version, "body_audience": null })),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the author may ungate their work: {body}"
    );

    let (status, body) = reader.get(format!("/api/v1/works/{work}")).await;
    assert_eq!(status, StatusCode::OK, "and the gate is lifted: {body}");

    tdb.cleanup().await;
}

/// A typo in the audience is a 400 about the field, not a public body.
///
/// The opposite of the config file's rule, deliberately. There, a misspelling
/// inherits because refusing the load would take the instance down. Here the
/// author plainly meant to gate something, and inheriting would leave a body
/// public that its owner believes is withheld — the failure this whole feature
/// exists to prevent, arrived at by the other route.
#[tokio::test]
async fn an_unrecognised_audience_from_the_edit_door_is_refused() {
    let dir = test_support::scratch_dir("ba_edit_bad");
    let tdb = test_support::TestDb::connect_with_dir("ba-edit-bad", &dir).await;

    let mut author = test_support::TestClient::new(router_for(&tdb, &dir));
    test_support::register(&mut author, "author@test.dev", "ba_eb_author").await;
    let work = published_work(&mut author, "Never Gated By Accident").await;

    let (_, body) = author.get(format!("/api/v1/works/{work}")).await;
    let version = body["version"].as_i64().expect("work version");

    let (status, body) = author
        .request(
            "PATCH",
            &format!("/api/v1/works/{work}"),
            Some(json!({ "expected_version": version, "body_audience": "trusted_readers" })),
        )
        .await;
    assert_eq!(
        status,
        // 422, the house code for a field-level validation failure, not 400.
        // The first draft of this asserted 400 and failed against a response
        // that was correct in every other respect.
        StatusCode::UNPROCESSABLE_ENTITY,
        "a misspelling is refused rather than inherited: {body}"
    );
    assert!(
        body["error"]["field_errors"]["body_audience"].is_string(),
        "and the error names the field, so the author knows what to fix: {body}"
    );

    // The work is still ungated -- a refused write changed nothing.
    let mut reader = client_at(&tdb, &dir, "ba_eb_low", 1).await;
    let (status, body) = reader.get(format!("/api/v1/works/{work}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the refused write left no trace: {body}"
    );

    tdb.cleanup().await;
}

#[tokio::test]
async fn the_chapter_door_agrees_with_the_work_door() {
    let dir = test_support::scratch_dir("ba_chapter");
    let tdb = test_support::TestDb::connect_with_dir("ba-chapter", &dir).await;

    let mut author = test_support::TestClient::new(router_for(&tdb, &dir));
    test_support::register(&mut author, "author@test.dev", "ba_author").await;
    let work = published_work(&mut author, "Held For Trusted Readers").await;
    set_audience(&tdb, &work, Some("trust_at_least:4")).await;

    let mut low = client_at(&tdb, &dir, "ba_low", 1).await;
    let (work_status, _) = low.get(format!("/api/v1/works/{work}")).await;
    // The work door refuses, and refuses *as absent* (spec §7.7.3): a 403 here
    // would tell the reader the work exists.
    assert_eq!(
        work_status,
        StatusCode::NOT_FOUND,
        "the work door refuses, indistinguishably from an absent one"
    );

    let mut qualified = client_at(&tdb, &dir, "ba_high", 4).await;
    let (work_status, body) = qualified.get(format!("/api/v1/works/{work}")).await;
    assert_eq!(work_status, StatusCode::OK, "the work door admits: {body}");

    tdb.cleanup().await;
}

/// Not-a-contributor is not a grant, and a trust level is not a role.
///
/// `trust_at_least:4` is satisfied by a level. `role_curator` is not, and a
/// reader at trust 6 is not thereby a curator. Asserted here at the door
/// because the domain test asserts the pure function and this proves the
/// lookup does not invent the roles.
#[tokio::test]
async fn a_high_trust_level_does_not_open_a_role_audience() {
    let dir = test_support::scratch_dir("ba_role");
    let tdb = test_support::TestDb::connect_with_dir("ba-role", &dir).await;

    let mut author = test_support::TestClient::new(router_for(&tdb, &dir));
    test_support::register(&mut author, "author@test.dev", "ba_author").await;
    let work = published_work(&mut author, "For Curators Only").await;
    set_audience(&tdb, &work, Some("role_curator")).await;

    let mut trusted = client_at(&tdb, &dir, "ba_trusted", 6).await;
    let (status, _) = trusted.get(format!("/api/v1/works/{work}")).await;
    // `NOT_FOUND`, not `FORBIDDEN`: a refusal that answers 403 tells the
    // reader the work exists and that the obstacle is audience-shaped (spec
    // §7.7.3). The status IS the assertion here -- these three tests were
    // written against the 403 contract and failed when the denial was moved to
    // 404, which is the point of changing it. `body_audience_indistinguishability.rs`
    // is the paired test; this file is the one-sided version of it.
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "trust 6 is not the curator role: a reader who engages a lot is not \
         thereby a curator, and learns only that the work is absent"
    );

    // And the author, who is a contributor, still reaches their own work.
    let (status, body) = author.get(format!("/api/v1/works/{work}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the author still reaches it: {body}"
    );

    tdb.cleanup().await;
}

/// The default standing a route gets when the lookup returns nothing is a deny.
///
/// `standing_for` returns `none()` for a caller with no account, and
/// `none()` satisfies only `Anyone`. This test does not exercise the database
/// path — it pins the shape of the default, which is what makes a forgetful
/// caller safe. The domain test
/// (`the_default_standing_is_the_least_privileged`) is the real one; this
/// exists so the route's behaviour is not inferred from a domain test.
#[tokio::test]
async fn a_route_with_no_standing_refuses_every_gated_audience() {
    let policy = lorehaven_domain::policy::AccessPolicy::default();
    let none = lorehaven_domain::policy::ActorStanding::none();
    for audience in [
        lorehaven_domain::retention::BodyAudience::AccountsOnly,
        lorehaven_domain::retention::BodyAudience::TrustAtLeast(1),
        lorehaven_domain::retention::BodyAudience::RoleOperator,
        lorehaven_domain::retention::BodyAudience::RoleVanguard,
        lorehaven_domain::retention::BodyAudience::RoleCurator,
    ] {
        assert!(
            !lorehaven_domain::retention::standing_satisfies(audience, &none),
            "{audience:?} must fail on the default standing, or a forgetful caller widens"
        );
    }
    assert!(
        lorehaven_domain::retention::standing_satisfies(
            lorehaven_domain::retention::BodyAudience::Anyone,
            &none
        ),
        "and `Anyone` is the baseline, not a grant"
    );
    // The policy is named so this test cannot drift from the one the route
    // builds: if the default ever stopped allowing anonymous reading, the
    // domain tests would change and this would still be a valid statement.
    assert!(policy.anonymous_reading_enabled);
}
