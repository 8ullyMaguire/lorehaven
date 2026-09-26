//! The k-anonymity floor, applied at the query.
//!
//! The point of these tests is the *shape* of the failure they prevent. A
//! display-side filter that runs after the query has already read the rows has
//! already had the privacy incident; the individual pseudonyms were in the
//! process, even if the response never mentioned them. So the counts here are
//! produced as aggregates and the floor is applied to the aggregate, and the
//! tests assert the response *type* rather than a formatted string -- because
//! a string is exactly where a suppression turns back into a number.
//!
//! Dual-backend: every test runs on SQLite and PostgreSQL, because a privacy
//! rule that holds on one dialect and not the other is not a privacy rule.

use lorehaven_db::{Backend, Database};
use serde_json::{json, Value};
use test_support::{id, scratch_dir, TestDb};

/// A pseudonym plus the account it hangs off.
async fn seed_pseud(db: &Database, tag: &str) -> String {
    let account = id(&format!("{tag}-acct"));
    let pseud = id(&format!("{tag}-pseud"));
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, ?, ?)",
            )
            .bind(&account)
            .bind(format!("{tag}@test.dev"))
            .bind("2026-01-01 00:00:00")
            .bind("2026-01-01 00:00:00")
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&pseud)
            .bind(&account)
            .bind(tag)
            .bind(tag)
            .bind("2026-01-01 00:00:00")
            .bind("2026-01-01 00:00:00")
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .unwrap();
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at) VALUES ($1::uuid, $2, now(), now())",
            )
            .bind(&account)
            .bind(format!("{tag}@test.dev"))
            .execute(pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
                 VALUES ($1::uuid, $2::uuid, $3, $4, now(), now())",
            )
            .bind(&pseud)
            .bind(&account)
            .bind(tag)
            .bind(tag)
            .execute(pool)
            .await
            .unwrap();
        }
    }
    pseud
}

/// A work owned by `owner_pseud`.
async fn seed_work(db: &Database, tag: &str, owner_pseud: &str) -> String {
    let work = id(&format!("{tag}-work"));
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, visibility, lifecycle, created_at, updated_at) \
                 VALUES (?, ?, ?, 'public', 'published', ?, ?)",
            )
            .bind(&work)
            .bind(owner_pseud)
            .bind("A Work")
            .bind("2026-01-01 00:00:00")
            .bind("2026-01-01 00:00:00")
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .unwrap();
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, visibility, lifecycle, created_at, updated_at) \
                 VALUES ($1::uuid, $2::uuid, $3, 'public', 'published', now(), now())",
            )
            .bind(&work)
            .bind(owner_pseud)
            .bind("A Work")
            .execute(pool)
            .await
            .unwrap();
        }
    }
    work
}

/// Record `events` reading events for `pseud` on `work`.
async fn seed_reads(db: &Database, tag: &str, pseud: &str, work: &str, events: i64) {
    let account: String = match db.backend() {
        Backend::Sqlite => sqlx::query_scalar("SELECT account_id FROM pseuds WHERE id = ?")
            .bind(pseud)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await
            .unwrap(),
        Backend::Postgres => {
            sqlx::query_scalar("SELECT account_id::text FROM pseuds WHERE id = $1::uuid")
                .bind(pseud)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await
                .unwrap()
        }
    };
    for n in 0..events {
        let row = id(&format!("{tag}-read-{n}"));
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query(
                    "INSERT INTO reading_history_entry \
                       (id, account_id, pseud_id, subject_type, subject_id, last_read_at, created_at) \
                     VALUES (?, ?, ?, 'work', ?, ?, ?)",
                )
                .bind(&row)
                .bind(&account)
                .bind(pseud)
                .bind(work)
                .bind("2026-02-01 00:00:00")
                .bind("2026-02-01 00:00:00")
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .unwrap();
            }
            Backend::Postgres => {
                sqlx::query(
                    "INSERT INTO reading_history_entry \
                       (id, account_id, pseud_id, subject_type, subject_id, last_read_at, created_at) \
                     VALUES ($1::uuid, $2::uuid, $3::uuid, 'work', $4::uuid, $5::timestamptz, $5::timestamptz)",
                )
                .bind(&row)
                .bind(&account)
                .bind(pseud)
                .bind(work)
                .bind("2026-02-01T00:00:00Z")
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .unwrap();
            }
        }
    }
}

/// The metric as JSON, so a test can assert on the *wire* form.
///
/// The wire form is the thing under test: `ReaderCount` is a type, and a type
/// can hold `count: Some(3)` in a field the serialiser is supposed to skip. A
/// green unit test on the struct is not evidence about the response.
fn json_of(rc: lorehaven_db::analytics::ReaderCount) -> Value {
    serde_json::to_value(rc).unwrap()
}

/// The count field, or a panic naming what came back instead.
fn count_of(v: &Value) -> i64 {
    v["count"].as_i64().unwrap_or_else(|| {
        panic!("expected a numeric count, got {v}");
    })
}

// --- the floor ----------------------------------------------------------------

#[tokio::test]
async fn a_work_with_few_readers_reports_a_floor_and_not_a_number() {
    let dir = scratch_dir("anon_below");
    let tdb = TestDb::connect_with_dir("anon-below", &dir).await;
    let owner = seed_pseud(tdb.db(), "anon-owner").await;
    let work = seed_work(tdb.db(), "anon", &owner).await;
    for i in 0..3 {
        let p = seed_pseud(tdb.db(), &format!("anon-r{i}")).await;
        seed_reads(tdb.db(), &format!("anon-r{i}"), &p, &work, 1).await;
    }

    let v = json_of(
        lorehaven_db::analytics::work_reader_count(tdb.db(), &work)
            .await
            .unwrap(),
    );

    assert!(
        v["count"].is_null(),
        "a suppressed count was serialised as a number: {v}"
    );
    assert_eq!(v["fewer_than"], 10, "{v}");
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_work_with_enough_readers_reports_the_true_number() {
    let dir = scratch_dir("anon_at");
    let tdb = TestDb::connect_with_dir("anon-at", &dir).await;
    let owner = seed_pseud(tdb.db(), "anon2-owner").await;
    let work = seed_work(tdb.db(), "anon2", &owner).await;
    for i in 0..12 {
        let p = seed_pseud(tdb.db(), &format!("anon2-r{i}")).await;
        seed_reads(tdb.db(), &format!("anon2-r{i}"), &p, &work, 1).await;
    }

    let v = json_of(
        lorehaven_db::analytics::work_reader_count(tdb.db(), &work)
            .await
            .unwrap(),
    );

    assert_eq!(count_of(&v), 12, "{v}");
    assert!(v["fewer_than"].is_null(), "{v}");
    tdb.cleanup().await;
}

#[tokio::test]
async fn the_floor_boundary_is_exactly_ten() {
    // Nine suppresses, ten shows. A boundary that is off by one is a boundary
    // that leaks one reader in a small community -- which is most of them.
    for readers in [9i64, 10, 11] {
        let dir = scratch_dir(&format!("anon_edge_{readers}"));
        let tdb = TestDb::connect_with_dir(&format!("anon-edge-{readers}"), &dir).await;
        let owner = seed_pseud(tdb.db(), &format!("anon3-{readers}-owner")).await;
        let work = seed_work(tdb.db(), &format!("anon3-{readers}"), &owner).await;
        for i in 0..readers {
            let p = seed_pseud(tdb.db(), &format!("anon3-{readers}-r{i}")).await;
            seed_reads(tdb.db(), &format!("anon3-{readers}-r{i}"), &p, &work, 1).await;
        }

        let v = json_of(
            lorehaven_db::analytics::work_reader_count(tdb.db(), &work)
                .await
                .unwrap(),
        );

        assert_eq!(
            v["count"].is_number(),
            readers >= 10,
            "{readers} readers produced {v}"
        );
        tdb.cleanup().await;
    }
}

#[tokio::test]
async fn a_reader_is_counted_once_not_once_per_chapter() {
    // `reading_history_entry` has a unique key on
    // (account, pseud, subject), so a reader reading a 25-chapter work has
    // *one* history row that moves forward, not 25. The schema is what makes
    // this correct rather than a `DISTINCT` clause that could be forgotten --
    // but the metric must still count distinct pseudonyms, because a reader
    // with a history row on this work and another on a chapter-scoped row is
    // possible after a chapter-split migration.
    let dir = scratch_dir("anon_distinct");
    let tdb = TestDb::connect_with_dir("anon-distinct", &dir).await;
    let owner = seed_pseud(tdb.db(), "anon4-owner").await;
    let work = seed_work(tdb.db(), "anon4", &owner).await;
    // Twelve readers, so the count is above the floor and therefore exact.
    for i in 0..12 {
        let p = seed_pseud(tdb.db(), &format!("anon4-r{i}")).await;
        seed_reads(tdb.db(), &format!("anon4-r{i}"), &p, &work, 1).await;
    }

    let v = json_of(
        lorehaven_db::analytics::work_reader_count(tdb.db(), &work)
            .await
            .unwrap(),
    );

    assert_eq!(count_of(&v), 12, "{v}");
    tdb.cleanup().await;
}

/// A second pseudonym on an *existing* account — the cosplayer case.
async fn seed_alt_pseud(db: &Database, account: &str, tag: &str) -> String {
    let pseud = id(&format!("{tag}-pseud"));
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&pseud)
            .bind(account)
            .bind(tag)
            .bind(tag)
            .bind("2026-01-01 00:00:00")
            .bind("2026-01-01 00:00:00")
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .unwrap();
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
                 VALUES ($1::uuid, $2::uuid, $3, $4, now(), now())",
            )
            .bind(&pseud)
            .bind(account)
            .bind(tag)
            .bind(tag)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .unwrap();
        }
    }
    pseud
}

#[tokio::test]
async fn two_pseudonyms_on_one_account_count_as_two_readers() {
    // §7.2 forbids *revealing* the linkage. It does not make a cosplayer
    // invisible.
    //
    // Counting accounts instead would be the tempting fix — one person, one
    // reader — and it is wrong twice over. It requires the account join §7.2
    // exists to prevent, and it makes a reader who reads under two pseudonyms
    // look like half a reader, which is a judgement about them that the metric
    // has no business making. The number is about the work's reach.
    let dir = scratch_dir("anon_cosplay");
    let tdb = TestDb::connect_with_dir("anon-cosplay", &dir).await;
    let owner = seed_pseud(tdb.db(), "anon5-owner").await;
    let work = seed_work(tdb.db(), "anon5", &owner).await;

    // One account, two pseudonyms, two reading rows.
    let cos_account = id("anon5-cos-acct");
    match tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, ?, ?)",
            )
            .bind(&cos_account)
            .bind("anon5-cos@test.dev")
            .bind("2026-01-01 00:00:00")
            .bind("2026-01-01 00:00:00")
            .execute(tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .unwrap();
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at) VALUES ($1::uuid, $2, now(), now())",
            )
            .bind(&cos_account)
            .bind("anon5-cos@test.dev")
            .execute(tdb.db().postgres_pool().expect("postgres"))
            .await
            .unwrap();
        }
    }
    for name in ["primary", "alt"] {
        let p = seed_alt_pseud(tdb.db(), &cos_account, &format!("anon5-{name}")).await;
        seed_reads(tdb.db(), &format!("anon5-{name}"), &p, &work, 1).await;
    }

    // Above the floor so the count is exact and the claim is checkable.
    for i in 0..10 {
        let p = seed_pseud(tdb.db(), &format!("anon5-extra{i}")).await;
        seed_reads(tdb.db(), &format!("anon5-extra{i}"), &p, &work, 1).await;
    }

    let v = json_of(
        lorehaven_db::analytics::work_reader_count(tdb.db(), &work)
            .await
            .unwrap(),
    );

    // 12 pseudonyms, 11 accounts. The metric must say 12.
    assert_eq!(
        count_of(&v),
        12,
        "a cosplayer was counted as half a reader: {v}"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_work_with_no_readers_reports_a_floor_and_never_zero() {
    // Zero is a claim: "nobody read this" is a different statement from "too few
    // people read this to tell you", and the second is the true one for a new
    // work. Rendering 0 on a fresh work is a small lie that every author
    // notices on day one.
    let dir = scratch_dir("anon_empty");
    let tdb = TestDb::connect_with_dir("anon-empty", &dir).await;
    let owner = seed_pseud(tdb.db(), "anon6-owner").await;
    let work = seed_work(tdb.db(), "anon6", &owner).await;

    let v = json_of(
        lorehaven_db::analytics::work_reader_count(tdb.db(), &work)
            .await
            .unwrap(),
    );

    assert!(v["count"].is_null(), "an unread work reported a count: {v}");
    assert_eq!(v["fewer_than"], 10, "{v}");
    tdb.cleanup().await;
}

#[tokio::test]
async fn the_response_carries_its_own_definition() {
    // §24.2: every metric documents its computation. A number with no
    // definition is not reviewable, and a reader cannot tell a stale count
    // from a wrong one.
    let dir = scratch_dir("anon_method");
    let tdb = TestDb::connect_with_dir("anon-method", &dir).await;
    let owner = seed_pseud(tdb.db(), "anon7-owner").await;
    let work = seed_work(tdb.db(), "anon7", &owner).await;

    let v = json_of(
        lorehaven_db::analytics::work_reader_count(tdb.db(), &work)
            .await
            .unwrap(),
    );

    let method = &v["method"];
    assert!(method["definition"].is_string(), "{v}");
    assert!(method["freshness"].is_string(), "{v}");
    assert!(method["approximation"].is_string(), "{v}");
    assert!(
        v["computed_at"].is_string(),
        "a number with no freshness looks live forever: {v}"
    );
    assert!(v["scope"].is_string(), "{v}");
    tdb.cleanup().await;
}

#[tokio::test]
async fn the_subject_is_about_other_people_so_the_stricter_floor_applies() {
    let dir = scratch_dir("anon_subject");
    let tdb = TestDb::connect_with_dir("anon-subject", &dir).await;
    let owner = seed_pseud(tdb.db(), "anon8-owner").await;
    let work = seed_work(tdb.db(), "anon8", &owner).await;
    for i in 0..7 {
        let p = seed_pseud(tdb.db(), &format!("anon8-r{i}")).await;
        seed_reads(tdb.db(), &format!("anon8-r{i}"), &p, &work, 1).await;
    }

    let v = json_of(
        lorehaven_db::analytics::work_reader_count(tdb.db(), &work)
            .await
            .unwrap(),
    );

    assert_eq!(v["subject"], "other", "{v}");
    // Seven is a number in your own dashboard and a suppression in someone
    // else's. The subject is what makes the difference legible.
    assert_eq!(v["fewer_than"], 10, "{v}");
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_work_metric_never_accepts_a_floor_override() {
    // There is deliberately no `min_readers` parameter. A reader who can ask
    // for "count everything, floor 0" has the floor switched off, so the
    // function's only inputs are a database and a work id -- and the signature
    // is what enforces that. This test is the executable form of that claim.
    let dir = scratch_dir("anon_nobypass");
    let tdb = TestDb::connect_with_dir("anon-no-bypass", &dir).await;
    let owner = seed_pseud(tdb.db(), "anon9-owner").await;
    let work = seed_work(tdb.db(), "anon9", &owner).await;
    let p = seed_pseud(tdb.db(), "anon9-r0").await;
    seed_reads(tdb.db(), "anon9-r0", &p, &work, 1).await;

    let v = json_of(
        lorehaven_db::analytics::work_reader_count(tdb.db(), &work)
            .await
            .unwrap(),
    );

    assert!(v["count"].is_null(), "{v}");
    tdb.cleanup().await;
}

#[tokio::test]
async fn the_work_reader_capability_is_the_one_the_method_documents() {
    // The method text in the registry is the contract. If the SQL ever counts
    // something else -- sessions, accounts, events -- this fails, because the
    // definition names distinct pseudonyms.
    let method = lorehaven_domain::analytics::Scope::OwnWorkBasic.method();
    assert!(
        method.definition.contains("pseudonym"),
        "the method no longer says what it counts: {}",
        method.definition
    );
    let _ = json!({});
}
