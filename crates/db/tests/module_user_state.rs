//! M65-04: per-user module state, on both engines.
//!
//! Five properties, and each one is a way a preference table goes wrong:
//!
//! 1. **No row means inherit.** `is_off` is `false` for an account that never wrote
//!    one — the instance's decision is the answer, and a reader who has not opened
//!    settings must not be treated as having said "on".
//! 2. **Narrowing is idempotent and observable.** Writing twice leaves one row, and
//!    the reader's list contains the id once.
//! 3. **Clearing means inherit again.** `clear` removes the row and reports whether
//!    there was one, so a caller can tell "I removed your change" from "there was
//!    nothing to remove".
//! 4. **One account's row is not another's.** The narrowing is keyed on the account,
//!    which is the whole point: a shared "instance was told to hide arena" flag is
//!    the bug this table exists to prevent.
//! 5. **The count is instance-wide**, because that is the number an operator reads
//!    before wondering whether the feature works at all.
//!
//! Run on SQLite by default; set `DARKHOLD_TEST_PG_URL` for the PostgreSQL arm.

use std::time::Duration;

use darkhold_db::{module_state, Database};

const T0: &str = "2026-11-02T00:00:00Z";

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
        "darkhold-modstate-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4().simple(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let url = match std::env::var("DARKHOLD_TEST_PG_URL") {
        Ok(admin) => {
            let name = format!(
                "lh_modstate_{}_{}",
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
            let (prefix, _) = admin.rsplit_once('/').expect("admin URL ends in a database");
            format!("{prefix}/{name}")
        }
        Err(_) => format!("sqlite://{}/darkhold.sqlite?mode=rwc", dir.display()),
    };
    let db = Database::connect(&make_config(url)).await.expect("connect");
    db.migrate().await.expect("migrate");
    db
}

/// An account row, because `module_user_state` names one.
async fn account(db: &Database) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let email = format!("{id}@example.invalid");
    match db.backend() {
        darkhold_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO accounts (id, email, status, age_state, created_at, updated_at, permission_statement)
                 VALUES ($1::uuid, $2, 'active', 'adult', $3, $3, 'minimal')",
            )
            .bind(uuid::Uuid::parse_str(&id).expect("uuid"))
            .bind(&email)
            .bind(T0)
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .expect("account");
        }
        darkhold_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO accounts (id, email, status, age_state, created_at, updated_at, permission_statement)
                 VALUES (?, ?, 'active', 'adult', ?, ?, 'minimal')",
            )
            .bind(&id)
            .bind(&email)
            .bind(T0)
            .bind(T0)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("account");
        }
    }
    id
}

#[tokio::test]
async fn a_reader_with_no_row_inherits() {
    let db = connect("inherit").await;
    let who = account(&db).await;

    assert!(
        !module_state::is_off(&db, &who, "arena")
            .await
            .expect("read"),
        "no row must mean the instance's decision, not a stored 'on'"
    );
    assert!(module_state::overrides_for(&db, &who)
        .await
        .expect("list")
        .is_empty());
}

#[tokio::test]
async fn narrowing_is_idempotent_and_listed_once() {
    let db = connect("idempotent").await;
    let who = account(&db).await;

    module_state::set_off(&db, &who, "arena", T0)
        .await
        .expect("first write");
    module_state::set_off(&db, &who, "arena", T0)
        .await
        .expect("second write");

    assert!(module_state::is_off(&db, &who, "arena").await.expect("read"));
    assert_eq!(
        module_state::overrides_for(&db, &who).await.expect("list"),
        vec!["arena".to_owned()],
        "one row per account per module, however many times it is written"
    );
    assert_eq!(module_state::count(&db).await.expect("count"), 1);
}

#[tokio::test]
async fn clearing_returns_to_the_instance_decision() {
    let db = connect("clear").await;
    let who = account(&db).await;

    module_state::set_off(&db, &who, "quiz", T0)
        .await
        .expect("write");
    assert!(
        module_state::clear(&db, &who, "quiz").await.expect("clear"),
        "the row was there"
    );
    assert!(
        !module_state::clear(&db, &who, "quiz").await.expect("clear"),
        "the second clear removed nothing, and says so"
    );
    assert!(!module_state::is_off(&db, &who, "quiz").await.expect("read"));
    assert_eq!(module_state::count(&db).await.expect("count"), 0);
}

#[tokio::test]
async fn one_accounts_narrowing_is_not_anothers() {
    let db = connect("per-account").await;
    let alice = account(&db).await;
    let bob = account(&db).await;

    module_state::set_off(&db, &alice, "arena", T0)
        .await
        .expect("write");

    assert!(module_state::is_off(&db, &alice, "arena").await.expect("read"));
    assert!(
        !module_state::is_off(&db, &bob, "arena").await.expect("read"),
        "bob never wrote a row; a per-account table that answers for him is not per-account"
    );
    // And the count is over accounts, not over people who agree with each other.
    assert_eq!(module_state::count(&db).await.expect("count"), 1);
}

#[tokio::test]
async fn two_rows_for_one_account_are_two_changes() {
    let db = connect("two-rows").await;
    let who = account(&db).await;

    module_state::set_off(&db, &who, "arena", T0)
        .await
        .expect("write");
    module_state::set_off(&db, &who, "narration", T0)
        .await
        .expect("write");

    assert_eq!(
        module_state::overrides_for(&db, &who).await.expect("list"),
        vec!["arena".to_owned(), "narration".to_owned()],
        "listed in id order, so a settings page renders a stable list"
    );
    assert_eq!(module_state::count(&db).await.expect("count"), 2);
}
