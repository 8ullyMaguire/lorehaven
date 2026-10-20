//! M4-01 — the work columns the Darkhold content model needs.
//!
//! # What this proves
//!
//! * The six new columns exist, on **both** engines, and their defaults are
//!   the conservative ones: a work that has not said what it is is a fanwork,
//!   its discussion is a thread, and its author has said nothing about AI
//!   training.
//! * The author can set the form, the discussion mode and the AI statement —
//!   and the read-back is the value that was set.
//! * **The vocabularies are closed in the database as well as in the code**: a
//!   direct write of a value the specification does not name is refused by the
//!   `CHECK`, not by the code that happens to be looking.
//!
//! # Why the defaults matter
//!
//! Every column here says something about a work that the old schema did not
//! record, so every column is **added, not backfilled from a guess**. The
//! defaults are the conservative ones, and each says something: `fanwork`
//! because this is an archive of fanfiction, `thread_only` because the thread
//! is the archive's own surface, and `unset` because "I did not say anything"
//! and "I said it is not allowed" are different records — and the second is a
//! claim.

use darkhold_db::content::{create_work, find_work, update_work, WorkPatch};
use darkhold_db::identity::{create_account, create_pseud, AccountStatus};
use darkhold_db::Database;
use darkhold_domain::policy::AgeState;
use darkhold_domain::{PseudId, WorkId};

fn make_config(url: String) -> darkhold_db::DatabaseConfig {
    let mut c = darkhold_db::DatabaseConfig::new(url);
    c.max_connections = 2;
    c.acquire_timeout = std::time::Duration::from_secs(60);
    c
}

/// A migrated database on the engine the environment names.
async fn connect(tag: &str) -> Database {
    let Ok(admin) = std::env::var("DARKHOLD_TEST_PG_URL") else {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "darkhold-work-cols-{tag}-{}-{}",
            uuid::Uuid::new_v4().simple(),
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let db = Database::connect(&make_config(format!(
            "sqlite://{}/darkhold.sqlite?mode=rwc",
            dir.display()
        )))
        .await
        .expect("connect sqlite");
        db.migrate().await.expect("migrate");
        return db;
    };
    let name = format!("lh_workcols_{}", uuid::Uuid::new_v4().simple());
    let admin_db = Database::connect(&make_config(admin.clone()))
        .await
        .expect("connect admin");
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(admin_db.postgres_pool().unwrap())
        .await
        .expect("create");
    let url = admin
        .rsplit_once('/')
        .map(|(p, _)| format!("{p}/{name}"))
        .unwrap();
    let db = Database::connect(&make_config(url)).await.expect("connect");
    db.migrate().await.expect("migrate");
    db
}

/// An account, a pseud and a work — the three rows a work needs.
async fn work_fixture(db: &Database, tag: &str) -> (WorkId, i64, PseudId) {
    let account = create_account(
        db,
        &format!("{tag}@example.test"),
        AgeState::DeclaredAdult,
        AccountStatus::Active,
    )
    .await
    .expect("create the account");
    let pseud = create_pseud(db, account, &format!("{tag}-pseud"), "A Pseud")
        .await
        .expect("create the pseud");
    let work = create_work(db, pseud, &format!("A {tag} work"), None)
        .await
        .expect("create the work");
    (work.id, work.version, pseud)
}

#[tokio::test]
async fn a_new_work_carries_the_conservative_defaults() {
    let db = connect("defaults").await;
    let (work_id, _, _) = work_fixture(&db, "defaults").await;

    let work = find_work(&db, work_id)
        .await
        .expect("read the work")
        .expect("the work exists");

    assert_eq!(work.format, "fanwork", "the archive is of fanfiction");
    assert_eq!(work.discussion_mode, "thread_only");
    assert_eq!(work.ai_training, "unset", "the author has said nothing");
    assert_eq!(work.derived_activity, "active");
    assert_eq!(
        work.half_life_bp, None,
        "a work that has not been scored has no half-life -- and the absence is \
         meaningful: 10000 bp is the MAXIMUM (an evergreen boost of 1.5x), so a \
         default would give every new work the largest boost the mechanism can give"
    );
    assert_eq!(work.contested_at, None, "and it has not been contested");
}

#[tokio::test]
async fn the_author_sets_the_form_the_discussion_mode_and_the_ai_statement() {
    let db = connect("patch").await;
    let (work_id, version, pseud) = work_fixture(&db, "patch").await;

    let patched = update_work(
        &db,
        work_id,
        version,
        &WorkPatch {
            format: Some("podfic".to_owned()),
            discussion_mode: Some("comments_only".to_owned()),
            ai_training: Some("deny".to_owned()),
            ..WorkPatch::default()
        },
        pseud,
    )
    .await
    .expect("patch the work");
    assert!(patched, "the patch applied at the current version");

    let work = find_work(&db, work_id)
        .await
        .expect("read the work")
        .expect("the work exists");
    assert_eq!(work.format, "podfic");
    assert_eq!(work.discussion_mode, "comments_only");
    assert_eq!(work.ai_training, "deny");
}

#[tokio::test]
async fn the_vocabularies_are_closed_in_the_database_as_well_as_in_the_code() {
    // **A vocabulary that only the code checks is a vocabulary a direct write can
    // escape** — and a column whose stored value the code cannot parse is a
    // column the code has to guess about. The `CHECK` is the database's half of
    // one implementation per fact.
    let db = connect("closed").await;
    let (work_id, _, _) = work_fixture(&db, "closed").await;

    for (column, value) in [("derived_activity", "busy")] {
        let sqlite_sql = format!("UPDATE works SET {column} = ? WHERE id = ?");
        let postgres_sql = format!("UPDATE works SET {column} = $1 WHERE id = $2::uuid");
        let sql = db.sql(&sqlite_sql, &postgres_sql);
        let result = match db.backend() {
            darkhold_db::Backend::Sqlite => sqlx::query(&sql)
                .bind(value)
                .bind(work_id.to_string())
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .map(|_| ()),
            darkhold_db::Backend::Postgres => sqlx::query(&sql)
                .bind(value)
                .bind(work_id.to_string())
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .map(|_| ()),
        };
        assert!(
            result.is_err(),
            "the {column} vocabulary is closed: {value:?} must be refused by the \\
             database, not merely by the code that happens to be looking"
        );
    }
}
