//! M45-22 step 1: the blend must exclude works the reader has already read.
//!
//! Both halves of this were dead code before this file existed. `rec_engine`
//! passed `seen: vec![]`, and `generate_traced` built a `HashSet` named
//! `_seen_set`, inserted into it, and never read it. Nothing anywhere excluded an
//! already-read work, so every recommendation surface could serve a work the
//! reader finished last week and present it as new.
//!
//! The exclusion is asserted against a real database on both engines, because the
//! failure it guards is partly a SQL-shape failure: `reading_history_entry` and
//! `reading_progress` are polymorphic (`subject_type` + `subject_id`), so querying
//! them as if they carried `work_id` returns nothing and the exclusion silently
//! applies to ratings alone.

use lorehaven_db::rec_strategy::{RecContext, RecRegistry};
use lorehaven_db::Database;
use std::sync::Arc;
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
///
/// Copied from `hit_rate.rs` rather than shared, for the reason documented there:
/// the db crate cannot depend on `test_support`, and the PostgreSQL URL's database
/// must be this call's own — reusing the shared `postgres` database makes every
/// later run fail the append-only migration checksum guard.
async fn connect(tag: &str) -> Database {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "lorehaven-seen-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let url = match std::env::var("LOREHAVEN_TEST_PG_URL") {
        Ok(admin) => {
            let name = format!(
                "lh_seen_{}_{}",
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

/// Insert with `?N#u` marking a uuid slot, `#t` a timestamp, `#i` a bigint.
async fn insert(db: &Database, sqlite: &str, args: &[&str]) {
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            let sqlite = &sqlite
                .replace("#u", "")
                .replace("#t", "")
                .replace("#i", "")
                .replace("::timestamptz", "")
                .replace(", false, ", ", 0, ");
            let mut q = sqlx::query(sqlite);
            for a in args {
                q = q.bind(*a);
            }
            q.execute(db.sqlite_pool().expect("sqlite pool"))
                .await
                .expect("insert");
        }
        lorehaven_db::Backend::Postgres => {
            let postgres = (1..=5).fold(sqlite.to_string(), |acc, n| {
                acc.replace(&format!("?{n}#u"), &format!("${n}::uuid"))
                    .replace(&format!("?{n}#t"), &format!("${n}::timestamptz"))
                    .replace(&format!("?{n}#i"), &format!("${n}::bigint"))
                    .replace(&format!("?{n}"), &format!("${n}"))
            });
            let mut q = sqlx::query(&postgres);
            for a in args {
                match uuid::Uuid::parse_str(a) {
                    Ok(u) => q = q.bind(u),
                    Err(_) => match a.parse::<i64>() {
                        Ok(n) => q = q.bind(n),
                        Err(_) => q = q.bind(*a),
                    },
                };
            }
            q.execute(db.postgres_pool().expect("postgres pool"))
                .await
                .expect("insert");
        }
    }
}

struct Reader {
    db: Database,
    account: String,
    pseud: String,
    works: Vec<String>,
}

impl Reader {
    /// A reader with an account, a pseud, and `n` published works.
    async fn build(tag: &str, works: usize) -> Self {
        let db = connect(tag).await;
        let run = uuid::Uuid::new_v4().to_string();
        let account = uuid::Uuid::new_v4().to_string();
        let pseud = uuid::Uuid::new_v4().to_string();

        insert(
            &db,
            "INSERT INTO accounts (id, email, created_at, updated_at) \
             VALUES (?1#u, ?2, '2026-01-01T00:00:00Z'::timestamptz, '2026-01-01T00:00:00Z')",
            &[&account, &format!("{tag}-{run}@example.com")],
        )
        .await;
        insert(
            &db,
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
             VALUES (?1#u, ?2#u, ?3, ?3, '2026-01-01T00:00:00Z'::timestamptz, '2026-01-01T00:00:00Z')",
            &[&pseud, &account, &format!("p{tag}-{run}")],
        )
        .await;

        let mut ids = Vec::with_capacity(works);
        for i in 0..works {
            let w = uuid::Uuid::new_v4().to_string();
            insert(
                &db,
                "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at, \
                         generated_content_posture) \
                 VALUES (?1#u, ?2#u, ?3, '2026-01-01T00:00:00Z'::timestamptz, \
                 '2026-01-01T00:00:00Z', 'forbid')",
                &[&w, &pseud, &format!("W{i}")],
            )
            .await;
            ids.push(w);
        }

        Self {
            db,
            account,
            pseud,
            works: ids,
        }
    }

    /// Record a read of `work`, the way the app's reader does.
    async fn read(&self, work: &str) {
        insert(
            &self.db,
            "INSERT INTO reading_history_entry (id, account_id, pseud_id, subject_type, \
                     subject_id, last_read_at, created_at) \
             VALUES (?1#u, ?2#u, ?3#u, 'work', ?4#u, \
             '2026-01-01T00:00:00Z'::timestamptz, '2026-01-01T00:00:00Z'::timestamptz)",
            &[
                &uuid::Uuid::new_v4().to_string(),
                &self.account,
                &self.pseud,
                work,
            ],
        )
        .await;
    }

    /// Record a *started* read, which lives in the polymorphic progress table.
    async fn start(&self, work: &str) {
        insert(
            &self.db,
            "INSERT INTO reading_progress (id, account_id, subject_type, subject_id, \
                     position_permille, created_at, updated_at) \
             VALUES (?1#u, ?2#u, 'work', ?3#u, 100, \
             '2026-01-01T00:00:00Z'::timestamptz, '2026-01-01T00:00:00Z'::timestamptz)",
            &[&uuid::Uuid::new_v4().to_string(), &self.account, work],
        )
        .await;
    }
}

/// A registry with one strategy that always returns `ids`, so the blend's
/// behaviour is isolated from the strategies themselves.
fn registry_returning(ids: Vec<String>) -> RecRegistry {
    let mut reg = RecRegistry::new(60.0);
    reg.register(
        "fixed",
        Arc::new(move |_db, _ctx| {
            let ids = ids.clone();
            Box::pin(async move { Ok(ids) })
        }),
    );
    reg
}

// ── the tests ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_work_in_the_seen_set_is_excluded_from_the_blend() {
    let r = Reader::build("exclude", 2).await;
    let reg = registry_returning(r.works.clone());

    let ids = reg
        .generate(
            &r.db,
            RecContext {
                account_id: r.account.clone(),
                seen: vec![r.works[0].clone()],
                cap: 10,
            },
        )
        .await
        .unwrap();

    assert_eq!(
        ids,
        vec![r.works[1].clone()],
        "the seen work must be dropped and the other kept"
    );
}

#[tokio::test]
async fn a_read_work_is_excluded() {
    let r = Reader::build("read", 2).await;
    r.read(&r.works[0]).await;

    let seen = lorehaven_db::reading::seen_work_ids(&r.db, &r.account)
        .await
        .unwrap();
    assert_eq!(
        seen,
        vec![r.works[0].clone()],
        "a read must be reported as seen, from the polymorphic history table"
    );
}

#[tokio::test]
async fn a_started_read_is_excluded() {
    let r = Reader::build("start", 2).await;
    r.start(&r.works[0]).await;

    let seen = lorehaven_db::reading::seen_work_ids(&r.db, &r.account)
        .await
        .unwrap();
    assert_eq!(
        seen,
        vec![r.works[0].clone()],
        "a started read lives in reading_progress, not the history table"
    );
}

/// The two tables are different tables. An exclusion that only reads one of them
/// is the exact bug this file guards, and the two tests above would both pass if
/// the union silently returned a single table's rows.
#[tokio::test]
async fn seen_covers_both_the_history_and_the_progress_tables() {
    let r = Reader::build("both", 3).await;
    r.read(&r.works[0]).await;
    r.start(&r.works[1]).await;

    let mut seen = lorehaven_db::reading::seen_work_ids(&r.db, &r.account)
        .await
        .unwrap();
    seen.sort();

    let mut expected = vec![r.works[0].clone(), r.works[1].clone()];
    expected.sort();

    assert_eq!(seen, expected, "both tables contribute to the union");
}

/// The exclusion must not renumber. If it did, dropping a seen work at rank 0
/// would promote rank 1 to rank 0 and inflate it above what any other strategy
/// would score for the same work.
///
/// Asserted on the *blend's* RRF score, not on the report's `ranked` list: that
/// list is deliberately the strategy's own unfiltered output (it is what shadow
/// mode compares strategies by, and §16.1a's contract is that the blend is
/// byte-for-byte the same computation in both modes). Reading ranks off it would
/// have tested the wrong thing and passed either way.
#[tokio::test]
async fn excluding_a_seen_work_does_not_renumber_the_others() {
    let r = Reader::build("rank", 3).await;

    // Two strategies that agree, so the scores are exactly doubled and the
    // expected value can be computed rather than eyeballed.
    let mut reg = RecRegistry::new(60.0);
    for _ in 0..2 {
        reg.register(
            "fixed",
            Arc::new({
                let ids = r.works.clone();
                move |_db, _ctx| {
                    let ids = ids.clone();
                    Box::pin(async move { Ok(ids) })
                }
            }),
        );
    }

    let report = reg
        .generate_traced(
            &r.db,
            RecContext {
                account_id: r.account.clone(),
                seen: vec![r.works[0].clone()],
                cap: 10,
            },
        )
        .await
        .unwrap();

    // The strategy's own view is untouched by the reader's history: three
    // produced, still ranked from position 0.
    let c = &report.per_strategy[0];
    assert_eq!(c.produced, 3);
    assert_eq!(c.ranked.len(), 3, "the report is a fact about the strategy");
    assert_eq!(c.ranked[0].0, r.works[0], "its rank 0 is still rank 0");

    // The blend, however, drops the seen work — and `works[1]` was ranked 1 by
    // the strategy, so if the filter renumbered it would score 1/(60+1) here
    // instead of 1/(60+2), and the two surviving works' order would depend on
    // which one the reader had already read.
    assert_eq!(report.blended.len(), 2);
    assert!(
        !report.blended.contains(&r.works[0]),
        "the seen work is dropped"
    );

    // The surviving order must be the strategy's order with the seen work
    // removed — not a re-sort. `works[1]` was rank 1, `works[2]` rank 2, so
    // `works[1]` must still lead.
    assert_eq!(
        report.blended[0], r.works[1],
        "rank 1 stays ahead of rank 2 regardless of what was filtered"
    );
}

/// `produced` counts what the strategy returned, which is a fact about the
/// strategy, not about the reader. Filtering must not rewrite it.
#[tokio::test]
async fn the_per_strategy_produced_count_is_unfiltered() {
    let r = Reader::build("produced", 2).await;
    let reg = registry_returning(r.works.clone());

    let report = reg
        .generate_traced(
            &r.db,
            RecContext {
                account_id: r.account.clone(),
                seen: vec![r.works[0].clone()],
                cap: 10,
            },
        )
        .await
        .unwrap();

    assert_eq!(
        report.per_strategy[0].produced, 2,
        "the strategy produced two works; the reader's history does not change that"
    );
    assert_eq!(
        report.blended.len(),
        1,
        "but only one survives into the blend"
    );
}
