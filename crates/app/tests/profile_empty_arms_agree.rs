//! The SQLite arm of `surprise_me_work` must agree with the PostgreSQL arm about whether the
//! reader's taste profile is EMPTY.
//!
//! This exists because the two arms once disagreed and the whole SQLite suite stayed green.
//! `profile_empty` counted taste-profile ROWS on SQLite and taste-profile SIGNALS on
//! PostgreSQL. Both answer "is the profile empty?" and they differ for a reader whose row
//! exists with `signals = []` — which is every brand-new account, since the row is created
//! before any signal is written.
//!
//! Three things hid it, which is why this is a test of its own rather than one more
//! assertion inside the surprise-me suite:
//!
//!   1. The seeded fixture had signals, so a row count and a signal count agreed anyway.
//!   2. Only one test read `profile_empty`, and it was the cold-start case with no row at
//!      all — where both implementations correctly say "empty".
//!   3. SQLite is the default engine and its arm was the wrong one, so the default run was
//!      the run structurally unable to see the difference.
//!
//! The three shapes below are the ones that separate the implementations. The empty-signals
//! row is the only one that does, so it is asserted directly and then swept as a table.
//!
//! SCOPE, verified in both directions: reintroducing the row-count bug makes this file fail
//! 2 of 4 on SQLite and pass 4 of 4 on PostgreSQL. That asymmetry is the point rather than a
//! weakness — the PostgreSQL arm was always the correct one, so the bug is a SQLite-only
//! defect and a SQLite-only test is exactly what catches it. It does mean a green run on
//! PostgreSQL proves nothing about this bug, which is why the header says so rather than
//! letting "4 passed" read as coverage.
//!
//! Every assertion goes through `surprise_me_work` and reads `profile_empty` off the
//! returned candidate. The first version of this file instead RE-IMPLEMENTED both engine
//! arms inline and asserted that its own copies agreed — and so it passed, four tests green,
//! against the very code it was written for. Reverting the SQLite arm to counting rows
//! changed nothing it could see. The comment is here because that failure mode is invisible:
//! the test looks rigorous, runs on a real database, and is blind.

use lorehaven_db::discovery::surprise_me_work;
use lorehaven_db::Backend;
use test_support::{scratch_dir, TestDb};

struct Fixture {
    tdb: TestDb,
    reader: String,
}

impl Fixture {
    async fn build(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let reader = create_account(&tdb, &format!("{tag}-reader@example.com")).await;
        // One published, public work so `surprise_me_work` has something to return and
        // `profile_empty` appears on the candidate at all. Without it the query yields
        // `Ok(None)` and there is no flag to assert on.
        let author = create_account(&tdb, &format!("{tag}-author@example.com")).await;
        create_published_work(&tdb, &author, &format!("{tag}-work")).await;
        Self { tdb, reader }
    }

    /// Seed the profile row with the given signals. `[]` is the shape this file is about: a
    /// row that EXISTS and holds nothing.
    async fn profile_with_signals(&self, signals: &str) {
        let signals = signals.replace('\'', "''");
        // `taste_profiles(account, signals, computed_at)` — 0012_discovery.sql:1. NOT NULL on
        // all three, so the timestamp is required rather than defaulted.
        let sql = self.tdb.sql(&format!(
            "INSERT INTO taste_profiles (account, signals, computed_at) \
             VALUES (?, '{signals}', '2026-01-01T00:00:00Z')"
        ));
        // The two pool types differ, so neither arm can be the match's value. Discard the
        // result and keep the side effect; the `.expect` stays inside each arm.
        match self.tdb.db().backend() {
            Backend::Sqlite => {
                sqlx::query(&sql)
                    .bind(&self.reader)
                    .execute(self.tdb.db().sqlite_pool().expect("sqlite"))
                    .await
                    .expect("insert taste profile");
            }
            Backend::Postgres => {
                sqlx::query(&sql)
                    .bind(&self.reader)
                    .execute(self.tdb.db().postgres_pool().expect("postgres"))
                    .await
                    .expect("insert taste profile");
            }
        }
    }

    /// The flag as PRODUCTION computes it.
    ///
    /// This calls `surprise_me_work` rather than re-implementing either arm, because a test
    /// that duplicates the thing it checks cannot fail when the thing is wrong — and this
    /// one did exactly that, passing against the broken code for four green tests.
    async fn profile_empty(&self) -> bool {
        let pick = surprise_me_work(self.tdb.db(), &self.reader, "2026-01-01")
            .await
            .expect("surprise_me_work");
        assert!(
            pick.candidate.is_some(),
            "a candidate: the fixture publishes one work, so the query must return one"
        );
        pick.profile_empty
    }

    /// The row count — what the broken SQLite arm used. Kept so the discriminating
    /// precondition can be stated on every run rather than assumed.
    async fn row_count(&self) -> i64 {
        self.tdb
            .count_by(
                "SELECT COUNT(*) FROM taste_profiles WHERE account = ?",
                &self.reader,
            )
            .await
    }
}

#[tokio::test]
async fn a_profile_row_holding_no_signals_counts_as_empty() {
    let fx = Fixture::build("empty-signals").await;
    fx.profile_with_signals("[]").await;

    // The precondition, STATED rather than assumed. If the fixture stopped creating the
    // row this test becomes vacuous — it would pass on a database where no row exists,
    // which is the case where both implementations are trivially correct.
    let rows = fx.row_count().await;
    assert_eq!(
        rows, 1,
        "precondition: the fixture must create the profile ROW, because this is the shape \
         that separates the two implementations. rows={rows}"
    );

    // The discriminating assertion, run through the production query. The row exists, so a
    // ROW count says "not empty"; the profile holds no signals, so a SIGNAL count says
    // "empty". The flag must follow the signals.
    assert!(
        fx.profile_empty().await,
        "a taste_profiles row holding `signals = []` IS an empty profile. Counting ROWS makes \
         this false, so a reader who has never expressed a preference is told the surface \
         inverted their taste on purpose."
    );
}

#[tokio::test]
async fn a_profile_holding_signals_counts_as_not_empty() {
    let fx = Fixture::build("populated-signals").await;
    fx.profile_with_signals(r#"["cozy-mystery","slow-burn"]"#)
        .await;

    assert!(
        !fx.profile_empty().await,
        "a profile holding two signals is NOT empty. This is the control for the test above: \
         a flag that always answered `true` would pass that one and fail this."
    );
}

#[tokio::test]
async fn a_reader_with_no_profile_row_counts_as_empty() {
    let fx = Fixture::build("no-row").await;

    assert_eq!(
        fx.row_count().await,
        0,
        "precondition: no profile row exists"
    );
    assert!(
        fx.profile_empty().await,
        "a reader with no profile row is the definition of an empty profile. This is the \
         shape the existing cold-start test already covered, which is why it never caught \
         the bug: both implementations are trivially correct here."
    );
}

#[tokio::test]
async fn all_three_profile_shapes_agree_between_the_two_arms() {
    // Named so a divergence says WHICH shape disagreed.
    let cases: [(&str, &str, &str, bool); 3] = [
        ("no-row", "shape-no-row", "", true),
        ("row-empty-signals", "shape-empty-signals", "[]", true),
        (
            "row-with-signals",
            "shape-with-signals",
            r#"["cozy-mystery"]"#,
            false,
        ),
    ];

    for (tag, label, signals, expected) in cases {
        let fx = Fixture::build(tag).await;
        if !signals.is_empty() {
            fx.profile_with_signals(signals).await;
        }

        assert_eq!(
            fx.profile_empty().await,
            expected,
            "{label} (signals = {signals:?}): profile_empty disagreed with the profile. The \
             two engine arms must return the same flag for all three shapes; a divergence \
             here means one arm is counting rows where the other counts signals."
        );
    }
}

// --- helpers ---

async fn create_account(tdb: &TestDb, email: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let sql = tdb.sql(
        "INSERT INTO accounts (id, email, created_at, updated_at) \
         VALUES (?, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    );
    match tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(email)
                .execute(tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("insert account");
        }
        Backend::Postgres => {
            // `accounts.id` is uuid on PostgreSQL, so the id binds as a uuid there and as
            // text on SQLite. Same asymmetry as `taste_profiles.account`, opposite type.
            sqlx::query(&sql)
                .bind(uuid::Uuid::parse_str(&id).expect("uuid"))
                .bind(email)
                .execute(tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("insert account");
        }
    }
    id
}

/// One published, public work, so `surprise_me_work` returns a candidate carrying the flag.
///
/// `works.owner_pseud_id` references a pseud and the pseud references the account, so both
/// rows are needed. The pseud reuses the work's id value because nothing here needs them to
/// differ, and one fewer uuid to thread is one fewer thing to get wrong.
async fn create_published_work(tdb: &TestDb, author: &str, title: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();

    let pseud_sql = tdb.sql(
        // `pseudos` NOT NULL columns without a DEFAULT: account_id, handle, display_name,
        // created_at, updated_at (0001_identity.sql:72). Read off the migration, because
        // discovering them one `NOT NULL constraint failed` at a time costs a round-trip
        // each and is how the first draft of this fixture got two wrong.
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
         VALUES (?, ?, 'fixture-handle', 'Fixture', \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    );
    match tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(&pseud_sql)
                .bind(&id)
                .bind(author)
                .execute(tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("insert pseud");
        }
        Backend::Postgres => {
            sqlx::query(&pseud_sql)
                .bind(uuid::Uuid::parse_str(&id).expect("uuid"))
                .bind(uuid::Uuid::parse_str(author).expect("uuid"))
                .execute(tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("insert pseud");
        }
    }

    let sql = tdb.sql(
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, \
             published_at, created_at, updated_at, generated_content_posture) \
         VALUES (?, ?, ?, 'published', 'public', '2026-01-01T00:00:00Z', \
             '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'forbid')",
    );
    match tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(&id)
                .bind(title)
                .execute(tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("insert work");
        }
        Backend::Postgres => {
            let uid = uuid::Uuid::parse_str(&id).expect("uuid");
            sqlx::query(&sql)
                .bind(uid)
                .bind(uid)
                .bind(title)
                .execute(tdb.db().postgres_pool().expect("postgres"))
                .await
                .expect("insert work");
        }
    }
    id
}
