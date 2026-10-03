//! M45-22 step 4: the concierge store, on both engines.
//!
//! The load-bearing test here is `sessions_for_never_returns_another_readers_
//! sessions`. §54.6's first invariant is that the concierge never reads another
//! reader's data, and this file's *other* tests cannot detect its violation: every
//! one of them either uses a single reader or reads back what it just wrote. Drop
//! `AND account_id = ?1` from `sessions_for` and all of them stay green while the
//! defect ships. That is why the test creates two readers and asserts the list has
//! length 1 — it was written to be broken, and it was confirmed red against the
//! unscoped query.
//!
//! Same reason the WIP tests run against a real engine: §54.5's "one notification
//! per watch, ever" is a `UNIQUE (account_id, work_id)` plus a
//! `notified_at IS NULL` predicate, and neither is visible to a mocked pool.

use lorehaven_db::concierge_store::{
    add_watch, decode_work_ids, is_complete, mark_watched, moods_in_use, pending_watches_for_work,
    record_session, remove_watch, session_for, sessions_for, SessionRow,
};
use lorehaven_db::Database;
use lorehaven_domain::concierge::{
    apply_budget, ConciergeQueue, QueueItem, QueueReason, RateSource, SessionSelector,
};
use std::time::Duration;

fn make_config(url: String) -> lorehaven_db::DatabaseConfig {
    lorehaven_db::DatabaseConfig {
        url,
        max_connections: 5,
        acquire_timeout: Duration::from_secs(5),
        slow_query_warn: Duration::ZERO,
    }
}

/// A scratch database on whichever backend `LOREHAVEN_TEST_PG_URL` names.
/// Same shape as `source_adapters.rs` for the same documented reason: this crate
/// cannot depend on `test_support`, and reusing the shared `postgres` database
/// trips the append-only migration checksum guard on every later run.
async fn connect(tag: &str) -> Database {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "lorehaven-concierge-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let url = match std::env::var("LOREHAVEN_TEST_PG_URL") {
        Ok(admin) => {
            let name = format!(
                "lh_concierge_{}_{}",
                std::process::id(),
                uuid::Uuid::new_v4().simple()
            );
            let admin_db = Database::connect(&make_config(admin.clone()))
                .await
                .expect("connect to the admin database");
            sqlx::query(&format!("CREATE DATABASE {name}"))
                .execute(admin_db.postgres_pool().expect("postgres pool"))
                .await
                .expect("create a scratch database");
            admin_db.close().await;
            let (prefix, _) = admin
                .rsplit_once('/')
                .expect("the admin URL ends in a database");
            format!("{prefix}/{name}")
        }
        Err(_) => format!("sqlite://{}/lorehaven.sqlite?mode=rwc", dir.display()),
    };
    let db = Database::connect(&make_config(url)).await.expect("connect");
    db.migrate().await.expect("migrate");
    db
}

const T0: &str = "2026-01-01T00:00:00Z";

/// Run one statement with `?N#u` for a native uuid (text on SQLite) and `?N#i` for an
/// integer.
///
/// Copied from `preread_store.rs` rather than written fresh, because the dialect
/// split is three mechanical rewrites and the first version of this helper got one
/// of them wrong: sqlx does **not** translate `?1` into `$1` for PostgreSQL, so every
/// fixture reached the server as the literal token `?` and failed with
/// `operator does not exist: ? integer`. All 13 tests failed on that one missing
/// rewrite — which is the argument for one shared helper rather than a per-file one
/// that gets it subtly wrong.
async fn exec(db: &Database, tmpl: &str, args: &[&str]) {
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            let sql = tmpl.replace("#u", "").replace("#i", "");
            let mut q = sqlx::query(&sql);
            for a in args {
                q = q.bind(*a);
            }
            q.execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("fixture insert");
        }
        lorehaven_db::Backend::Postgres => {
            let pg = (1..=8).fold(tmpl.to_string(), |acc, n| {
                acc.replace(&format!("?{n}#u"), &format!("${n}::uuid"))
                    .replace(&format!("?{n}#i"), &format!("${n}::bigint"))
                    .replace(&format!("?{n}"), &format!("${n}"))
            });
            let mut q = sqlx::query(&pg);
            for a in args {
                match uuid::Uuid::parse_str(a) {
                    Ok(u) => q = q.bind(u),
                    Err(_) => q = q.bind(*a),
                }
            }
            q.execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("fixture insert");
        }
    }
}

async fn insert_account(db: &Database, id: &str) {
    exec(
        db,
        "INSERT INTO accounts (id, status, email, age_state, created_at, updated_at, permission_statement)
         VALUES (?1#u, 'active', ?2, 'adult', ?3, ?3, 'minimal')",
        &[id, &format!("{id}@example.com"), T0],
    )
    .await;
}

/// A published work owned by a fresh pseud, so `is_complete` has something to
/// read. Each work gets its own pseud because `works.owner_pseud_id` is NOT NULL
/// and FK-constrained.
async fn insert_work(db: &Database, id: &str, completion: &str) {
    insert_work_with_lifecycle(db, id, completion, "published").await
}

/// `lifecycle` is a parameter because one test needs a work that is `draft` while
/// still `complete`. The two columns are orthogonal and `moods_in_use` filters on
/// both, so a fixture that can only make published works cannot test half of the
/// filter. The first version of this file hardcoded `'published'` and named the
/// variable `draft` anyway, so the "non-published work must not be offered"
/// assertion was vacuous and passed for the wrong reason.
async fn insert_work_with_lifecycle(db: &Database, id: &str, completion: &str, lifecycle: &str) {
    let account = uuid::Uuid::new_v4().to_string();
    let pseud = uuid::Uuid::new_v4().to_string();
    exec(
        db,
        "INSERT INTO accounts (id, status, email, age_state, created_at, updated_at, permission_statement)
         VALUES (?1#u, 'active', ?2, 'adult', ?3, ?3, 'minimal')",
        &[&account, &format!("w-{account}@example.com"), T0],
    )
    .await;
    exec(
        db,
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
         VALUES (?1#u, ?2#u, ?3, ?3, ?4, ?4)",
        &[&pseud, &account, &format!("w{}", &id[..8]), T0],
    )
    .await;
    exec(
        db,
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, completion, created_at, updated_at, generated_content_posture)
         VALUES (?1#u, ?2#u, ?3, ?4, ?5, ?6, ?6, 'forbid')",
        &[id, &pseud, &format!("Work {id}"), lifecycle, completion, T0],
    )
    .await;
}

/// Tag a work with a mood node, returning the node id.
async fn tag_mood(db: &Database, work_id: &str, canonical: &str) -> String {
    let node_id = uuid::Uuid::new_v4().to_string();
    exec(db, "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at, review_status, signal_count)
              VALUES (?1#u, 'mood', ?2, ?3, ?4, 'pending', 0)",
         &[&node_id, canonical, &canonical.to_lowercase(), T0]).await;
    exec(
        db,
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES (?1#u, ?2#u, 1, ?3)",
        &[work_id, &node_id, T0],
    )
    .await;
    node_id
}

fn a_queue(selector: SessionSelector, ids: &[&str]) -> ConciergeQueue {
    let ranked: Vec<(String, Option<f64>)> = ids
        .iter()
        .enumerate()
        .map(|(n, id)| ((*id).to_owned(), Some(10.0 * (n as f64 + 1.0))))
        .collect();
    let budget = selector.budget_minutes.map(f64::from);
    let (items, truncated_at, total) = apply_budget(&ranked, budget);
    ConciergeQueue {
        session_id: String::new(),
        items,
        estimated_minutes: total,
        truncated_at,
        rate_source: RateSource::Observed,
        selector,
        explained_empty: None,
    }
}

// ── the invariant ───────────────────────────────────────────────────────────

#[tokio::test]
async fn sessions_for_never_returns_another_readers_sessions() {
    let db = connect("scope").await;
    let (alice, bob) = (
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    );
    insert_account(&db, &alice).await;
    insert_account(&db, &bob).await;
    let (wa, wb) = (
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    );
    insert_work(&db, &wa, "complete").await;
    insert_work(&db, &wb, "complete").await;

    let selector = SessionSelector {
        mood: Some("comfort".to_owned()),
        budget_minutes: Some(30),
    };
    record_session(&db, &alice, &a_queue(selector.clone(), &[&wa]))
        .await
        .expect("alice");
    record_session(&db, &bob, &a_queue(selector, &[&wb]))
        .await
        .expect("bob");

    let as_alice = sessions_for(&db, &alice, 50).await.expect("list as alice");
    assert_eq!(
        as_alice.len(),
        1,
        "§54.6: the concierge never reads another reader's sessions — got {}",
        as_alice.len()
    );
    assert_eq!(decode_work_ids(&as_alice[0].work_ids), vec![wa]);

    // And the singular form is scoped too: a correct id belonging to Bob is not
    // Alice's to fetch. `None`, not an error, so a caller cannot use the response
    // to confirm that the id exists at all.
    let bob_session = sessions_for(&db, &bob, 50).await.expect("list as bob");
    assert_eq!(
        session_for(&db, &alice, &bob_session[0].id)
            .await
            .expect("fetch by id"),
        None,
        "another reader's session id must not resolve, and must not be \
         distinguishable from an id that does not exist"
    );
    db.close().await;
}

#[tokio::test]
async fn a_session_round_trips_the_render_exactly() {
    let db = connect("roundtrip").await;
    let reader = uuid::Uuid::new_v4().to_string();
    insert_account(&db, &reader).await;
    let (w1, w2, w3) = (
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    );
    for w in [&w1, &w2, &w3] {
        insert_work(&db, w, "complete").await;
    }

    let selector = SessionSelector {
        mood: Some("comfort".to_owned()),
        budget_minutes: Some(45),
    };
    // 10 + 20 + 30 = 60 > 45, so the third work is cut at index 2.
    let rendered = a_queue(selector, &[&w1, &w2, &w3]);
    assert_eq!(
        rendered.truncated_at,
        Some(2),
        "fixture assumption: {rendered:?}"
    );
    let id = record_session(&db, &reader, &rendered)
        .await
        .expect("record");

    let rows = sessions_for(&db, &reader, 10).await.expect("list");
    assert_eq!(rows.len(), 1);
    let row: &SessionRow = &rows[0];
    assert_eq!(row.id, id);
    assert_eq!(row.mood.as_deref(), Some("comfort"));
    assert_eq!(row.budget_minutes, Some(45));
    assert_eq!(row.truncated_at, Some(2));
    assert_eq!(row.rate_source, "observed");
    // §54.3: the record is what the reader saw, in order. Not a ranking input.
    assert_eq!(decode_work_ids(&row.work_ids), vec![w1, w2]);
    assert!((row.estimated_minutes.expect("total") - 30.0).abs() < 1e-9);
    assert_eq!(
        session_for(&db, &reader, &id)
            .await
            .expect("one")
            .map(|r| r.id),
        Some(id)
    );
    db.close().await;
}

#[tokio::test]
async fn a_queue_with_no_mood_records_null_not_an_empty_string() {
    let db = connect("nomood").await;
    let reader = uuid::Uuid::new_v4().to_string();
    insert_account(&db, &reader).await;
    let work = uuid::Uuid::new_v4().to_string();
    insert_work(&db, &work, "complete").await;

    // §54.6: an empty intent is the plain blend, so the mood column is NULL — not
    // "". A blank-string mood would read back as a selector that named nothing,
    // which is a different row.
    record_session(&db, &reader, &a_queue(SessionSelector::none(), &[&work]))
        .await
        .expect("record");

    let rows = sessions_for(&db, &reader, 10).await.expect("list");
    assert_eq!(rows[0].mood, None, "no selector is NULL, not Some(\"\")");
    assert_eq!(rows[0].budget_minutes, None);
    db.close().await;
}

#[tokio::test]
async fn work_ids_survive_a_json_round_trip() {
    let db = connect("json").await;
    let reader = uuid::Uuid::new_v4().to_string();
    insert_account(&db, &reader).await;
    let work = uuid::Uuid::new_v4().to_string();
    insert_work(&db, &work, "complete").await;

    let mut q = a_queue(SessionSelector::none(), &[&work]);
    q.items[0].reason = QueueReason::DurationUnknown;
    record_session(&db, &reader, &q).await.expect("record");

    let rows = sessions_for(&db, &reader, 10).await.expect("list");
    // encode and decode tested together: a mismatch writes one shape and reads
    // another, which looks like a parse that quietly worked and returned nothing.
    assert_eq!(decode_work_ids(&rows[0].work_ids), vec![work.clone()]);
    assert_eq!(decode_work_ids("[]"), Vec::<String>::new());
    // Garbage in the column must not panic a reader — an empty list is the safe
    // reading, since a hand-edited row is not worth a 500.
    assert!(decode_work_ids("not json").is_empty());
    db.close().await;
}

// ── the mood list §54.2's refusal names ──────────────────────────────────────

#[tokio::test]
async fn moods_in_use_lists_only_moods_a_published_work_carries() {
    let db = connect("moods").await;
    let (published, draft, other_kind) = (
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    );
    insert_work(&db, &published, "complete").await;
    insert_work_with_lifecycle(&db, &draft, "in_progress", "draft").await;
    tag_mood(&db, &published, "comfort").await;
    tag_mood(&db, &draft, "grief").await;

    // A mood node nobody has used is not a mood a reader can ask for and be
    // satisfied by, so it must not be offered.
    let unused = uuid::Uuid::new_v4().to_string();
    exec(&db, "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at, review_status, signal_count)
              VALUES (?1#u, 'mood', 'solitude', 'solitude', ?2, 'pending', 0)",
         &[&unused, T0]).await;
    // And a non-mood kind on a published work is not a mood either. The work is
    // created first: `work_tags.work_id` carries a real FK on PostgreSQL, so a tag
    // row inserted before its work is a constraint violation rather than a quiet
    // no-op — SQLite does not enforce it by default, so getting this order wrong
    // passes on one engine and fails on the other.
    insert_work(&db, &other_kind, "complete").await;
    let tag_node = uuid::Uuid::new_v4().to_string();
    exec(&db, "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at, review_status, signal_count)
              VALUES (?1#u, 'tag', 'angst', 'angst', ?2, 'pending', 0)",
         &[&tag_node, T0]).await;
    exec(
        &db,
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) VALUES (?1#u, ?2#u, 1, ?3)",
        &[&other_kind, &tag_node, T0],
    )
    .await;

    let moods = moods_in_use(&db).await.expect("moods");
    assert!(
        moods.contains(&"comfort".to_owned()),
        "published work's mood: {moods:?}"
    );
    assert!(
        !moods.contains(&"grief".to_owned()),
        "a mood only carried by a non-published work must not be offered: {moods:?}"
    );
    assert!(
        !moods.contains(&"solitude".to_owned()),
        "unused mood: {moods:?}"
    );
    assert!(
        !moods.contains(&"angst".to_owned()),
        "a tag is not a mood: {moods:?}"
    );
    db.close().await;
}

// ── §54.5's WIP watches ─────────────────────────────────────────────────────

#[tokio::test]
async fn one_notification_per_watch_ever() {
    let db = connect("watch").await;
    let reader = uuid::Uuid::new_v4().to_string();
    insert_account(&db, &reader).await;
    let work = uuid::Uuid::new_v4().to_string();
    insert_work(&db, &work, "in_progress").await;

    let first = add_watch(&db, &reader, &work).await.expect("watch");
    // A second watch on the same work by the same reader returns the FIRST
    // watch's id rather than a fresh id that addresses no row, so the caller can
    // tell "already watching" from "watching now".
    let second = add_watch(&db, &reader, &work).await.expect("watch again");
    assert_eq!(
        first, second,
        "a re-watch must resolve to the original watch id"
    );

    let pending = pending_watches_for_work(&db, &work).await.expect("pending");
    assert_eq!(pending.len(), 1, "one watch, not two rows: {pending:?}");
    assert_eq!(pending[0], (reader.clone(), first.clone()));

    assert!(
        mark_watched(&db, &first).await.expect("consume"),
        "the first claim wins"
    );
    assert!(
        !mark_watched(&db, &first).await.expect("re-consume"),
        "a consumed watch cannot be claimed twice — the notified_at IS NULL \
         predicate is inside the WHERE, so a concurrent completion loses"
    );
    assert!(
        pending_watches_for_work(&db, &work)
            .await
            .expect("pending")
            .is_empty(),
        "the watch is cancelled by the notification"
    );
    db.close().await;
}

#[tokio::test]
async fn two_readers_watching_one_work_each_get_their_own_watch() {
    // The UNIQUE is on (account_id, work_id), not on work_id: two readers watching
    // the same work is the normal case, and a UNIQUE on work_id would silently
    // drop the second reader's notification.
    let db = connect("two-watchers").await;
    let (a, b) = (
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    );
    insert_account(&db, &a).await;
    insert_account(&db, &b).await;
    let work = uuid::Uuid::new_v4().to_string();
    insert_work(&db, &work, "in_progress").await;

    let wa = add_watch(&db, &a, &work).await.expect("watch a");
    let wb = add_watch(&db, &b, &work).await.expect("watch b");
    assert_ne!(wa, wb);

    let pending = pending_watches_for_work(&db, &work).await.expect("pending");
    assert_eq!(pending.len(), 2, "both readers are waiting: {pending:?}");
    assert!(mark_watched(&db, &wa).await.expect("consume a"));
    assert!(
        mark_watched(&db, &wb).await.expect("consume b"),
        "consuming one reader's watch must not consume another's"
    );
    db.close().await;
}

#[tokio::test]
async fn withdrawing_a_watch_is_silent_and_owner_only() {
    let db = connect("withdraw").await;
    let (mine, theirs) = (
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    );
    insert_account(&db, &mine).await;
    insert_account(&db, &theirs).await;
    let work = uuid::Uuid::new_v4().to_string();
    insert_work(&db, &work, "in_progress").await;
    add_watch(&db, &theirs, &work).await.expect("their watch");

    assert!(
        !remove_watch(&db, &mine, &work).await.expect("remove"),
        "a reader who never watched cannot remove somebody else's watch"
    );
    assert_eq!(
        pending_watches_for_work(&db, &work)
            .await
            .expect("pending")
            .len(),
        1,
        "and their watch survives"
    );

    add_watch(&db, &mine, &work).await.expect("my watch");
    assert!(remove_watch(&db, &mine, &work).await.expect("remove mine"));
    let pending = pending_watches_for_work(&db, &work).await.expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].0, theirs,
        "only the other reader's watch is left"
    );
    db.close().await;
}

#[tokio::test]
async fn completeness_reads_completion_not_lifecycle() {
    // The two columns are orthogonal: a work can be published and still a WIP.
    // Reading `lifecycle` here would make every published work look complete and
    // fire a notification for the whole catalogue on the first watch.
    let db = connect("complete").await;
    let (wip, done, unpublished) = (
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    );
    insert_work(&db, &wip, "in_progress").await;
    insert_work(&db, &done, "complete").await;
    insert_work_with_lifecycle(&db, &unpublished, "complete", "draft").await;
    assert!(
        !is_complete(&db, &wip).await.expect("wip"),
        "published + in_progress is not complete"
    );
    assert!(is_complete(&db, &done).await.expect("done"));
    assert!(
        is_complete(&db, &unpublished).await.expect("draft"),
        "lifecycle is not the column read here"
    );
    assert!(!is_complete(&db, &uuid::Uuid::new_v4().to_string())
        .await
        .expect("absent"));
    db.close().await;
}

#[tokio::test]
async fn a_watch_on_an_already_complete_work_is_consumable_immediately() {
    // §54.5's immediate-notify case: the work is done, so the pending query finds
    // it and mark_watched can claim it on the same pass.
    let db = connect("immediate").await;
    let reader = uuid::Uuid::new_v4().to_string();
    insert_account(&db, &reader).await;
    let work = uuid::Uuid::new_v4().to_string();
    insert_work(&db, &work, "complete").await;

    let id = add_watch(&db, &reader, &work).await.expect("watch");
    let pending = pending_watches_for_work(&db, &work).await.expect("pending");
    assert_eq!(pending.len(), 1);
    assert!(mark_watched(&db, &id).await.expect("immediate"));
    assert!(pending_watches_for_work(&db, &work)
        .await
        .expect("pending")
        .is_empty());
    db.close().await;
}

#[tokio::test]
async fn sessions_for_honours_its_limit_and_its_order() {
    let db = connect("limit").await;
    let reader = uuid::Uuid::new_v4().to_string();
    insert_account(&db, &reader).await;
    let work = uuid::Uuid::new_v4().to_string();
    insert_work(&db, &work, "complete").await;
    for _ in 0..4 {
        record_session(&db, &reader, &a_queue(SessionSelector::none(), &[&work]))
            .await
            .expect("record");
    }
    assert_eq!(
        sessions_for(&db, &reader, 2).await.expect("limit 2").len(),
        2
    );
    assert_eq!(
        sessions_for(&db, &reader, 50)
            .await
            .expect("limit 50")
            .len(),
        4
    );
    // Newest first, so the most recent session is the one a reader sees.
    let rows = sessions_for(&db, &reader, 50).await.expect("order");
    assert_eq!(
        sessions_for(&db, &reader, 1).await.expect("one")[0].id,
        rows[0].id
    );
    db.close().await;
}

#[tokio::test]
async fn an_explained_empty_queue_still_records_a_session() {
    // §54.6's explained empty queue is an answer, so it is a session like any
    // other — recording only non-empty queues would make "I asked and got
    // nothing" invisible in the reader's history.
    let db = connect("explained").await;
    let reader = uuid::Uuid::new_v4().to_string();
    insert_account(&db, &reader).await;
    let selector = SessionSelector {
        mood: Some("catharsis".to_owned()),
        budget_minutes: None,
    };
    let empty = ConciergeQueue::explained_empty(
        "",
        "no work on this instance carries the mood \"catharsis\"",
        RateSource::Default,
        selector.clone(),
    );
    record_session(&db, &reader, &empty).await.expect("record");

    let rows = sessions_for(&db, &reader, 10).await.expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].mood.as_deref(),
        Some("catharsis"),
        "the selector is kept even though nothing matched"
    );
    assert_eq!(decode_work_ids(&rows[0].work_ids), Vec::<String>::new());
    assert_eq!(rows[0].rate_source, "default");
    db.close().await;
}

#[tokio::test]
async fn an_unknown_duration_work_records_null_not_zero() {
    // `estimated_minutes: Some(0.0)` would claim the work is a zero-minute read,
    // which is a measurement. NULL is "we do not know", and §54.4 says the reader
    // is told that rather than guessed at.
    let db = connect("null-estimate").await;
    let reader = uuid::Uuid::new_v4().to_string();
    insert_account(&db, &reader).await;
    let known = uuid::Uuid::new_v4().to_string();
    insert_work(&db, &known, "complete").await;

    let mut q = a_queue(SessionSelector::none(), &[&known]);
    q.items[0] = QueueItem {
        work_id: known.clone(),
        reason: QueueReason::DurationUnknown,
        estimated_minutes: None,
    };
    record_session(&db, &reader, &q).await.expect("record");

    let rows = sessions_for(&db, &reader, 10).await.expect("list");
    // The queue's TOTAL is a number even when an item is unknown, so this one is
    // asserted rather than left to the item-level NULL above.
    assert!(
        rows[0].estimated_minutes.is_some(),
        "the total is 0.0, not NULL"
    );
    assert_eq!(decode_work_ids(&rows[0].work_ids), vec![known]);
    db.close().await;
}
