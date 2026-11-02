//! M45-28 — §56 WIP adoption, closure notes and lineage, on both engines.
//!
//! What the domain unit tests cannot reach, and what this file pins:
//!
//! 1. An offer opens on an abandoned work the caller owns.
//! 2. A `complete` work cannot be offered — the refusal names `NotAbandoned`.
//! 3. A work somebody else owns cannot be offered, and says `NotOwner` rather than `404`.
//! 4. One open offer per work, and **a second offer after a withdrawal succeeds** — the partial
//!    unique index is the mechanism, and this is the test that would fail if it were a plain one.
//! 5. Withdrawal by anybody but the author is refused.
//! 6. **Adoption does not move `works.owner_pseud_id`** — read before, read after.
//! 7. A below-bar adoption reports both the bar and the caller's level.
//! 8. Adopting records the adopter and takes the offer off the open list.
//! 9. Lineage answers `available: false` with a reason when `revisions` is absent, and
//!    orders contributors by first contribution when it is present.
//! 10. Closure notes version 1, 2, 3 and the latest is the highest.
//! 11. A work with no closure note is `None`, never an error.
//!
//! Run on SQLite by default; set `DARKHOLD_TEST_PG_URL` for the PostgreSQL arm.

use std::time::Duration;

use darkhold_db::wip_adoption::{
    adopt, contributors, latest_closure_note, list_open, open_offer, open_offer_for,
    put_closure_note, withdraw,
};
use darkhold_db::{Backend, Database};
use darkhold_domain::wip_adoption::{AdoptionRefusal, OpenRefusal, WithdrawalRefusal};

const T0: &str = "2026-03-01T00:00:00Z";

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
        "darkhold-adoption-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let url = match std::env::var("DARKHOLD_TEST_PG_URL") {
        Ok(admin) => {
            let name = format!(
                "lh_adoption_{}_{}",
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

/// An account with one pseud, returning (account_id, pseud_id).
async fn who(db: &Database, tag: &str) -> (String, String) {
    let account = uuid::Uuid::new_v4().to_string();
    let pseud = uuid::Uuid::new_v4().to_string();
    let handle = format!("{tag}-{}", &account[..8]);
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, ?, ?)")
                .bind(&account)
                .bind(format!("{tag}@example.invalid"))
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
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at)
                 VALUES ($1::uuid, $2, $3::timestamptz, $3::timestamptz)",
            )
            .bind(uuid::Uuid::parse_str(&account).expect("uuid"))
            .bind(format!("{tag}@example.invalid"))
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
    }
    (account, pseud)
}

/// A work owned by `pseud`, in `completion` state. Returns the work id.
async fn work(db: &Database, pseud: &str, completion: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, completion, created_at, updated_at,
                                    generated_content_posture)
                 VALUES (?, ?, ?, ?, ?, ?, 'forbid')",
            )
            .bind(&id)
            .bind(pseud)
            .bind("An abandoned WIP")
            .bind(completion)
            .bind(T0)
            .bind(T0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("work");
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, completion, created_at, updated_at,
                                    generated_content_posture)
                 VALUES ($1::uuid, $2::uuid, $3, $4, $5::timestamptz, $5::timestamptz, 'forbid')",
            )
            .bind(uuid::Uuid::parse_str(&id).expect("uuid"))
            .bind(uuid::Uuid::parse_str(pseud).expect("uuid"))
            .bind("An abandoned WIP")
            .bind(completion)
            .bind(T0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("work");
        }
    }
    id
}

async fn owner_of(db: &Database, work_id: &str) -> String {
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query("SELECT owner_pseud_id FROM works WHERE id = ?")
                .bind(work_id)
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("owner");
            use sqlx::Row;
            row.get::<String, _>("owner_pseud_id")
        }
        Backend::Postgres => {
            let row = sqlx::query("SELECT owner_pseud_id::text AS owner FROM works WHERE id::text = $1")
                .bind(work_id)
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await
                .expect("owner");
            use sqlx::Row;
            row.get::<String, _>("owner")
        }
    }
}

#[tokio::test]
async fn an_offer_opens_on_an_abandoned_work_the_caller_owns() {
    let db = connect("open").await;
    let (_, author) = who(&db, "author").await;
    let work_id = work(&db, &author, "abandoned").await;

    let offer = open_offer(&db, &work_id, &author)
        .await
        .expect("no database error")
        .expect("opened");
    assert_eq!(offer.work_id, work_id);
    assert_eq!(offer.author_pseud_id, author);
    assert_eq!(offer.status, "open");
    assert!(offer.adopted_by_pseud_id.is_none());
}

#[tokio::test]
async fn a_complete_work_cannot_be_offered() {
    let db = connect("complete").await;
    let (_, author) = who(&db, "author").await;
    let work_id = work(&db, &author, "complete").await;

    let outcome = open_offer(&db, &work_id, &author)
        .await
        .expect("no database error");
    assert_eq!(outcome, Err(OpenRefusal::NotAbandoned));
    assert!(
        open_offer_for(&db, &work_id).await.expect("read").is_none(),
        "a refusal wrote a row"
    );
}

#[tokio::test]
async fn only_abandoned_is_offerable_in_progress_and_hiatus_are_not() {
    // Measured hole: the first cut of this file tested `complete` and `abandoned` only, and the
    // mutation that let `in_progress` through survived. A WIP still being written and a WIP on
    // hiatus are both works the author means to finish.
    let db = connect("states").await;
    let (_, author) = who(&db, "author").await;

    for completion in ["in_progress", "hiatus", "complete"] {
        let work_id = work(&db, &author, completion).await;
        assert_eq!(
            open_offer(&db, &work_id, &author).await.expect("read"),
            Err(OpenRefusal::NotAbandoned),
            "{completion} must not be offerable"
        );
    }

    let abandoned = work(&db, &author, "abandoned").await;
    assert!(
        open_offer(&db, &abandoned, &author).await.expect("read").is_ok(),
        "abandoned must be offerable"
    );
}

#[tokio::test]
async fn a_work_somebody_else_owns_cannot_be_offered() {
    let db = connect("notowner").await;
    let (_, author) = who(&db, "author").await;
    let (_, stranger) = who(&db, "stranger").await;
    let work_id = work(&db, &author, "abandoned").await;

    let outcome = open_offer(&db, &work_id, &stranger)
        .await
        .expect("no database error");
    assert_eq!(outcome, Err(OpenRefusal::NotOwner));
    assert!(open_offer_for(&db, &work_id).await.expect("read").is_none());
}

#[tokio::test]
async fn one_open_offer_per_work_and_a_second_after_withdrawal_succeeds() {
    let db = connect("partial").await;
    let (_, author) = who(&db, "author").await;
    let work_id = work(&db, &author, "abandoned").await;

    let first = open_offer(&db, &work_id, &author)
        .await
        .expect("no database error")
        .expect("opened");
    assert_eq!(
        open_offer(&db, &work_id, &author).await.expect("read"),
        Err(OpenRefusal::AlreadyOpen),
        "a second open offer was allowed"
    );

    withdraw(&db, &first.id, &author)
        .await
        .expect("no database error")
        .expect("withdrawn");

    // The partial index is `WHERE status = 'open'`, so the withdrawn row does not block this.
    let second = open_offer(&db, &work_id, &author)
        .await
        .expect("no database error")
        .expect("a second offer after a withdrawal is allowed");
    assert_ne!(second.id, first.id);
}

#[tokio::test]
async fn withdrawal_by_anybody_but_the_author_is_refused() {
    let db = connect("withdraw").await;
    let (_, author) = who(&db, "author").await;
    let (_, stranger) = who(&db, "stranger").await;
    let work_id = work(&db, &author, "abandoned").await;
    let offer = open_offer(&db, &work_id, &author)
        .await
        .expect("read")
        .expect("opened");

    assert_eq!(
        withdraw(&db, &offer.id, &stranger).await.expect("read"),
        Err(WithdrawalRefusal::NotTheAuthor)
    );
    let still = open_offer_for(&db, &work_id)
        .await
        .expect("read")
        .expect("still open");
    assert_eq!(still.id, offer.id);
}

#[tokio::test]
async fn adoption_does_not_move_the_work_to_a_new_owner() {
    let db = connect("nomove").await;
    let (_, author) = who(&db, "author").await;
    let (_, adopter) = who(&db, "adopter").await;
    let work_id = work(&db, &author, "abandoned").await;
    let offer = open_offer(&db, &work_id, &author)
        .await
        .expect("read")
        .expect("opened");

    let before = owner_of(&db, &work_id).await;
    let adopted = adopt(&db, &offer.id, &adopter, 2, 5)
        .await
        .expect("no database error")
        .expect("adopted");
    let after = owner_of(&db, &work_id).await;

    assert_eq!(before, author, "the fixture started with the author as owner");
    assert_eq!(after, before, "adoption moved owner_pseud_id — §56.1");
    assert_eq!(adopted.adopted_by_pseud_id.as_deref(), Some(adopter.as_str()));
}

#[tokio::test]
async fn a_below_bar_adoption_reports_both_numbers() {
    let db = connect("bar").await;
    let (_, author) = who(&db, "author").await;
    let (_, adopter) = who(&db, "adopter").await;
    let work_id = work(&db, &author, "abandoned").await;
    let offer = open_offer(&db, &work_id, &author)
        .await
        .expect("read")
        .expect("opened");

    assert_eq!(
        adopt(&db, &offer.id, &adopter, 4, 1).await.expect("read"),
        Err(AdoptionRefusal::BelowBar { bar: 4, caller: 1 })
    );
    let still_open = open_offer_for(&db, &work_id)
        .await
        .expect("read")
        .expect("an adoption that did not happen left the offer open");
    assert_eq!(still_open.status, "open");
}

#[tokio::test]
async fn adopting_records_the_adopter_and_empties_the_offer_list() {
    let db = connect("adopted").await;
    let (_, author) = who(&db, "author").await;
    let (_, adopter) = who(&db, "adopter").await;
    let work_id = work(&db, &author, "abandoned").await;
    let offer = open_offer(&db, &work_id, &author)
        .await
        .expect("read")
        .expect("opened");
    assert_eq!(list_open(&db, 10).await.expect("list").len(), 1);

    adopt(&db, &offer.id, &adopter, 2, 2)
        .await
        .expect("read")
        .expect("adopted");

    assert!(
        list_open(&db, 10).await.expect("list").is_empty(),
        "an adopted offer stayed on the open list"
    );
    assert_eq!(
        adopt(&db, &offer.id, &adopter, 2, 2).await.expect("read"),
        Err(AdoptionRefusal::NotOpen),
        "an offer was adopted twice"
    );
}

#[tokio::test]
async fn lineage_is_unavailable_in_words_when_the_table_is_absent() {
    let db = connect("nolineage").await;
    let (_, author) = who(&db, "author").await;
    let work_id = work(&db, &author, "abandoned").await;

    // A schema with no revision table: lineage is a named answer, not a 500.
    let drop = "DROP TABLE revisions";
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(drop)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("drop");
        }
        Backend::Postgres => {
            sqlx::query(&format!("{drop} CASCADE"))
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("drop");
        }
    }

    let lineage = contributors(&db, &work_id).await.expect("no database error");
    assert!(lineage.contributors.is_empty());
    assert!(!lineage.available, "claimed lineage it could not read");
    let reason = lineage.reason.expect("a reason");
    assert!(
        reason.contains("revisions"),
        "the reason should name the missing table, got {reason:?}"
    );
}

#[tokio::test]
async fn lineage_orders_by_first_contribution_not_by_volume() {
    let db = connect("lineage").await;
    let (_, author) = who(&db, "author").await;
    let (_, late) = who(&db, "late").await;
    let work_id = work(&db, &author, "abandoned").await;
    let unit = uuid::Uuid::new_v4().to_string();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO units (id, work_id, order_key, title, created_at, updated_at)
                 VALUES (?, ?, 10, 'one', ?, ?)",
            )
            .bind(&unit)
            .bind(&work_id)
            .bind(T0)
            .bind(T0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("chapter");
            for (n, (pseud, at)) in [
                (1, (&author, "2026-01-01T00:00:00Z")),
                (2, (&author, "2026-02-01T00:00:00Z")),
                (3, (&late, "2026-02-15T00:00:00Z")),
            ] {
                sqlx::query(
                    "INSERT INTO revisions (id, unit_id, revision_number, document_json,
                        sanitized_html, plain_text, word_count, note, created_by_pseud_id,
                        restored_from_id, created_at)
                     VALUES (?, ?, ?, '{}', '', 'text', 10, NULL, ?, NULL, ?)",
                )
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(&unit)
                .bind(n as i64)
                .bind(pseud)
                .bind(at)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("revision");
            }
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO units (id, work_id, order_key, title, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, 10, 'one', $3::timestamptz, $3::timestamptz)",
            )
            .bind(uuid::Uuid::parse_str(&unit).expect("uuid"))
            .bind(uuid::Uuid::parse_str(&work_id).expect("uuid"))
            .bind(T0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("chapter");
            for (n, (pseud, at)) in [
                (1, (&author, "2026-01-01T00:00:00Z")),
                (2, (&author, "2026-02-01T00:00:00Z")),
                (3, (&late, "2026-02-15T00:00:00Z")),
            ] {
                sqlx::query(
                    "INSERT INTO revisions (id, unit_id, revision_number, document_json,
                        sanitized_html, plain_text, word_count, note, created_by_pseud_id,
                        restored_from_id, created_at)
                     VALUES ($1::uuid, $2::uuid, $3, '{}', '', 'text', 10, NULL, $4::uuid, NULL, $5::timestamptz)",
                )
                .bind(uuid::Uuid::new_v4())
                .bind(uuid::Uuid::parse_str(&unit).expect("uuid"))
                .bind(n as i64)
                .bind(uuid::Uuid::parse_str(pseud).expect("uuid"))
                .bind(at)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("revision");
            }
        }
    }

    let lineage = contributors(&db, &work_id).await.expect("read");
    assert!(lineage.available, "the table is present here");
    assert_eq!(lineage.contributors.len(), 2);
    assert_eq!(
        lineage.contributors[0].pseud_id, author,
        "the author started first and must come first even though both wrote for the same span"
    );
    assert_eq!(lineage.contributors[0].revisions, 2);
    assert_eq!(lineage.contributors[1].pseud_id, late);
    assert_eq!(lineage.contributors[1].revisions, 1);
}

#[tokio::test]
async fn closure_notes_version_upward_and_the_latest_is_the_highest() {
    let db = connect("notes").await;
    let (_, author) = who(&db, "author").await;
    let work_id = work(&db, &author, "abandoned").await;

    assert!(
        latest_closure_note(&db, &work_id).await.expect("read").is_none(),
        "a work with no note is None, not an error"
    );

    for (n, body) in ["first: it ends here", "second: on reflection, here instead"].iter().enumerate() {
        let note = put_closure_note(&db, &work_id, &author, body).await.expect("write");
        assert_eq!(note.revision_number, n as i64 + 1);
        assert_eq!(&note.body, body);
    }

    let latest = latest_closure_note(&db, &work_id)
        .await
        .expect("read")
        .expect("a note");
    assert_eq!(latest.revision_number, 2);
    assert_eq!(latest.body, "second: on reflection, here instead");
}

#[tokio::test]
async fn a_work_that_is_not_abandoned_any_more_cannot_be_adopted() {
    let db = connect("stopped").await;
    let (_, author) = who(&db, "author").await;
    let (_, adopter) = who(&db, "adopter").await;
    let work_id = work(&db, &author, "abandoned").await;
    let offer = open_offer(&db, &work_id, &author)
        .await
        .expect("read")
        .expect("opened");

    // The author comes back: the offer is stale, and the work is no longer adoptable.
    let sql = match db.backend() {
        Backend::Sqlite => "UPDATE works SET completion = 'in_progress' WHERE id = ?",
        Backend::Postgres => "UPDATE works SET completion = 'in_progress' WHERE id::text = $1",
    };
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(sql)
                .bind(&work_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("update");
        }
        Backend::Postgres => {
            sqlx::query(sql)
                .bind(&work_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("update");
        }
    }

    assert_eq!(
        adopt(&db, &offer.id, &adopter, 2, 9).await.expect("read"),
        Err(AdoptionRefusal::NotAbandoned)
    );
}
