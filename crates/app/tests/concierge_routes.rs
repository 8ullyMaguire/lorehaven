//! M45-22 — §54's concierge over HTTP (spec §54.7's acceptance lines).
//!
//! The domain and store tests prove the rules hold when the functions are called
//! directly. This file proves the ROUTES carry them, and it is the only place two
//! of §54.7's lines are observable at all:
//!
//! - "no selector returns the same works as discovery, in the same order." The
//!   session layer sits on top of the §16 blend, and the one way it could make the
//!   default path worse is by changing what a reader with no selector sees. Only a
//!   test that asks both routes can see that.
//! - "a selector matching nothing is explained, not replaced by a fallback."
//!
//! Every case runs on whichever engine the harness chose, so the suite is run
//! twice — see `docs/plans/REMAINING-2026-10-03.md` for the two invocations.

use axum::http::StatusCode;
use serde_json::{json, Value};

use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;

use test_support::{scratch_dir, TestClient, TestDb};

fn router_for(tdb: &TestDb, dir: &std::path::Path) -> axum::Router {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    server::build_router(AppState::new(config, tdb.db().clone()))
}

/// A registered reader. The concierge has no trust bar — it is the reader's own
/// queue — so no trust level is set here, and a fixture that assumed one would be
/// asserting a requirement §54 does not have.
/// Returns the client, the account id and the ACTIVE pseud id.
async fn reader(
    tdb: &TestDb,
    dir: &std::path::Path,
    tag: &str,
) -> (TestClient, String, Option<String>) {
    let mut client = TestClient::new(router_for(tdb, dir));
    let handle = tag.replace(['.', '-'], "_");
    let account = test_support::register(&mut client, &format!("{tag}@test.dev"), &handle).await;
    // The ACTIVE pseud, which is what `reading_history_entry` is keyed by.
    let pseud = test_support::active_pseud_id(&mut client, &account).await;
    (client, account, pseud)
}

/// Run one fixture statement, binding every value as text.
///
/// `?N#u` for a native uuid (TEXT on SQLite) and `?N#i` for an integer, matching
/// `preread_store.rs`'s convention. The markers are resolved per backend because a
/// `::uuid` cast is a syntax error on SQLite and a missing cast is a decode failure
/// on PostgreSQL.
///
/// Copied rather than invented: sqlx does NOT translate `?1` into `$1` for
/// PostgreSQL, and `TestDb` has no statement runner of its own. Getting that wrong
/// reaches the server as a literal `?` and fails with
/// `operator does not exist: ? integer`.
async fn exec(tdb: &TestDb, tmpl: &str, binds: &[&str]) {
    let pg = tdb.is_postgres();
    let sql = tmpl
        .replace("#u", if pg { "::uuid" } else { "" })
        .replace("#i", if pg { "::bigint" } else { "" })
        .replace("#t", if pg { "::timestamptz" } else { "" });

    // `?N` slot numbers are honoured here, NOT by `TestDb::sql`.
    //
    // `lorehaven_db::rewrite_placeholders` (db/src/lib.rs:462) walks the string and
    // renumbers every `?` it meets in order, ignoring the digits entirely. So a
    // template written as `VALUES (?1#u, ?, ?3#t)` comes out as `$1, $2, $3` and the
    // author's numbering is silently discarded — which is invisible until a slot is
    // REPEATED, at which point `?3#t, ?3#t` becomes `$3, $4` and the statement asks
    // for a fourth bind it was never given. PostgreSQL answers that with 42P18
    // "could not determine data type of parameter"; SQLite, which needs no rewrite,
    // answers "NOT NULL constraint failed: accounts.updated_at" instead. Two
    // unrelated-looking failures for one cause.
    //
    // So the rewrite happens here: bare `?` takes the next slot, `?N` takes slot N,
    // and the position index is `$N` on PostgreSQL and `?N` on SQLite. Repetition is
    // then free — `?3#t, ?3#t` is one bind, used twice — which is what the
    // fixtures mean by it.
    let mut out = String::with_capacity(sql.len() + 8);
    let mut next = 1usize;
    let mut chars = sql.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if ch != '?' {
            out.push(ch);
            continue;
        }
        // An explicit slot number, or the next one.
        let mut digits = String::new();
        while let Some((_, d)) = chars.peek() {
            if d.is_ascii_digit() {
                digits.push(*d);
                chars.next();
            } else {
                break;
            }
        }
        let slot: usize = if digits.is_empty() {
            let s = next;
            next += 1;
            s
        } else {
            let s = digits.parse().expect("a slot number");
            next = next.max(s + 1);
            s
        };
        if pg {
            out.push('$');
        } else {
            out.push('?');
        }
        out.push_str(&slot.to_string());
        let _ = idx;
    }

    // The casts still decide the TYPES, so a bare `?` beside a uuid column is
    // unambiguous. `#i` is `::bigint` rather than `::integer` because `word_count`
    // is BIGINT on PostgreSQL.
    if pg {
        let pool = tdb.db().postgres_pool().expect("postgres pool");
        let mut q = sqlx::query(&out);
        for b in binds {
            // A value that parses as a uuid is bound as a uuid; everything else is
            // text. Decided per BIND, because the cast in the SQL already tells
            // PostgreSQL which is which and a wrong Rust type here would be a decode
            // error rather than a coercion.
            match uuid::Uuid::parse_str(b) {
                Ok(u) => q = q.bind(u),
                Err(_) => q = q.bind(*b),
            }
        }
        q.execute(pool).await.expect("fixture insert");
    } else {
        let pool = tdb.db().sqlite_pool().expect("sqlite pool");
        let mut q = sqlx::query(&out);
        for b in binds {
            q = q.bind(*b);
        }
        q.execute(pool).await.expect("fixture insert");
    }
}

/// Give `count` works to the instance, published, complete and public, each with one
/// chapter of `words` words so a duration estimate exists.
///
/// The chapter and revision rows are not decoration. `duration_estimates` reads
/// `chapter_revisions.word_count`; a work with no chapters has NO estimate, and
/// §54.4 then charges it the queue's midpoint — correct behaviour, but it makes
/// every budget assertion in this file untestable, because the total stops being
/// predictable. So every work seeded here has a real length.
async fn seed_works(tdb: &TestDb, count: usize, words: u32) -> Vec<String> {
    const T0: &str = "2026-01-01T00:00:00Z";
    let mut ids = Vec::with_capacity(count);
    for n in 0..count {
        let account = uuid::Uuid::new_v4().to_string();
        let pseud = uuid::Uuid::new_v4().to_string();
        let work = uuid::Uuid::new_v4().to_string();
        let chapter = uuid::Uuid::new_v4().to_string();
        let revision = uuid::Uuid::new_v4().to_string();

        // One statement per table: sqlx refuses a multi-statement batch on
        // PostgreSQL, and this fixture runs on both engines.
        exec(
            tdb,
            "INSERT INTO accounts (id, status, email, age_state, created_at, updated_at, permission_statement)
             VALUES (?1#u, 'active', ?2, 'adult', ?3#t, ?3#t, 'minimal')",
            &[&account, &format!("seed-{account}@example.com"), T0],
        )
        .await;
        exec(
            tdb,
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
             VALUES (?1#u, ?2#u, ?3, ?3, ?4#t, ?4#t)",
            &[&pseud, &account, &format!("seed{n}{}", &work[..8]), T0],
        )
        .await;
        exec(
            tdb,
            "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, completion, created_at, updated_at, generated_content_posture)
             VALUES (?1#u, ?2#u, ?3, 'published', 'public', 'complete', ?4#t, ?4#t, 'forbid')",
            &[&work, &pseud, &format!("Seed work {n}"), T0],
        )
        .await;
        // The chapter goes in FIRST with a NULL `current_revision_id`, then the
        // revision, then the link is closed.
        //
        // Inserting them the other way round fails, and only on the engine that
        // enforces the FK: `chapters.current_revision_id` REFERENCES
        // `chapter_revisions(id)`, and SQLite does not check that on insert by
        // default while PostgreSQL always does. The chapter's own migration comment
        // notes the cycle ("SQLite resolves a foreign key target when it is used
        // rather than when it is declared") — which is exactly why the safe order is
        // the three-step one and not the two-step one that reads more naturally.
        exec(
            tdb,
            "INSERT INTO chapters (id, work_id, order_key, title, current_revision_id, created_at, updated_at)
             VALUES (?1#u, ?2#u, ?3#i, 'One', NULL, ?4#t, ?4#t)",
            &[&chapter, &work, "10", T0],
        )
        .await;
        // `created_by_pseud_id` is NOT NULL and FK-constrained: a revision records
        // who saved it, which is the pseud that owns the work here.
        exec(
            tdb,
            "INSERT INTO chapter_revisions (id, chapter_id, revision_number, document_json, sanitized_html, plain_text, word_count, created_by_pseud_id, created_at)
             VALUES (?1#u, ?2#u, 1, '{}', '<p>x</p>', 'x', ?3#i, ?4#u, ?5#t)",
            &[&revision, &chapter, &words.to_string(), &pseud, T0],
        )
        .await;
        exec(
            tdb,
            "UPDATE chapters SET current_revision_id = ?2#u WHERE id = ?1#u",
            &[&chapter, &revision],
        )
        .await;
        ids.push(work);
    }
    ids
}

/// Tag `work` with a mood node, so `moods_in_use` and `filter_by_mood` can see it.
///
/// `work_tags.weight` is 1 and `taxonomy_nodes.review_status` is `pending` because
/// neither query filters on either: `moods_in_use` selects `tn.norm` for nodes of
/// kind `mood` joined to a published work, and `filter_by_mood` compares that norm
/// against the selector. A confirmed-status node would be equally visible, and an
/// unconfirmed one equally invisible — so the value here is arbitrary and is
/// written down rather than left to look meaningful.
async fn tag_mood(tdb: &TestDb, work: &str, canonical: &str) {
    const T0: &str = "2026-01-01T00:00:00Z";
    let node = uuid::Uuid::new_v4().to_string();
    exec(
        tdb,
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at, review_status, signal_count)
         VALUES (?1#u, 'mood', ?2, ?3, ?4#t, 'pending', 0)",
        &[&node, canonical, &canonical.to_lowercase(), T0],
    )
    .await;
    exec(
        tdb,
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES (?1#u, ?2#u, 1, ?3#t)",
        &[work, &node, T0],
    )
    .await;
}

/// Stagger `updated_at`, which is what `public_recommendations` orders by
/// (discovery.rs:321). Without this the popularity engine's order is arbitrary and
/// the parity test is flaky rather than wrong — a worse failure, because a flaky
/// test is one people re-run.
async fn bump_updated_at(tdb: &TestDb, work: &str, n: usize) {
    let at = format!("2026-01-{:02}T00:00:00Z", n + 1);
    exec(
        tdb,
        "UPDATE works SET updated_at = ?2 WHERE id = ?1#u",
        &[work, &at],
    )
    .await;
}

/// Give `work` `count` five-star ratings, which is what the popularity engine's
/// signal reads. Readers are generated rather than registered: a rating's only
/// requirements are an account and a work, and registering twenty accounts to move
/// one row would make this fixture slow without making it stronger.
async fn add_ratings(tdb: &TestDb, work: &str, count: i64) {
    if count <= 0 {
        return;
    }
    for _ in 0..count {
        let account = uuid::Uuid::new_v4().to_string();
        let pseud = uuid::Uuid::new_v4().to_string();
        exec(
            tdb,
            "INSERT INTO accounts (id, status, email, age_state, created_at, updated_at, permission_statement)
             VALUES (?1#u, 'active', ?2, 'adult', ?3#t, ?3#t, 'minimal')",
            &[&account, &format!("rater-{account}@example.com"), "2026-01-01T00:00:00Z"],
        )
        .await;
        // `rating.pseud_id` is NOT NULL and `stars` is the column (CHECK between 1
        // and 5), and `is_public` defaults to 0 — a private rating is still a
        // rating, and the popularity aggregate counts either, so it is left at the
        // default rather than restated.
        exec(
            tdb,
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
             VALUES (?1#u, ?2#u, ?3, ?3, ?4#t, ?4#t)",
            &[
                &pseud,
                &account,
                &format!("rater{}", &account[..8]),
                "2026-01-01T00:00:00Z",
            ],
        )
        .await;
        exec(
            tdb,
            // `id` is an explicit column here because SQLite defaults it (a
            // rowid alias or a DEFAULT) while PostgreSQL has no default for it and
            // rejects the row with 23502. Naming it costs one bind and makes the
            // fixture identical on both engines.
            "INSERT INTO rating (id, account_id, pseud_id, work_id, stars, created_at, updated_at)
             VALUES (?1#u, ?2, ?3#u, ?4#u, 5, ?5#t, ?5#t)",
            &[
                &uuid::Uuid::new_v4().to_string(),
                &account,
                &pseud,
                work,
                "2026-01-01T00:00:00Z",
            ],
        )
        .await;
    }
}

/// Take a work out of the candidate list while leaving its moods registered.
///
/// The exclusion is `visibility`, not lifecycle: `public_recommendations` filters
/// on `lifecycle = 'published' AND visibility = 'public'` (discovery.rs:319), and
/// `moods_in_use` filters on lifecycle alone (concierge_store.rs:252). So a
/// published-but-unlisted work is exactly the one row shape where the two
/// disagree — validation can still see its moods, the blend never serves it.
///
/// Two other exclusions were tried and both are wrong:
///
/// - Lifecycle. Both sides check it, so demoting the work removes the mood from
///   `moods_in_use` too and the request comes back 422-unknown.
/// - Reading history. `seen_work_ids` only reaches the PLUGGABLE path
///   (`RecContext.seen`, rec_engine.rs:66); the default `rec_mode` is `legacy` and
///   `blend()` applies no seen-exclusion at all, so the work is served anyway, via
///   the popularity engine, whichever engine dropped it.
///
/// That missing exclusion in the legacy blend is a real defect and is NOT this
/// test's business: it predates §54, and "fix the blend here" would smuggle a
/// ranking change into a concierge fixture. Recorded, not taken.
async fn hide_from_candidates(tdb: &TestDb, work: &str) {
    exec(
        tdb,
        "UPDATE works SET visibility = 'unlisted' WHERE id = ?1#u",
        &[work],
    )
    .await;
}

// ---------------------------------------------------------------------------
// §54.7 — the door
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_queue_needs_a_session() {
    let dir = scratch_dir("conc_unauth");
    let tdb = TestDb::connect_with_dir("conc-unauth", &dir).await;
    let mut anonymous = TestClient::new(router_for(&tdb, &dir));

    // Every route, not just the obvious one. §54.3 records the session and §54.5
    // writes a watch, so a route that skipped the extractor would be writing on
    // behalf of nobody.
    let calls: Vec<(&str, &str)> = vec![
        ("GET", "/api/v1/me/concierge"),
        ("GET", "/api/v1/me/concierge/sessions"),
        ("GET", "/api/v1/me/watches"),
        (
            "PUT",
            "/api/v1/me/watches/00000000-0000-0000-0000-000000000001",
        ),
        (
            "DELETE",
            "/api/v1/me/watches/00000000-0000-0000-0000-000000000001",
        ),
    ];
    for (method, path) in calls {
        let (status, body) = anonymous.request(method, path, Some(json!({}))).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} must need a session: {body}"
        );
    }
}

// ---------------------------------------------------------------------------
// §54.6 — the selector is validated, and matching nothing is explained
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_unknown_mood_is_refused_naming_the_moods_we_have() {
    let dir = scratch_dir("conc_unknown_mood");
    let tdb = TestDb::connect_with_dir("conc-unknown-mood", &dir).await;
    let (mut client, _a, _p) = reader(&tdb, &dir, "conc_mood_ok").await;
    let works = seed_works(&tdb, 2, 4000).await;
    tag_mood(&tdb, &works[0], "comfort").await;

    let (status, body) = client.get("/api/v1/me/concierge?mood=catharsis").await;

    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an unknown mood is a refused selector, not a server error: {body}"
    );
    let rendered = body.to_string();
    assert!(
        rendered.contains("catharsis"),
        "the refusal names what was asked for: {rendered}"
    );
    assert!(
        rendered.contains("comfort"),
        "§54.2 requires naming the moods the taxonomy HAS — a refusal a reader \
         cannot act on is the same defect as a bare 403: {rendered}"
    );
    // And it must NOT have fallen back to the unfiltered queue.
    assert!(
        !rendered.contains("\"items\""),
        "a refused selector returns no queue at all: {rendered}"
    );
}

#[tokio::test]
async fn a_mood_matching_nothing_is_explained_and_still_recorded() {
    let dir = scratch_dir("conc_empty_mood");
    let tdb = TestDb::connect_with_dir("conc-empty-mood", &dir).await;
    let (mut client, _a, _p) = reader(&tdb, &dir, "conc_empty").await;
    let works = seed_works(&tdb, 2, 4000).await;
    // Tagged 'comfort', asked for 'grief'. `grief` must EXIST on the instance for
    // this to be the case being tested: the sibling test proves an unknown mood is
    // refused with 422, so an untagged 'grief' would exercise that path instead and
    // this test would pass for the wrong reason.
    //
    // `grief` is carried by a work the blend cannot serve, and the exclusion has to
    // be one BOTH `moods_in_use` and the blend agree on.
    //
    // Two were tried and both are wrong:
    //
    // - Lifecycle. `moods_in_use` joins `works.lifecycle = 'published'`
    //   (concierge_store.rs:252), so a draft's moods are not "in use" and the
    //   request comes back 422-unknown instead of exercising §54.6.
    // - Reading history. `seen_work_ids` is only consulted by the PLUGGABLE path
    //   (`RecContext.seen`, rec_engine.rs:66). The default `rec_mode` is `legacy`,
    //   and `blend()` has no seen-exclusion at all, so the work is served anyway —
    //   and it comes back through the popularity engine regardless of which engine
    //   the personalised one dropped it from.
    //
    //   That absence is a real defect and it is NOT this test's business: it is
    //   about the legacy blend, it predates §54, and "fix the blend here" would be a
    //   silent ranking change smuggled into a concierge fixture. Recorded instead.
    //
    tag_mood(&tdb, &works[0], "comfort").await;
    tag_mood(&tdb, &works[1], "grief").await;
    hide_from_candidates(&tdb, &works[1]).await;

    // Prove the exclusion happened, rather than assuming it: the grief-tagged work
    // must NOT be in the unfiltered queue. Without this the assertion below would
    // pass for the wrong reason — an empty blend would also produce no grief match.
    let (status, unfiltered) = client.get("/api/v1/me/concierge").await;
    assert_eq!(status, StatusCode::OK, "{unfiltered}");
    assert!(
        !unfiltered["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|i| i["work_id"] == works[1]),
        "the grief-tagged work must not be a candidate, or this fixture cannot \
         test 'matched nothing': {unfiltered}"
    );

    let (status, body) = client.get("/api/v1/me/concierge?mood=grief").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["items"].as_array().expect("an array").len(),
        0,
        "§54.6: matching nothing is an answer, not a fallback to the unfiltered \
         queue: {body}"
    );
    let why = body["explained_empty"]
        .as_str()
        .expect("§54.6 requires an explanation for an empty queue");
    assert!(
        !why.is_empty(),
        "the explanation must say something: {body}"
    );
    assert!(
        body["truncated_at"].is_null(),
        "a selector that matched nothing is not a budget cut, and the two must not \
         be reported alike: {body}"
    );

    // §54.6's other half: it is still a session, so "I asked and got nothing" is
    // visible in the reader's own history.
    //
    // Asked for BY MOOD, not by position: the guard GET above rendered the
    // unfiltered queue on its way to proving the exclusion, and every render is
    // recorded. Asserting on a COUNT would be asserting on this test's own setup,
    // and asserting on `sessions[0]` would pass whichever render landed first.
    let (status, history) = client.get("/api/v1/me/concierge/sessions").await;
    assert_eq!(status, StatusCode::OK, "{history}");
    let sessions = history["sessions"].as_array().expect("sessions");
    let explained: Vec<&Value> = sessions
        .iter()
        .filter(|s| s["mood"] == json!("grief"))
        .collect();
    assert_eq!(
        explained.len(),
        1,
        "the explained empty queue is recorded as a session of its own: {history}"
    );
    assert_eq!(
        explained[0]["work_ids"].as_array().expect("work_ids").len(),
        0,
        "and it records that it returned nothing, rather than storing a fallback: \
         {history}"
    );
    assert_eq!(
        explained[0]["truncated_at"],
        Value::Null,
        "'nothing matched' is not 'something was cut', and the stored session must \
         not conflate them: {history}"
    );
}

// ---------------------------------------------------------------------------
// §54.4 — the budget
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_budget_cuts_the_tail_and_names_the_cut() {
    let dir = scratch_dir("conc_budget");
    let tdb = TestDb::connect_with_dir("conc-budget", &dir).await;
    let (mut client, _a, _p) = reader(&tdb, &dir, "conc_budget").await;
    // 8000 words each = 40 minutes each at §11's 200 wpm. Four works = 160 minutes.
    let _ = seed_works(&tdb, 4, 8000).await;

    let (status, full) = client.get("/api/v1/me/concierge").await;
    assert_eq!(status, StatusCode::OK, "{full}");
    let all = full["items"].as_array().expect("items").len();
    assert!(
        all > 1,
        "the fixture needs more than one work to cut: {full}"
    );

    // 100 minutes buys two 40-minute works and must name index 2.
    let (status, cut) = client.get("/api/v1/me/concierge?minutes=100").await;
    assert_eq!(status, StatusCode::OK, "{cut}");
    assert_eq!(
        cut["items"].as_array().expect("items").len(),
        2,
        "40 + 40 fits in 100, a third does not: {cut}"
    );
    assert_eq!(
        cut["truncated_at"], 2,
        "§54.4 names the index it cut at: {cut}"
    );
    assert!(
        (cut["estimated_minutes"].as_f64().expect("a number") - 80.0).abs() < 0.01,
        "the total is the SUM OF WHAT WAS RETURNED, not the budget asked for: {cut}"
    );

    // A zero budget is not a fallback to everything.
    let (status, zero) = client.get("/api/v1/me/concierge?minutes=0").await;
    assert_eq!(status, StatusCode::OK, "{zero}");
    assert_eq!(
        zero["items"].as_array().expect("items").len(),
        0,
        "§54.6: a selector matching nothing is not a fallback, and neither is a \
         budget of zero: {zero}"
    );
    assert_eq!(
        zero["truncated_at"], 0,
        "'nothing fit' and 'nothing was cut' are different facts: {zero}"
    );
}

// ---------------------------------------------------------------------------
// §54.7 — the invariant that matters most
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_selector_returns_the_same_works_as_discovery() {
    let dir = scratch_dir("conc_parity");
    let tdb = TestDb::connect_with_dir("conc-parity", &dir).await;
    let (mut client, _a, _p) = reader(&tdb, &dir, "conc_parity").await;

    // Deliberately NOT uniform. The first version of this test seeded five
    // identical 4000-word works, and it was BLIND to the defect it was written for:
    // restoring the pre-fix pipeline (the pluggable `rec_engine` path, reversed)
    // left this test green. Two different engines over five indistinguishable works
    // rank them the same way by accident, so a fixture that cannot distinguish the
    // pipelines cannot prove they agree.
    //
    // The works differ in what the engines actually score on — length, popularity
    // (ratings, reads), and recency — so a different pipeline produces a different
    // order and the assertion has something to bite on.
    for (n, words) in [4000u32, 4000, 4000, 25000, 90000].iter().enumerate() {
        let works = seed_works(&tdb, 1, *words).await;
        // Spread the published timestamps: `public_recommendations` orders by
        // `updated_at DESC` (discovery.rs:321), so identical timestamps would make
        // that engine's order arbitrary and the test flaky rather than wrong.
        bump_updated_at(&tdb, &works[0], n).await;
        // Ratings drive the popularity engine. Staggered counts so its order is
        // distinct from the personalized one.
        add_ratings(&tdb, &works[0], 30 - n as i64).await;
    }

    // Same reader, same moment, two routes. The concierge must be a VIEW over the
    // §16 blend and not a second ranker — §54.1's whole argument.
    let (status, discovery) = client.get("/api/v1/discovery").await;
    assert_eq!(status, StatusCode::OK, "{discovery}");

    let (status, concierge) = client.get("/api/v1/me/concierge").await;
    assert_eq!(status, StatusCode::OK, "{concierge}");

    // Both routes key their rows on `work_id`; `get_discovery` returns them under
    // `items` alongside `sort` and a `request_id`.
    let from_discovery: Vec<&str> = discovery["items"]
        .as_array()
        .expect("discovery returns items")
        .iter()
        .map(|v| v["work_id"].as_str().expect("a string work_id"))
        .collect();
    let from_concierge: Vec<&str> = concierge["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|v| v["work_id"].as_str().expect("a string id"))
        .collect();

    eprintln!("PROBE {}", serde_json::to_string(&from_concierge).unwrap());
    assert!(
        from_discovery.len() >= 4,
        "the fixture needs several distinguishable works, or the two pipelines \
         agree by accident rather than by construction: {discovery}"
    );
    assert_eq!(
        from_concierge, from_discovery,
        "§54.7: a reader with no selector sees the same works as discovery, in the \
         same order. The concierge is a view over §16's blend, not a second ranker."
    );
}

// What this test does and does not prove, measured rather than assumed.
//
// PROVEN. Restoring the pre-fix pipeline — `rec_engine::generate_with_registry`,
// the PLUGGABLE path, which the default `rec_mode = legacy` does not take — turns
// this test red, and the failure is exactly the defect: the same five works in the
// OPPOSITE order. That was the original finding, so this test does bite on the bug
// it was written for.
//
// NOT PROVEN, and worth writing down. Two mutations that LOOK like they break
// parity leave this test green, and both are equivalent mutants rather than gaps:
//
//   * reversing the engine list before `blend()` — `blend()` sorts by score, so
//     reversing the rows while their scores still descend changes nothing;
//   * blending only the first engine — on this fixture the personalized engine
//     already returns every seeded work, so the RRF merge over three engines
//     produces the same list as one.
//
// The second is a real limitation with a real shape: a fixture where one engine
// sees everything cannot detect a change to how many engines run. `bump_updated_at`
// and `add_ratings` exist to give the engines different views, and they are not
// yet enough. Widening them further belongs with a case that seeds a work NO engine
// ranks — at which point this assertion would also be checking that the two routes
// agree about what to EXCLUDE.
//
// A truncation mutation (`.take(CANDIDATE_CAP)` -> `.take(2)`) does turn it red, so
// the comparison is reading real values on both sides rather than passing vacuously.

#[tokio::test]
async fn two_renders_with_no_intervening_writes_are_identical() {
    let dir = scratch_dir("conc_determinism");
    let tdb = TestDb::connect_with_dir("conc-determinism", &dir).await;
    let (mut client, _a, _p) = reader(&tdb, &dir, "conc_determinism").await;
    let _ = seed_works(&tdb, 4, 6000).await;

    // §54.7: two renders of one session are identical, or "the 20-minute queue" is
    // not a thing a reader can have an opinion about.
    let (_, first) = client.get("/api/v1/me/concierge?minutes=60").await;
    let (_, second) = client.get("/api/v1/me/concierge?minutes=60").await;

    let items = |b: &Value| -> Vec<String> {
        b["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|i| i["work_id"].as_str().expect("an id").to_owned())
            .collect()
    };
    assert_eq!(
        items(&first),
        items(&second),
        "two renders of the same request are identical"
    );
    assert_eq!(
        first["estimated_minutes"], second["estimated_minutes"],
        "and so is the total: {first} vs {second}"
    );
    // The session ids differ — these are two renders, not one cached answer — which
    // is what makes this a determinism claim about the LIST rather than about a
    // stored row being read back.
    assert_ne!(
        first["session_id"], second["session_id"],
        "each render is recorded as its own session"
    );
}

// ---------------------------------------------------------------------------
// §54.5 — watches
// ---------------------------------------------------------------------------

#[tokio::test]
async fn watching_a_work_twice_gives_one_watch_and_one_notification() {
    let dir = scratch_dir("conc_watch");
    let tdb = TestDb::connect_with_dir("conc-watch", &dir).await;
    let (mut client, _a, _p) = reader(&tdb, &dir, "conc_watch").await;
    let works = seed_works(&tdb, 1, 4000).await;
    let work = &works[0];

    let (status, first) = client
        .put(&format!("/api/v1/me/watches/{work}"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    // The work is already complete, so §54.5's immediate-notify case fires.
    assert_eq!(
        first["notified"], true,
        "a watch on a finished work notifies now: {first}"
    );

    let (status, second) = client
        .put(&format!("/api/v1/me/watches/{work}"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(
        first["watch_id"], second["watch_id"],
        "a second watch resolves to the FIRST watch, so a double-tap cannot produce \
         two notifications: {first} vs {second}"
    );
    assert_eq!(
        second["notified"], false,
        "and the second attempt notifies nobody — §54.5 says once ever: {second}"
    );

    let (status, list) = client.get("/api/v1/me/watches").await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let watches = list["watches"].as_array().expect("watches");
    assert_eq!(watches.len(), 1, "one watch, not two: {list}");
    assert_eq!(
        watches[0]["pending"], false,
        "the notification consumed it, so the reader can see it already fired: {list}"
    );
}

#[tokio::test]
async fn withdrawing_a_watch_is_silent_and_idempotent() {
    let dir = scratch_dir("conc_withdraw");
    let tdb = TestDb::connect_with_dir("conc-withdraw", &dir).await;
    let (mut client, _a, _p) = reader(&tdb, &dir, "conc_withdraw").await;
    let works = seed_works(&tdb, 1, 4000).await;
    let work = &works[0];

    let (status, _) = client
        .put(&format!("/api/v1/me/watches/{work}"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, removed) = client
        .request("DELETE", &format!("/api/v1/me/watches/{work}"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{removed}");
    assert_eq!(removed["removed"], true, "it was there: {removed}");

    // Idempotent: the state the caller asked for is the state they now have, so a
    // retried DELETE must succeed rather than 404.
    let (status, again) = client
        .request("DELETE", &format!("/api/v1/me/watches/{work}"), None)
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a retried withdraw still succeeds: {again}"
    );
    assert_eq!(
        again["removed"], false,
        "and reports honestly that nothing went: {again}"
    );

    let (_, list) = client.get("/api/v1/me/watches").await;
    assert_eq!(
        list["watches"].as_array().expect("watches").len(),
        0,
        "§54.5: withdrawing is silent — no tombstone: {list}"
    );
}

#[tokio::test]
async fn a_reader_cannot_see_or_remove_another_readers_watches() {
    let dir = scratch_dir("conc_private");
    let tdb = TestDb::connect_with_dir("conc-private", &dir).await;
    let (mut alice, _a, _ap) = reader(&tdb, &dir, "conc_alice").await;
    let (mut bob, _b, _bp) = reader(&tdb, &dir, "conc_bob").await;
    let works = seed_works(&tdb, 1, 4000).await;
    let work = &works[0];

    let (status, _) = alice
        .put(&format!("/api/v1/me/watches/{work}"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);

    // §54.6's first invariant over HTTP: Bob's list is empty and Bob's DELETE
    // removes nothing. The store is scoped in SQL, so this can only pass if the
    // route passes the caller's own account through.
    let (_, bobs_list) = bob.get("/api/v1/me/watches").await;
    assert_eq!(
        bobs_list["watches"].as_array().expect("watches").len(),
        0,
        "another reader's watches are not visible: {bobs_list}"
    );

    let (status, bob_removed) = bob
        .request("DELETE", &format!("/api/v1/me/watches/{work}"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{bob_removed}");
    assert_eq!(
        bob_removed["removed"], false,
        "and cannot be removed by someone who never watched: {bob_removed}"
    );

    let (_, alices_list) = alice.get("/api/v1/me/watches").await;
    assert_eq!(
        alices_list["watches"].as_array().expect("watches").len(),
        1,
        "Alice's watch survived Bob's attempt: {alices_list}"
    );
}

#[tokio::test]
async fn one_readers_sessions_are_not_another_readers() {
    let dir = scratch_dir("conc_sessions_private");
    let tdb = TestDb::connect_with_dir("conc-sessions-private", &dir).await;
    let (mut alice, _a, _ap) = reader(&tdb, &dir, "conc_sa").await;
    let (mut bob, _b, _bp) = reader(&tdb, &dir, "conc_sb").await;
    let _ = seed_works(&tdb, 3, 4000).await;

    for n in 0..2 {
        let (status, body) = alice.get("/api/v1/me/concierge?minutes=30").await;
        assert_eq!(
            status,
            StatusCode::OK,
            "alice render {n} on postgres: {body}"
        );
    }
    let (status, bobbody) = bob.get("/api/v1/me/concierge?minutes=30").await;
    assert_eq!(status, StatusCode::OK, "{bobbody}");

    let (_, alices) = alice.get("/api/v1/me/concierge/sessions").await;
    let (_, bobs) = bob.get("/api/v1/me/concierge/sessions").await;
    assert_eq!(
        alices["sessions"].as_array().expect("sessions").len(),
        2,
        "Alice rendered twice: {alices}"
    );
    assert_eq!(
        bobs["sessions"].as_array().expect("sessions").len(),
        1,
        "§54.6: Bob sees his own session and only his own: {bobs}"
    );

    // And the histories do not overlap, which is the property that would break if
    // the route passed a session id instead of the account.
    let a_ids: Vec<&str> = alices["sessions"]
        .as_array()
        .expect("sessions")
        .iter()
        .map(|s| s["id"].as_str().expect("an id"))
        .collect();
    let b_ids: Vec<&str> = bobs["sessions"]
        .as_array()
        .expect("sessions")
        .iter()
        .map(|s| s["id"].as_str().expect("an id"))
        .collect();
    for id in &b_ids {
        assert!(
            !a_ids.contains(id),
            "a session id appears in both readers' histories: {id}"
        );
    }
}

// ---------------------------------------------------------------------------
// §54.4 — the rate is stated, not guessed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_rate_source_is_stated_and_defaults_without_an_observation() {
    let dir = scratch_dir("conc_rate");
    let tdb = TestDb::connect_with_dir("conc-rate", &dir).await;
    let (mut client, _a, _p) = reader(&tdb, &dir, "conc_rate").await;
    let _ = seed_works(&tdb, 2, 4000).await;

    let (status, body) = client.get("/api/v1/me/concierge").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // A brand-new reader has no reading history, so the instance default is used —
    // and the response SAYS which, because a reader comparing two queues built on
    // different rates would otherwise see a difference and be given no explanation.
    assert_eq!(
        body["rate_source"], "default",
        "a reader with no observed progress is served the instance default, stated: {body}"
    );
    assert_ne!(
        body["rate_source"], "observed",
        "§54.4 forbids guessing a personal rate without an observation"
    );
}
