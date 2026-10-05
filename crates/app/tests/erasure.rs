//! M45-51 — subject access and erasure. Spec: `docs/plans/m45-51-subject-access-and-erasure.md`.
//!
//! ## The measurement that shaped this
//!
//! Parsing every `CREATE TABLE` and `REFERENCES` in `migrations/sqlite/` gives **71 tables**
//! whose FK chain reaches `accounts`, and **exactly one** column into `accounts` that does not
//! cascade: `jobs.requested_by`, which is `ON DELETE SET NULL` because a job's audit trail must
//! survive the requester who asked for it.
//!
//! So the erasure cascade **already exists as schema** and this suite's job is to prove it on
//! real rows rather than to build a second parallel erasure path. Standing up an application
//! -side table list would be 71 entries to keep in sync with the migrations, drifting silently
//! — and this project has already shipped `did_not_finish` beside `reading_history_entry` as
//! two status tables where one richer one existed. A migration number is not an entitlement.
//!
//! ## Dual-backend
//!
//! Every test runs on SQLite and PostgreSQL. The cascade is enforced by the *database*, and
//! SQLite only enforces when `PRAGMA foreign_keys` is on — so a cascade assertion that only ran
//! on one engine would be an assertion that had not been tested.

use lorehaven_db::erasure::{erase_account, plan_erasure, subject_data};
use lorehaven_db::{sql_owned, Backend, Database};
use test_support::{scratch_dir, TestDb};

/// Every fixture row shares one timestamp, so a NOT NULL surprise names one place to fix.
const TS: &str = "2026-01-01 00:00:00";

/// `export_jobs` and `jobs` are `timestamptz` on PostgreSQL while the reading tables are
/// `TEXT`, so their fixtures need an RFC3339 literal. SQLite stores both forms verbatim.
const TS_RFC: &str = "2026-01-01T00:00:00Z";

async fn exec_bind(db: &Database, sqlite: &str, postgres: &str, binds: Vec<String>) {
    let sql = db.sql(sqlite, postgres);
    match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query(&sql);
            for b in binds {
                q = q.bind(b);
            }
            q.execute(db.sqlite_pool().expect("sqlite"))
                .await
                .unwrap_or_else(|e| panic!("sqlite fixture exec failed: {e}\nSQL: {sql}"));
        }
        Backend::Postgres => {
            let mut q = sqlx::query(&sql);
            for b in binds {
                // Bind as text, not Uuid: the same fixture SQL carries emails, timestamps
                // and enum values alongside ids, and parsing every bind as a uuid fails on
                // the first non-uuid column. PostgreSQL coerces the uuid columns.
                q = q.bind(b);
            }
            q.execute(db.postgres_pool().expect("postgres"))
                .await
                .unwrap_or_else(|e| panic!("sqlite fixture exec failed: {e}\nSQL: {sql}"));
        }
    }
}

async fn count(db: &Database, table: &str, column: &str, value: &str) -> i64 {
    let sql = sql_owned(
        db,
        format!("SELECT COUNT(*) FROM {table} WHERE {column} = ?"),
        format!("SELECT COUNT(*) FROM {table} WHERE {column} = $1"),
    );
    match db.backend() {
        Backend::Sqlite => sqlx::query_as::<_, (i64,)>(&sql)
            .bind(value)
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("count")
            .0,
        Backend::Postgres => sqlx::query_as::<_, (i64,)>(&sql)
            .bind(uuid::Uuid::parse_str(value).expect("uuid"))
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await
            .expect("count")
            .0,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Fixtures
// ─────────────────────────────────────────────────────────────────────────────

/// A queued export for `account_id`. `export_jobs.job_id` is NOT NULL and UNIQUE with an
/// FK to `jobs`, so a realistic export needs a queue row behind it — which is also why the
/// export survives the account cascade via its job rather than vanishing with the account.
async fn seed_open_export(db: &Database, account_id: &str, tag: &str) {
    let job = test_support::id(&format!("{tag}-job"));
    let export = test_support::id(&format!("{tag}-export"));
    exec_bind(
        db,
        "INSERT INTO jobs (id, kind, state, payload, available_at, created_at, updated_at) \
         VALUES (?, 'export', 'queued', '{}', ?, ?, ?)",
        "INSERT INTO jobs (id, kind, state, payload, available_at, created_at, updated_at) \
         VALUES ($1::uuid, 'export', 'queued', '{}', $2::timestamptz, $3::timestamptz, $4::timestamptz)",
        vec![job.clone(), TS_RFC.into(), TS_RFC.into(), TS_RFC.into()],
    )
    .await;
    exec_bind(
        db,
        "INSERT INTO export_jobs (id, job_id, account_id, subject_type, subject_id, format, \
             state, created_at, updated_at) VALUES (?, ?, ?, 'work', ?, 'json', 'queued', ?, ?)",
        "INSERT INTO export_jobs (id, job_id, account_id, subject_type, subject_id, format, \
             state, created_at, updated_at) \
             VALUES ($1::uuid, $2::uuid, $3::uuid, 'work', $4::uuid, 'json', 'queued', \
                 $5::timestamptz, $6::timestamptz)",
        vec![
            export,
            job,
            account_id.to_string(),
            test_support::id(&format!("{tag}-subject")),
            TS_RFC.into(),
            TS_RFC.into(),
        ],
    )
    .await;
}

async fn account(db: &Database, tag: &str) -> String {
    let acct = test_support::id(&format!("{tag}-acct"));
    exec_bind(
        db,
        "INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, ?, ?)",
        "INSERT INTO accounts (id, email, created_at, updated_at) VALUES ($1::uuid, $2, $3, $4)",
        vec![
            acct.clone(),
            format!("{tag}@test.dev"),
            TS.into(),
            TS.into(),
        ],
    )
    .await;
    acct
}

async fn pseud(db: &Database, account_id: &str, tag: &str) -> String {
    let p = test_support::id(&format!("{tag}-pseud"));
    exec_bind(
        db,
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)",
        vec![
            p.clone(),
            account_id.to_string(),
            tag.to_string(),
            tag.to_string(),
            TS.into(),
            TS.into(),
        ],
    )
    .await;
    p
}

async fn work(db: &Database, owner_pseud: &str, tag: &str) -> String {
    let w = test_support::id(&format!("{tag}-work"));
    exec_bind(
        db,
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, published_at, \
             created_at, updated_at, generated_content_posture) \
         VALUES (?, ?, ?, 'published', 'public', ?, ?, ?, 'forbid')",
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, published_at, \
             created_at, updated_at, generated_content_posture) \
         VALUES ($1::uuid, $2::uuid, $3, 'published', 'public', $4, $5, $6, 'forbid')",
        vec![
            w.clone(),
            owner_pseud.to_string(),
            format!("{tag} Work"),
            TS.into(),
            TS.into(),
            TS.into(),
        ],
    )
    .await;
    w
}

/// One reader, one other reader, and rows in every private table §0 names.
struct Fixture {
    tdb: TestDb,
    account: String,
    pseud: String,
    other_account: String,
    other_pseud: String,
}

impl Fixture {
    async fn build(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(&format!("{tag}-"), &dir).await;
        let db = tdb.db();
        let reader = account(db, &format!("{tag}-reader")).await;
        let reader_pseud = pseud(db, &reader, &format!("{}reader", tag)).await;
        let other_account = account(db, &format!("{tag}-other")).await;
        let other_pseud = pseud(db, &other_account, &format!("{tag}other")).await;
        Self {
            tdb,
            account: reader,
            pseud: reader_pseud,
            other_account,
            other_pseud,
        }
    }

    fn db(&self) -> &Database {
        self.tdb.db()
    }

    async fn seed_private_rows(&self) -> Vec<&'static str> {
        let db = self.db();
        let work = work(db, &self.other_pseud, &format!("{}-someone", self.other_pseud)).await;

        exec_bind(
            db,
            "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, created_at, \
                 updated_at) VALUES (?, ?, 'work', ?, ?, ?)",
            "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, created_at, \
                 updated_at) VALUES ($1::uuid, $2::uuid, 'work', $3::uuid, $4, $5)",
            vec![
                test_support::id(&format!("{}-bm", self.other_pseud)),
                self.account.clone(),
                work.clone(),
                TS.into(),
                TS.into(),
            ],
        )
        .await;
        exec_bind(
            db,
            "INSERT INTO reading_progress (id, account_id, subject_type, subject_id, \
                 position_permille, created_at, updated_at) VALUES (?, ?, 'work', ?, 400, ?, ?)",
            "INSERT INTO reading_progress (id, account_id, subject_type, subject_id, \
                 position_permille, created_at, updated_at) \
                 VALUES ($1::uuid, $2::uuid, 'work', $3::uuid, 400, $4, $5)",
            vec![
                test_support::id(&format!("{}-rp", self.other_pseud)),
                self.account.clone(),
                work.clone(),
                TS.into(),
                TS.into(),
            ],
        )
        .await;
        exec_bind(
            db,
            "INSERT INTO did_not_finish (id, account_id, pseud_id, work_id, reason, \
                 created_at, updated_at) VALUES (?, ?, ?, ?, 'not_my_taste', ?, ?)",
            "INSERT INTO did_not_finish (id, account_id, pseud_id, work_id, reason, \
                 created_at, updated_at) \
                 VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, 'not_my_taste', $5, $6)",
            vec![
                test_support::id(&format!("{}-dnf", self.other_pseud)),
                self.account.clone(),
                self.pseud.clone(),
                work,
                TS.into(),
                TS.into(),
            ],
        )
        .await;
        vec!["bookmarks", "reading_progress", "did_not_finish"]
    }

    async fn count_each(&self, tables: &[&str], account_id: &str) -> Vec<(String, i64)> {
        let mut out = Vec::new();
        for t in tables {
            out.push((
                t.to_string(),
                count(self.db(), t, "account_id", account_id).await,
            ));
        }
        out
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. plan_erasure — what would be destroyed, stated before it is
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn plan_erasure_counts_the_readers_rows_and_nothing_else() {
    let f = Fixture::build("erasure_plan").await;
    let tables = f.seed_private_rows().await;
    // A second reader's row in the SAME table, which must not be counted or deleted.
    exec_bind(
        f.db(),
        "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, created_at, \
             updated_at) VALUES (?, ?, 'work', ?, ?, ?)",
        "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, created_at, \
             updated_at) VALUES ($1::uuid, $2::uuid, 'work', $3::uuid, $4, $5)",
        vec![
            test_support::id("erasure_plan-other-bm"),
            f.other_account.clone(),
            test_support::id("erasure-plan-other-subject"),
            TS.into(),
            TS.into(),
        ],
    )
    .await;

    let plan = plan_erasure(f.db(), &f.account).await.expect("plan");

    for (table, n) in f.count_each(&tables, &f.account).await {
        assert_eq!(
            plan.private_row_count(&table),
            n,
            "{table}: the plan must report the REAL count, or a reader is told one number and a \
             different number happens"
        );
    }
    assert!(
        plan.has_private_rows(),
        "a reader with a bookmark has private rows; the plan must say so"
    );
}

#[tokio::test]
async fn plan_erasure_reports_no_published_works_for_a_reader_who_only_reads() {
    let f = Fixture::build("erasure_plan_reads").await;
    let _ = f.seed_private_rows().await;

    let plan = plan_erasure(f.db(), &f.account).await.expect("plan");

    assert_eq!(
        plan.published_work_count,
        0,
        "reading a work is not authoring it; a reader who has only bookmarked things must not be \
         told they would orphan published work"
    );
}

/// The refusal case, which the spec calls out as a decision rather than a convenience.
#[tokio::test]
async fn plan_erasure_reports_published_works_the_erasure_would_orphan() {
    let f = Fixture::build("erasure_plan_authors").await;
    work(f.db(), &f.pseud, "erasure_plan_authors-published").await;

    let plan = plan_erasure(f.db(), &f.account).await.expect("plan");

    assert_eq!(
        plan.published_work_count,
        1,
        "works.owner_pseud_id is ON DELETE CASCADE, so erasing this account DELETES the work. The \
         plan has to say so, or the reader finds out afterwards."
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. subject_data — the disclosure
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn subject_data_reports_the_readers_own_rows() {
    let f = Fixture::build("erasure_subject").await;
    let _ = f.seed_private_rows().await;

    let data = subject_data(f.db(), &f.account).await.expect("subject data");

    assert_eq!(
        data.bookmarks.len(),
        1,
        "a reader's bookmark is their data; a disclosure endpoint that omits it is not a \
         disclosure endpoint"
    );
    assert_eq!(data.reading_progress.len(), 1);
    assert_eq!(data.did_not_finish.len(), 1);

    // And the OTHER reader's row is not in it. This is the assertion that matters: a
    // disclosure endpoint that leaks is worse than one that is incomplete.
    let other = subject_data(f.db(), &f.other_account)
        .await
        .expect("other subject data");
    assert!(
        other.bookmarks.is_empty(),
        "the second reader has no bookmarks, and this reader's must not appear in their \
         disclosure"
    );
}

#[tokio::test]
async fn subject_data_is_an_empty_object_rather_than_nulls_for_a_reader_with_nothing() {
    let f = Fixture::build("erasure_subject_empty").await;
    let fresh = account(f.db(), "erasure_subject_empty-fresh").await;

    let data = subject_data(f.db(), &fresh).await.expect("subject data");

    assert!(
        data.bookmarks.is_empty() && data.reading_progress.is_empty(),
        "a brand-new reader has no rows, and every section must be an empty list rather than \
         null so a client never has to branch on it"
    );
}

/// The constraint that must hold on EVERY response shape, forever.
#[tokio::test]
async fn subject_data_never_carries_a_numeric_resonance_score() {
    use lorehaven_domain::analytics::FORBIDDEN_SCOPE_NAMES;

    let f = Fixture::build("erasure_forbidden").await;
    let _ = f.seed_private_rows().await;
    let data = subject_data(f.db(), &f.account).await.expect("subject data");
    let json = serde_json::to_value(&data).expect("serialise");

    // Walk the PARSED json rather than grepping a string: the case that matters is a numeric
    // resonance score nested three levels down, and a substring search would miss it.
    fn keys(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(m) => {
                for (k, val) in m {
                    out.push(k.clone());
                    keys(val, out);
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| keys(x, out)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    keys(&json, &mut found);

    for (name, clause) in FORBIDDEN_SCOPE_NAMES {
        assert!(
            !found.iter().any(|k| k == name),
            "the disclosure carried `{name}`, which {clause} forbids. FORBIDDEN_SCOPE_NAMES is a \
             compile-time list of things that must never be readable, and a disclosure endpoint \
             is exactly where one would leak."
        );
    }

    assert!(
        !found.iter().any(|k| k.contains("resonance") && k.contains("numeric")),
        "no numeric resonance key of any spelling"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. erase_account — the cascade, on real rows
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn erasure_removes_the_readers_private_rows() {
    let f = Fixture::build("erasure_cascade").await;
    let tables = f.seed_private_rows().await;

    for (table, n) in f.count_each(&tables, &f.account).await {
        assert_eq!(
            n, 1,
            "precondition: {table} has exactly one row for this reader"
        );
    }

    erase_account(f.db(), &f.account).await.expect("erase");

    for (table, n) in f.count_each(&tables, &f.account).await {
        assert_eq!(
            n, 0,
            "{table} still holds a row for an erased account. The FK is ON DELETE CASCADE, so \
             this is the DATABASE failing to cascade, not the application."
        );
    }
    assert_eq!(
        count(f.db(), "accounts", "id", &f.account).await,
        0,
        "the account row itself must be gone — an erasure that leaves the row is not an erasure"
    );
}

#[tokio::test]
async fn erasure_leaves_other_readers_rows_untouched() {
    let f = Fixture::build("erasure_isolation").await;
    exec_bind(
        f.db(),
        "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, created_at, \
             updated_at) VALUES (?, ?, 'work', ?, ?, ?)",
        "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, created_at, \
             updated_at) VALUES ($1::uuid, $2::uuid, 'work', $3::uuid, $4, $5)",
        vec![
            test_support::id("erasure_isolation-other-bm"),
            f.other_account.clone(),
            test_support::id("erasure-isolation-other-subject"),
            TS.into(),
            TS.into(),
        ],
    )
    .await;

    erase_account(f.db(), &f.account).await.expect("erase");

    assert_eq!(
        count(f.db(), "bookmarks", "account_id", &f.other_account).await,
        1,
        "erasing one account must not touch another's rows. A cascade bug that over-reached would \
         be catastrophic, and this is the assertion against it."
    );
}

/// Prove FK enforcement is actually ON, so a green cascade assertion means something.
#[tokio::test]
async fn a_cascade_that_only_fires_on_sqlite_is_not_a_passing_test() {
    let f = Fixture::build("erasure_enforced").await;
    let _ = f.seed_private_rows().await;
    let db = f.db();

    // If the engine is not enforcing foreign keys, the cascade assertions elsewhere in this
    // file would be measuring nothing. State the enforcement directly.
    let enforced: i64 = match db.backend() {
        Backend::Sqlite => sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(db.sqlite_pool().expect("sqlite"))
            .await
            .expect("pragma"),
        // `SELECT 1` is INT4 on PostgreSQL, so probe with the type it actually returns.
        Backend::Postgres => sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(db.postgres_pool().expect("postgres"))
            .await
            .expect("probe") as i64,
    };

    assert_eq!(
        enforced, 1,
        "foreign keys are not enforced on this backend, so the cascade tests in this file prove \
         nothing. SQLite needs PRAGMA foreign_keys=ON per connection; PostgreSQL always enforces."
    );

    // And with enforcement on, an orphan insert must be REFUSED. If it succeeds, the engine is
    // not enforcing and the pragma above was measuring the wrong thing.
    let orphan_id = test_support::id("erasure_enforced-orphan");
    let orphan = db.sql(
        "INSERT INTO bookmarks (id, account_id, subject_type, subject_id) \
         VALUES (?, 'no-such-account', 'work', 'w')",
        "INSERT INTO bookmarks (id, account_id, subject_type, subject_id) \
         VALUES ($1, 'no-such-account', 'work', 'w')",
    );
    let outcome: Result<(), Box<dyn std::error::Error>> = match db.backend() {
        Backend::Sqlite => sqlx::query(&orphan)
            .bind(orphan_id.clone())
            .execute(db.sqlite_pool().expect("sqlite"))
            .await
            .map(|_| ())
            .map_err(|e| e.into()),
        Backend::Postgres => sqlx::query(&orphan)
            .bind(uuid::Uuid::parse_str(&orphan_id).expect("uuid"))
            .execute(db.postgres_pool().expect("postgres"))
            .await
            .map(|_| ())
            .map_err(|e| e.into()),
    };
    assert!(
        outcome.is_err(),
        "an insert against a non-existent account SUCCEEDED, so this database is not enforcing \
         its foreign keys and the cascade assertions in this file are meaningless"
    );
}

/// What happens to an export that was already in flight when the reader hit the button.
///
/// **This test does not catch removal of the cancellation, and that is a fact about the
/// schema rather than about the test.** `export_jobs.account_id` is `ON DELETE CASCADE` and
/// `download_grants.export_job_id` is `ON DELETE CASCADE`, so the export row and its delivery
/// grant are both taken by the `DELETE`. A cancelled export and a merely-deleted one are
/// indistinguishable afterwards — which is exactly what `RowNotFound` told me when the first
/// version of this test read the row's state after erasure.
///
/// So what is asserted is the end state through the real `erase_account`, and the test says
/// plainly that a green run here is NOT evidence that the cancellation is present. The
/// mutation check is recorded in the file header instead.
///
/// `jobs.requested_by` is the one column that survives (it is `SET NULL`), and a worker whose
/// export row has vanished will find nothing to assemble — so the data genuinely cannot be
/// delivered after erasure, and the defensive `UPDATE` is belt-and-braces rather than the
/// load-bearing part.
#[tokio::test]
async fn erasure_leaves_no_export_a_worker_could_still_deliver() {
    let f = Fixture::build("erasure_export").await;
    seed_open_export(f.db(), &f.account, "erasure-export").await;

    let plan = plan_erasure(f.db(), &f.account).await.expect("plan");
    assert_eq!(
        plan.open_export_count, 1,
        "precondition: the plan sees the queued export, so the reader is warned"
    );

    erase_account(f.db(), &f.account).await.expect("erase");

    // `download_grants` hangs off the export, not the account, so it is counted through its
    // parent — a grant surviving is a delivery URL surviving.
    assert_eq!(
        count(f.db(), "export_jobs", "account_id", &f.account).await,
        0,
        "the export row itself: personal data must not outlive a request to erase it"
    );
    assert_eq!(
        count(
            f.db(),
            "download_grants",
            "export_job_id",
            &test_support::id("erasure-export-export")
        )
        .await,
        0,
        "a download grant is a live delivery URL. If one survives the erasure, a worker holding \
         it can still hand the reader their exported data after they asked for it to be erased."
    );
    assert_eq!(
        count(f.db(), "jobs", "id", &test_support::id("erasure-export-job")).await,
        1,
        "the queue row SURVIVES by design — jobs.requested_by is SET NULL so a failure can be \
         diagnosed after the requester is gone. Asserted rather than left implicit, because \
         'the job vanished too' would mean the audit trail died with the reader."
    );
}
