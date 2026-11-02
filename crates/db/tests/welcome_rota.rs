//! M45-26 — the welcome rota, on both engines.
//!
//! The store's arithmetic is small; what this file pins is the five properties that
//! only exist once a database is involved, and each is a way the feature could be
//! built and be useless:
//!
//! 1. **A commented work is never offered** — the rota exists to produce a first
//!    comment, so a work that already has one is not its business.
//! 2. **A work with an open welcome is not offered to anybody else** — two readers
//!    on one work is two promises for one comment.
//! 3. **One author gets one welcome** — the unit is an author hearing from someone,
//!    not a work.
//! 4. **A reader holds one open welcome at a time** — otherwise the 72 hours is a
//!    deadline on nothing.
//! 5. **The rota closes on the comment, not on the button** — and a comment written
//!    *before* the assignment does not count, because the work had none when it was
//!    offered.
//!
//! Run on SQLite by default; set `DARKHOLD_TEST_PG_URL` for the PostgreSQL arm.

use std::time::Duration;

use darkhold_db::welcome::{self, Claim, Complete, DUE_HOURS};
use darkhold_db::Database;

const T0: &str = "2026-01-01T00:00:00Z";

fn make_config(url: String) -> darkhold_db::DatabaseConfig {
    darkhold_db::DatabaseConfig {
        url,
        max_connections: 2,
        acquire_timeout: Duration::from_secs(5),
        slow_query_warn: Duration::ZERO,
    }
}

async fn connect(tag: &str) -> Database {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "darkhold-welcome-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let url = match std::env::var("DARKHOLD_TEST_PG_URL") {
        Ok(admin) => {
            let name = format!(
                "lh_welcome_{}_{}",
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
        Err(_) => format!("sqlite://{}/darkhold.sqlite?mode=rwc", dir.display()),
    };
    let db = Database::connect(&make_config(url)).await.expect("connect");
    db.migrate().await.expect("migrate");
    db
}

/// A reader: an account and its pseud. `welcome` promises are made to pseuds.
struct Reader {
    account: String,
    pseud: String,
}

async fn reader(db: &Database, tag: &str) -> Reader {
    let run = uuid::Uuid::new_v4().to_string();
    let account = uuid::Uuid::new_v4().to_string();
    let pseud = uuid::Uuid::new_v4().to_string();
    let handle = format!("p{tag}-{run}");
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at)
                 VALUES ($1::uuid, $2, $3::timestamptz, $3::timestamptz)",
            )
            .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
            .bind(format!("{tag}-{run}@example.com"))
            .bind(T0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("account");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, $3, $3, $4::timestamptz, $4::timestamptz)",
            )
            .bind(uuid::Uuid::parse_str(&pseud).expect("uuid"))
            .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
            .bind(&handle)
            .bind(T0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("pseud");
        }
        darkhold_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, ?, ?)",
            )
            .bind(&account)
            .bind(format!("{tag}-{run}@example.com"))
            .bind(T0)
            .bind(T0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("account");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&pseud)
            .bind(&account)
            .bind(&handle)
            .bind(&handle)
            .bind(T0)
            .bind(T0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("pseud");
        }
    }
    Reader { account, pseud }
}

/// A catalogue row, with an optional work id. Returns the library item id.
async fn item(
    db: &Database,
    account: &str,
    owner_pseud: &str,
    key: &str,
    author: &str,
    status: &str,
    likes: i64,
    work: Option<&str>,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            // A work id on a library row is a foreign key, so the work has to exist.
            if work.is_some() {
                sqlx::query(
                    "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at,
                                        generated_content_posture)
                     VALUES ($1::uuid, $2::uuid, $3, $4::timestamptz, $4::timestamptz, 'forbid')
                     ON CONFLICT DO NOTHING",
                )
                .bind(uuid::Uuid::parse_str(work.expect("work id")).expect("uuid"))
                .bind(uuid::Uuid::parse_str(owner_pseud).expect("uuid"))
                .bind(format!("{key} work"))
                .bind(T0)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("work");
            }
            sqlx::query(
                "INSERT INTO library_items
                 (id, account_id, source_key, source_work_key, title, author_text, summary,
                  status, source_url, created_at, updated_at, likes, work_id)
                 VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6, '', $7,
                         'https://example.invalid/w', $8, $8, $9, $10::uuid)",
            )
            .bind(uuid::Uuid::parse_str(&id).expect("uuid"))
            .bind(uuid::Uuid::parse_str(account).expect("uuid"))
            .bind("sb")
            .bind(key)
            .bind(format!("{key} title"))
            .bind(author)
            .bind(status)
            .bind(T0)
            .bind(likes)
            .bind(work.map(|w| uuid::Uuid::parse_str(w).expect("uuid")))
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("library item");
        }
        darkhold_db::Backend::Sqlite => {
            if let Some(w) = work {
                sqlx::query(
                    "INSERT OR IGNORE INTO works (id, owner_pseud_id, title, created_at, updated_at,
                                        generated_content_posture)
                     VALUES (?, ?, ?, ?, ?, 'forbid')",
                )
                .bind(w)
                .bind(owner_pseud)
                .bind(format!("{key} work"))
                .bind(T0)
                .bind(T0)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("work");
            }
            sqlx::query(
                "INSERT INTO library_items
                 (id, account_id, source_key, source_work_key, title, author_text, summary,
                  status, source_url, created_at, updated_at, likes, work_id)
                 VALUES (?, ?, ?, ?, ?, ?, '', ?, 'https://example.invalid/w', ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(account)
            .bind("sb")
            .bind(key)
            .bind(format!("{key} title"))
            .bind(author)
            .bind(status)
            .bind(T0)
            .bind(T0)
            .bind(likes)
            .bind(work)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("library item");
        }
    }
    id
}

async fn comment(db: &Database, work: &str, pseud: &str, at: &str) {
    let id = uuid::Uuid::new_v4().to_string();
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO comments (id, subject_type, subject_id, author_pseud, body,
                                       body_version, created_at)
                 VALUES ($1::uuid, 'work', $2, $3::uuid, 'a first comment', 'v1', $4)",
            )
            .bind(uuid::Uuid::parse_str(&id).expect("uuid"))
            .bind(work)
            .bind(uuid::Uuid::parse_str(pseud).expect("uuid"))
            .bind(at)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("comment");
        }
        darkhold_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO comments (id, subject_type, subject_id, author_pseud, body,
                                       body_version, created_at)
                 VALUES (?, 'work', ?, ?, 'a first comment', 'v1', ?)",
            )
            .bind(&id)
            .bind(work)
            .bind(pseud)
            .bind(at)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("comment");
        }
    }
}

fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[tokio::test]
async fn a_work_without_a_comment_is_offered() {
    let db = connect("offered").await;
    let r = reader(&db, "offered").await;
    let w = uuid();
    item(&db, &r.account, &r.pseud, "a", "author-a", "ongoing", 10, Some(&w)).await;

    match welcome::claim(&db, &r.pseud).await.expect("claim") {
        Claim::Assigned(a) => {
            assert_eq!(a.work_id, w);
            assert!(a.completed_at.is_none());
            assert!(!a.overdue);
        }
        other => panic!("expected an assignment, got {other:?}"),
    }
}

#[tokio::test]
async fn a_commented_work_is_never_offered() {
    let db = connect("commented").await;
    let r = reader(&db, "commented").await;
    let commented = uuid();
    let open = uuid();
    // The commented work would win on endorsement, so if the exclusion is dropped
    // this test fails by *which* work came back, not merely by whether one did.
    item(&db, &r.account, &r.pseud, "a", "author-a", "ongoing", 99, Some(&commented)).await;
    item(&db, &r.account, &r.pseud, "b", "author-b", "ongoing", 1, Some(&open)).await;
    comment(&db, &commented, &r.pseud, T0).await;

    match welcome::claim(&db, &r.pseud).await.expect("claim") {
        Claim::Assigned(a) => assert_eq!(a.work_id, open, "the commented work must not be offered"),
        other => panic!("expected the uncommented work, got {other:?}"),
    }
}

#[tokio::test]
async fn a_work_with_an_open_welcome_is_not_offered_to_anybody_else() {
    let db = connect("taken").await;
    let first = reader(&db, "taken-one").await;
    let second = reader(&db, "taken-two").await;
    let w = uuid();
    // An UNKNOWN author, deliberately. `one_author_gets_one_welcome` shows the author
    // rule already blocks a re-offer, so with a named author this clause is masked and
    // the test passes whether the clause exists or not — measured: it did. The one
    // author key the author rule exempts (empty) is therefore the only fixture that
    // isolates "this work is already promised to someone", which is what the clause is
    // for: two readers on one work is two promises for one comment.
    item(&db, &first.account, &first.pseud, "a", "", "ongoing", 10, Some(&w)).await;

    assert!(matches!(
        welcome::claim(&db, &first.pseud).await.expect("claim"),
        Claim::Assigned(_)
    ));
    match welcome::claim(&db, &second.pseud).await.expect("claim") {
        Claim::NothingToWelcome { eligible } => assert_eq!(eligible, 1),
        other => panic!("the second reader must not be handed the same work: {other:?}"),
    }
}

#[tokio::test]
async fn one_author_gets_one_welcome() {
    let db = connect("author").await;
    let first = reader(&db, "author-one").await;
    let second = reader(&db, "author-two").await;
    let a = uuid();
    let b = uuid();
    // Two works, one author. The author is welcomed once, so the second reader finds
    // nothing even though a second work is uncommented and untaken.
    item(&db, &first.account, &first.pseud, "a", "Same Author", "ongoing", 10, Some(&a)).await;
    item(&db, &first.account, &first.pseud, "b", "Same Author", "ongoing", 5, Some(&b)).await;

    assert!(matches!(
        welcome::claim(&db, &first.pseud).await.expect("claim"),
        Claim::Assigned(_)
    ));
    match welcome::claim(&db, &second.pseud).await.expect("claim") {
        Claim::NothingToWelcome { eligible } => assert_eq!(eligible, 2),
        other => panic!("one author, one welcome: {other:?}"),
    }
}

#[tokio::test]
async fn a_reader_holds_one_open_welcome_at_a_time() {
    let db = connect("oneopen").await;
    let r = reader(&db, "oneopen").await;
    item(&db, &r.account, &r.pseud, "a", "author-a", "ongoing", 10, Some(&uuid())).await;
    item(&db, &r.account, &r.pseud, "b", "author-b", "ongoing", 5, Some(&uuid())).await;

    let first = match welcome::claim(&db, &r.pseud).await.expect("claim") {
        Claim::Assigned(a) => a,
        other => panic!("expected an assignment: {other:?}"),
    };
    match welcome::claim(&db, &r.pseud).await.expect("claim again") {
        Claim::AlreadyOpen(a) => {
            assert_eq!(a.id, first.id, "the open welcome is returned, not a second one")
        }
        other => panic!("a second claim must not take another work: {other:?}"),
    }
    assert_eq!(welcome::mine(&db, &r.pseud).await.expect("mine").len(), 1);
}

#[tokio::test]
async fn complete_refuses_without_a_comment() {
    let db = connect("nocomment").await;
    let r = reader(&db, "nocomment").await;
    let w = uuid();
    item(&db, &r.account, &r.pseud, "a", "author-a", "ongoing", 10, Some(&w)).await;
    let a = match welcome::claim(&db, &r.pseud).await.expect("claim") {
        Claim::Assigned(a) => a,
        other => panic!("expected an assignment: {other:?}"),
    };

    match welcome::complete(&db, &r.pseud, &a.id).await.expect("complete") {
        Complete::NoCommentYet { work_id } => assert_eq!(work_id, w),
        other => panic!("the rota must not close without the comment: {other:?}"),
    }
    // Nothing was written: the row is still the reader's open welcome.
    let open = welcome::open_for(&db, &r.pseud)
        .await
        .expect("open")
        .expect("still open");
    assert_eq!(open.id, a.id);
    assert!(open.completed_at.is_none());
}

#[tokio::test]
async fn complete_accepts_the_comment_that_evidences_it() {
    let db = connect("evidence").await;
    let r = reader(&db, "evidence").await;
    let w = uuid();
    item(&db, &r.account, &r.pseud, "a", "author-a", "ongoing", 10, Some(&w)).await;
    let a = match welcome::claim(&db, &r.pseud).await.expect("claim") {
        Claim::Assigned(a) => a,
        other => panic!("expected an assignment: {other:?}"),
    };
    // After the assignment, in the same canonical shape the column accepts.
    comment(&db, &w, &r.pseud, &darkhold_db::identity::now_rfc3339()).await;

    match welcome::complete(&db, &r.pseud, &a.id).await.expect("complete") {
        Complete::Done(done) => {
            assert!(done.completed_at.is_some());
            assert!(!done.overdue, "a closed welcome is never overdue");
        }
        other => panic!("the comment evidences the welcome: {other:?}"),
    }
    assert!(
        welcome::open_for(&db, &r.pseud).await.expect("open").is_none(),
        "a closed welcome is not the reader's open one"
    );
}

#[tokio::test]
async fn a_comment_from_before_the_assignment_does_not_close_it() {
    let db = connect("stale").await;
    let r = reader(&db, "stale").await;
    let w = uuid();
    item(&db, &r.account, &r.pseud, "a", "author-a", "ongoing", 10, Some(&w)).await;
    let a = match welcome::claim(&db, &r.pseud).await.expect("claim") {
        Claim::Assigned(a) => a,
        other => panic!("expected an assignment: {other:?}"),
    };
    // The row exists now, but the comment is backdated to before the welcome was
    // taken — which is what a comment written earlier would look like. The window is
    // the whole of the evidence rule, so this must not close the rota.
    comment(&db, &w, &r.pseud, T0).await;

    match welcome::complete(&db, &r.pseud, &a.id).await.expect("complete") {
        Complete::NoCommentYet { .. } => {}
        other => panic!("a comment older than the assignment is not evidence: {other:?}"),
    }
}

#[tokio::test]
async fn the_due_date_is_seventy_two_hours_out() {
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;

    let db = connect("due").await;
    let r = reader(&db, "due").await;
    item(&db, &r.account, &r.pseud, "a", "author-a", "ongoing", 10, Some(&uuid())).await;
    let a = match welcome::claim(&db, &r.pseud).await.expect("claim") {
        Claim::Assigned(a) => a,
        other => panic!("expected an assignment: {other:?}"),
    };
    let assigned = OffsetDateTime::parse(&a.assigned_at, &Rfc3339).expect("assigned");
    let due = OffsetDateTime::parse(&a.due_at, &Rfc3339).expect("due");
    // Literally 72, not `DUE_HOURS`: asserting a value against the constant that
    // produces it is a tautology, and the mutation that changed the promise to 48
    // hours passed this test unchanged.
    assert_eq!((due - assigned).whole_hours(), 72);
    assert_eq!(DUE_HOURS, 72, "the promise is three days");
}

#[tokio::test]
async fn an_unfinished_unendorsed_work_is_never_offered() {
    let db = connect("ineligible").await;
    let r = reader(&db, "ineligible").await;
    // No likes, not finished: nothing a reader could be asked to welcome yet.
    item(&db, &r.account, &r.pseud, "a", "author-a", "ongoing", 0, Some(&uuid())).await;
    match welcome::claim(&db, &r.pseud).await.expect("claim") {
        Claim::NothingToWelcome { eligible } => assert_eq!(eligible, 0),
        other => panic!("an unread, unfinished work is not a welcome: {other:?}"),
    }
}

#[tokio::test]
async fn a_finished_work_is_offered_even_with_no_endorsement() {
    let db = connect("finished").await;
    let r = reader(&db, "finished").await;
    let w = uuid();
    // The live catalogue's shape, measured 2026-11-02: 397 rows are `complete` with no
    // likes, and 0 rows are both. A rule of "complete AND endorsed" routes nothing.
    item(&db, &r.account, &r.pseud, "a", "author-a", "complete", 0, Some(&w)).await;
    match welcome::claim(&db, &r.pseud).await.expect("claim") {
        Claim::Assigned(a) => assert_eq!(a.work_id, w),
        other => panic!("a finished work is welcomable: {other:?}"),
    }
}

#[tokio::test]
async fn a_library_row_with_no_work_id_is_never_offered() {
    let db = connect("nowork").await;
    let r = reader(&db, "nowork").await;
    // Endorsed, unfinished, uncommented — but no work id, so no comment could ever
    // attach to it and offering it would be a promise nobody could keep.
    item(&db, &r.account, &r.pseud, "a", "author-a", "ongoing", 500, None).await;
    match welcome::claim(&db, &r.pseud).await.expect("claim") {
        Claim::NothingToWelcome { eligible } => assert_eq!(eligible, 0),
        other => panic!("a row with no work id cannot be welcomed: {other:?}"),
    }
}

#[tokio::test]
async fn the_report_counts_need_and_keeping() {
    let db = connect("report").await;
    let one = reader(&db, "report-one").await;
    let two = reader(&db, "report-two").await;
    let w1 = uuid();
    let w2 = uuid();
    item(&db, &one.account, &one.pseud, "a", "author-a", "ongoing", 10, Some(&w1)).await;
    item(&db, &one.account, &one.pseud, "b", "author-b", "ongoing", 5, Some(&w2)).await;

    let empty = welcome::report(&db).await.expect("report");
    assert!(empty.measured);
    assert_eq!(empty.eligible, 2);
    assert_eq!(empty.assigned, 0);

    let a = match welcome::claim(&db, &one.pseud).await.expect("claim") {
        Claim::Assigned(a) => a,
        other => panic!("expected an assignment: {other:?}"),
    };
    comment(&db, &a.work_id, &one.pseud, &darkhold_db::identity::now_rfc3339()).await;
    assert!(matches!(
        welcome::complete(&db, &one.pseud, &a.id).await.expect("complete"),
        Complete::Done(_)
    ));
    assert!(matches!(
        welcome::claim(&db, &two.pseud).await.expect("claim"),
        Claim::Assigned(_)
    ));

    let after = welcome::report(&db).await.expect("report");
    assert_eq!(after.assigned, 2);
    assert_eq!(after.completed, 1);
    assert_eq!(after.open, 1);
    assert_eq!(after.overdue, 0);
}
