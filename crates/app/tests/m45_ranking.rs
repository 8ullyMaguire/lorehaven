//! Spec §47 — the ranking substrate, integration level (M45-10, -11, -13, -15, -49).
//!
//! Everything here runs on both engines. `rank_works` and its helpers are pure
//! given a reader's weights, so most of these are unit tests in
//! `crates/db/src/ranking.rs`; what this file adds is the part a unit test cannot
//! reach — that the propensity survives the round trip into the database, that
//! the constraint rejects what it should, and that two calls on the same
//! database state agree.
//!
//! The determinism case is the one worth having at this level. §47.9 requires it
//! and it is the property that would break first if a future change moved the
//! candidate query into `rank_works`.

use lorehaven_db::ranking::{
    rank_works, ranked_propensity, scout_value, InteractionKind, RankOptions, SlotKind,
};
use lorehaven_domain::WorkId;
use test_support::{scratch_dir, TestDb, TEST_PASSWORD};

/// One test's database.
///
/// `TestDb::connect_with_dir` rather than a hand-written config: a config naming
/// a SQLite URL while the pool is on PostgreSQL builds an app that reads a
/// database nobody migrated, so every assertion passes locally and fails under
/// PostgreSQL.
struct Harness {
    _dir: std::path::PathBuf,
    tdb: TestDb,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        Self { _dir: dir, tdb }
    }

    fn db(&self) -> &lorehaven_db::Database {
        self.tdb.db()
    }

    /// The stored interaction kind for one kudos row.
    ///
    /// A method rather than a local closure because a closure returning two
    /// different `QueryScalar` future types across a dialect branch does not
    /// type-check: the branches are different concrete types and the compiler
    /// will not unify them behind an `async move` block.
    async fn kudos_kind(&self, work: &WorkId, account: &str) -> sqlx::Result<String> {
        if self.tdb.is_postgres() {
            sqlx::query_scalar(
                "SELECT kind FROM work_kudos WHERE work_id = $1::uuid AND account_id = $2::uuid",
            )
            .bind(work.to_string())
            .bind(account)
            .fetch_one(self.tdb.db().postgres_pool().expect("pg"))
            .await
        } else {
            sqlx::query_scalar("SELECT kind FROM work_kudos WHERE work_id = ? AND account_id = ?")
                .bind(work.to_string())
                .bind(account)
                .fetch_one(self.tdb.db().sqlite_pool().expect("sqlite"))
                .await
        }
    }

    /// The obscurity recorded at read time for one kudos row.
    async fn kudos_obscurity(&self, work: &WorkId, account: &str) -> sqlx::Result<f64> {
        if self.tdb.is_postgres() {
            sqlx::query_scalar(
                "SELECT obscurity_at_read FROM work_kudos \
                 WHERE work_id = $1::uuid AND account_id = $2::uuid",
            )
            .bind(work.to_string())
            .bind(account)
            .fetch_one(self.tdb.db().postgres_pool().expect("pg"))
            .await
        } else {
            sqlx::query_scalar(
                "SELECT obscurity_at_read FROM work_kudos WHERE work_id = ? AND account_id = ?",
            )
            .bind(work.to_string())
            .bind(account)
            .fetch_one(self.tdb.db().sqlite_pool().expect("sqlite"))
            .await
        }
    }

    /// An account and a pseud to hang works off.
    ///
    /// `works.owner_pseud_id` is NOT NULL and references `pseuds`, which in turn
    /// references `accounts`, so a work cannot be created without the whole chain.
    /// Seeded once per Harness and reused.
    async fn ensure_owner(&self) -> String {
        let now = "2026-01-01T00:00:00Z".to_owned();
        if self.tdb.is_postgres() {
            let pool = self.tdb.db().postgres_pool().expect("pg");
            sqlx::query(
                "INSERT INTO accounts (id, status, email, created_at, updated_at) \
                 SELECT gen_random_uuid(), 'active', gen_random_uuid()::text || '@t.test', \
                        now()::text, now()::text \
                 WHERE NOT EXISTS (SELECT 1 FROM accounts)",
            )
            .execute(pool)
            .await
            .expect("seed account (pg)");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
                 SELECT gen_random_uuid(), (SELECT id FROM accounts LIMIT 1), \
                        'o' || substr(gen_random_uuid()::text, 1, 8), 'Owner', \
                        now()::text, now()::text \
                 WHERE NOT EXISTS (SELECT 1 FROM pseuds WHERE handle LIKE 'o%')",
            )
            .execute(pool)
            .await
            .expect("seed pseud (pg)");
            sqlx::query_scalar::<_, String>(
                "SELECT id::text FROM pseuds WHERE handle LIKE 'o%' LIMIT 1",
            )
            .fetch_one(pool)
            .await
            .expect("owner pseud (pg)")
        } else {
            let pool = self.tdb.db().sqlite_pool().expect("sqlite");
            sqlx::query(
                "INSERT INTO accounts (id, status, email, created_at, updated_at) \
                 SELECT lower(hex(randomblob(16))), 'active', \
                        lower(hex(randomblob(16))) || '@t.test', ?, ? \
                 WHERE NOT EXISTS (SELECT 1 FROM accounts)",
            )
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .expect("seed account (sqlite)");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
                 SELECT lower(hex(randomblob(16))), (SELECT id FROM accounts LIMIT 1), \
                        'o' || substr(lower(hex(randomblob(16))), 1, 8), 'Owner', ?, ? \
                 WHERE NOT EXISTS (SELECT 1 FROM pseuds WHERE handle LIKE 'o%')",
            )
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .expect("seed pseud (sqlite)");
            sqlx::query_scalar::<_, String>("SELECT id FROM pseuds WHERE handle LIKE 'o%' LIMIT 1")
                .fetch_one(pool)
                .await
                .expect("owner pseud (sqlite)")
        }
    }

    /// A work with an id we control, so assertions can name it.
    async fn work(&self, n: u32) -> WorkId {
        let id = WorkId::from_uuid(uuid::Uuid::from_u128(u128::from(n)));
        let owner = self.ensure_owner().await;
        let sqlite = "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at) \
                      VALUES (?, ?, ?, ?, ?)";
        // `$1::uuid` is required and `sql_owned` does not add it: it renumbers
        // the placeholders, nothing else. `works.id` is a UUID column on
        // PostgreSQL, so a bare `$1` bound as text is error 42804.
        let postgres = "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at) \
            VALUES ($1::uuid, $2::uuid, $3, $4, $5)";
        let now = "2026-01-01T00:00:00Z";
        let sql = if self.tdb.is_postgres() {
            postgres
        } else {
            sqlite
        };
        // Branched rather than unified into one `pool` binding: the two accessors
        // return different types (`&PgPool` and `&SqlitePool`) and an `if` that
        // assigns one or the other does not compile.
        if self.tdb.is_postgres() {
            sqlx::query(sql)
                .bind(id.to_string())
                .bind(&owner)
                .bind(format!("work {n}"))
                .bind(now)
                .bind(now)
                .execute(self.tdb.db().postgres_pool().expect("pg"))
                .await
                .expect("insert work (pg)");
        } else {
            sqlx::query(sql)
                .bind(id.to_string())
                .bind(&owner)
                .bind(format!("work {n}"))
                .bind(now)
                .bind(now)
                .execute(self.tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("insert work (sqlite)");
        }
        id
    }
}

fn dims(
    map: &'static [(&'static str, &'static [&'static str])],
) -> impl Fn(&WorkId) -> Vec<String> {
    let owned: Vec<(WorkId, Vec<String>)> = map
        .iter()
        .enumerate()
        .map(|(i, (_, tags))| {
            (
                WorkId::from_uuid(uuid::Uuid::from_u128(i as u128 + 1)),
                tags.iter().map(|t| (*t).to_owned()).collect(),
            )
        })
        .collect();
    move |id: &WorkId| {
        owned
            .iter()
            .find(|(candidate, _)| candidate == id)
            .map(|(_, tags)| tags.clone())
            .unwrap_or_default()
    }
}

/// A reader whose arena weights make `angst` strongly preferred.
///
/// The account is created first: `arena_weights.account_id` is a foreign key, so
/// seeding weights for an account that does not exist is a constraint violation
/// rather than a silent no-op.
async fn seed_weights(h: &Harness, account: &str) {
    let now = "2026-01-01T00:00:00Z".to_owned();
    if h.tdb.is_postgres() {
        sqlx::query(
            "INSERT INTO accounts (id, status, email, created_at, updated_at) \
             VALUES ($1::uuid, 'active', $2, now(), now()) ON CONFLICT (id) DO NOTHING",
        )
        .bind(account)
        .bind(format!("{account}@t.test"))
        .execute(h.tdb.db().postgres_pool().expect("pg"))
        .await
        .expect("seed account (pg)");
    } else {
        sqlx::query(
            "INSERT INTO accounts (id, status, email, created_at, updated_at) \
             VALUES (?, 'active', ?, ?, ?) ON CONFLICT (id) DO NOTHING",
        )
        .bind(account)
        .bind(format!("{account}@t.test"))
        .bind(&now)
        .bind(&now)
        .execute(h.tdb.db().sqlite_pool().expect("sqlite"))
        .await
        .expect("seed account (sqlite)");
    }
    for (key, weight) in [("angst", 0.9_f64), ("fluff", 0.2_f64)] {
        // Branches are monomorphised over the pool type rather than unified into
        // one `pool` binding: `postgres_pool()` and `sqlite_pool()` return
        // different types, so an `if` assigning one or the other does not compile.
        if h.tdb.is_postgres() {
            sqlx::query(
                // `arena_weights.created_at` is TIMESTAMPTZ on PostgreSQL and
                // `account_id` is UUID (migrations/postgres/0066_taste_arena.sql:26).
                // `now()` rather than a bound string, and the explicit `::uuid`.
                "INSERT INTO arena_weights (account_id, dimension_key, weight, elo_rating, \
                 matches_played, created_at) \
                 SELECT $1::uuid, $2, $3, 0, 0, now() WHERE NOT EXISTS \
                 (SELECT 1 FROM arena_weights WHERE account_id = $1::uuid AND dimension_key = $2)",
            )
            .bind(account)
            .bind(key)
            .bind(weight)
            .execute(h.tdb.db().postgres_pool().expect("pg"))
            .await
            .expect("seed arena weight (pg)");
        } else {
            sqlx::query(
                "INSERT INTO arena_weights (account_id, dimension_key, weight, elo_rating, \
                 matches_played, created_at) \
                 SELECT ?, ?, ?, 0, 0, ? WHERE NOT EXISTS \
                 (SELECT 1 FROM arena_weights WHERE account_id = ? AND dimension_key = ?)",
            )
            .bind(account)
            .bind(key)
            .bind(weight)
            .bind(&now)
            .execute(h.tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("seed arena weight (sqlite)");
        }
    }
}

// ---------------------------------------------------------------------------
// §47.9 determinism
// ---------------------------------------------------------------------------

/// Two calls on the same database state return byte-identical ordering.
///
/// The candidate set is deliberately passed in shuffled order: §47.2 sorts by id
/// before scoring, and this is what proves it. Without the sort a candidate that
/// happened to arrive second would win a tie against the same candidate arriving
/// first — passing in isolation, differing between calls.
#[tokio::test]
async fn two_calls_on_the_same_state_agree_exactly() {
    let h = Harness::new("rank_determinism").await;
    let account = "33333333-3333-3333-3333-333333333333";
    let candidates: Vec<WorkId> = vec![
        WorkId::from_uuid(uuid::Uuid::from_u128(1)),
        WorkId::from_uuid(uuid::Uuid::from_u128(2)),
        WorkId::from_uuid(uuid::Uuid::from_u128(3)),
        WorkId::from_uuid(uuid::Uuid::from_u128(4)),
    ];
    let options = RankOptions::default();
    let by_tag = dims(&[
        ("", &["angst"]),
        ("", &["angst"]),
        ("", &["fluff"]),
        ("", &["fluff"]),
    ]);

    let first = rank_works(h.db(), account, candidates.clone(), &by_tag, &options)
        .await
        .expect("first rank");
    let reversed: Vec<WorkId> = candidates.iter().rev().copied().collect();
    let second = rank_works(h.db(), account, reversed, &by_tag, &options)
        .await
        .expect("second rank");

    assert_eq!(
        first.ranked, second.ranked,
        "the same candidates in a different arrival order produced a different ranking"
    );
}

/// The reader's own weights decide the order.
///
/// The other tests pass a `dimensions_for` map and no weights, so every candidate
/// scores 0 and the ordering is only deterministic, not *meaningful*. This one
/// seeds a real arena weight and asserts the preference actually wins — the
/// property §47.2's "scored" stage exists to provide.
#[tokio::test]
async fn the_readers_weights_decide_the_order() {
    let h = Harness::new("rank_weights").await;
    let account = "88888888-8888-8888-8888-888888888888";
    seed_weights(&h, account).await;

    let by_tag = dims(&[("angst", &["angst"]), ("fluff", &["fluff"])]);
    let candidates = vec![
        WorkId::from_uuid(uuid::Uuid::from_u128(1)),
        WorkId::from_uuid(uuid::Uuid::from_u128(2)),
    ];

    let outcome = rank_works(
        h.db(),
        account,
        candidates.clone(),
        &by_tag,
        &RankOptions::default(),
    )
    .await
    .expect("rank with weights");

    assert_eq!(
        outcome.ranked.first().map(|r| r.work_id),
        Some(candidates[0]),
        "angst scores 0.9 against the reader's weight and fluff 0.2, so the \
         angst work should lead; got {:?}",
        outcome
            .ranked
            .iter()
            .map(|r| (r.work_id, r.score))
            .collect::<Vec<_>>()
    );
    assert!(
        outcome.ranked[0].score > outcome.ranked[1].score,
        "the leading row scored no higher than the one after it"
    );
}

/// An uncalibrated reader still gets a stable, reproducible order.
///
/// §47.2's determinism requirement has to hold for the reader who never entered
/// the arena — an empty weight set is a legitimate state, not an error — so this
/// asserts two calls agree rather than asserting anything about taste.
#[tokio::test]
async fn an_uncalibrated_reader_still_gets_a_reproducible_order() {
    let h = Harness::new("rank_uncalibrated").await;
    let stranger = "99999999-9999-9999-9999-999999999999";
    let by_tag = dims(&[
        ("angst", &["angst"]),
        ("fluff", &["fluff"]),
        ("poetry", &["poetry"]),
    ]);
    let candidates: Vec<WorkId> = (1..=3)
        .map(|n| WorkId::from_uuid(uuid::Uuid::from_u128(n)))
        .collect();

    let first = rank_works(
        h.db(),
        stranger,
        candidates.clone(),
        &by_tag,
        &RankOptions::default(),
    )
    .await
    .expect("first");
    let second = rank_works(
        h.db(),
        stranger,
        candidates,
        &by_tag,
        &RankOptions::default(),
    )
    .await
    .expect("second");

    assert_eq!(first.ranked, second.ranked);
    assert!(first.ranked.iter().all(|r| r.score == 0.0));
}

/// An empty candidate set is an empty result, not an error.
#[tokio::test]
async fn no_candidates_is_not_an_error() {
    let h = Harness::new("rank_empty").await;
    let outcome = rank_works(
        h.db(),
        "44444444-4444-4444-4444-444444444444",
        Vec::new(),
        &|_| Vec::new(),
        &RankOptions::default(),
    )
    .await
    .expect("empty rank");
    assert!(outcome.ranked.is_empty());
    assert!(outcome.all_impressions().is_empty());
}

// ---------------------------------------------------------------------------
// §47.8 the propensity invariant
// ---------------------------------------------------------------------------

/// Every ranked row carries an impression with a positive propensity, and the
/// propensities sum to one.
///
/// This is the invariant that makes M45-13's offline evaluation possible at all.
/// Asserted at the integration level because the failure it guards against is a
/// *database* failure — a propensity that gets lost, coerced to zero, or never
/// computed on one engine.
#[tokio::test]
async fn every_ranked_row_has_a_positive_propensity() {
    let h = Harness::new("rank_propensity").await;
    let account = "55555555-5555-5555-5555-555555555555";
    let candidates: Vec<WorkId> = (1..=5)
        .map(|n| WorkId::from_uuid(uuid::Uuid::from_u128(n)))
        .collect();
    let by_tag = dims(&[
        ("", &["angst"]),
        ("", &["angst"]),
        ("", &["fluff"]),
        ("", &["fluff"]),
        ("", &["poetry"]),
    ]);

    let outcome = rank_works(
        h.db(),
        account,
        candidates,
        &by_tag,
        &RankOptions::default(),
    )
    .await
    .expect("rank");

    assert_eq!(outcome.ranked.len(), 5, "a candidate was dropped");
    let impressions = outcome.impressions.clone();
    assert_eq!(
        impressions.len(),
        outcome.ranked.len(),
        "not every ranked row has an impression"
    );
    for impression in &impressions {
        assert!(
            impression.propensity > 0.0 && impression.propensity <= 1.0,
            "a propensity left the (0,1] range: {}",
            impression.propensity
        );
        assert_eq!(impression.slot_kind, SlotKind::Ranked);
    }
    let total: f64 = impressions.iter().map(|i| i.propensity).sum();
    assert!((total - 1.0).abs() < 1e-9, "propensities summed to {total}");
}

/// The propensity the database will actually accept.
///
/// §47.3's constraint is `propensity > 0 AND propensity <= 1`, so a computed
/// propensity the database rejects is not a working pipeline. Probed directly
/// rather than inferred: §47.8 calls this constraint unforgeable, and it is
/// worth knowing that it refuses.
///
/// Zero is the case that matters, because inverse-propensity scoring divides by
/// it — a row that slipped in at zero would make the offline estimate
/// meaningless rather than loudly wrong. 1.0 is checked as the inclusive
/// boundary, so the check is not off by one in the direction that would reject
/// a legitimate single-candidate impression.
#[tokio::test]
async fn the_database_refuses_a_zero_propensity_and_accepts_one() {
    let h = Harness::new("rank_propensity_constraint").await;
    let work = h.work(1).await;

    // `recommendation_slots.pseud_id` is NOT NULL and references `pseuds`, and a
    // NOT NULL failure fires before the CHECK and would prove nothing. So the
    // whole parent chain has to exist: account, then pseud.
    if h.tdb.is_postgres() {
        sqlx::query(
            "INSERT INTO accounts (id, status, email, created_at, updated_at) \
             SELECT gen_random_uuid(), 'active', gen_random_uuid()::text || '@t.test', \
                    now()::text, now()::text \
             WHERE NOT EXISTS (SELECT 1 FROM accounts)",
        )
        .execute(h.tdb.db().postgres_pool().expect("pg"))
        .await
        .expect("seed account (pg)");
        sqlx::query(
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
             SELECT gen_random_uuid(), (SELECT id FROM accounts LIMIT 1), \
                    'h' || substr(gen_random_uuid()::text, 1, 6), 'H', now()::text, now()::text \
             WHERE EXISTS (SELECT 1 FROM accounts) \
               AND NOT EXISTS (SELECT 1 FROM pseuds WHERE handle LIKE 'h%')",
        )
        .execute(h.tdb.db().postgres_pool().expect("pg"))
        .await
        .expect("seed pseud (pg)");
    } else {
        let now = "2026-01-01T00:00:00Z".to_owned();
        sqlx::query(
            "INSERT INTO accounts (id, status, email, created_at, updated_at) \
             SELECT lower(hex(randomblob(16))), 'active', \
                    lower(hex(randomblob(16))) || '@t.test', ?, ? \
             WHERE NOT EXISTS (SELECT 1 FROM accounts)",
        )
        .bind(&now)
        .bind(&now)
        .execute(h.tdb.db().sqlite_pool().expect("sqlite"))
        .await
        .expect("seed account (sqlite)");
        sqlx::query(
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
             SELECT lower(hex(randomblob(16))), (SELECT id FROM accounts LIMIT 1), \
                    'h' || substr(lower(hex(randomblob(16))), 1, 6), 'H', ?, ? \
             WHERE EXISTS (SELECT 1 FROM accounts) \
               AND NOT EXISTS (SELECT 1 FROM pseuds WHERE handle LIKE 'h%')",
        )
        .bind(&now)
        .bind(&now)
        .execute(h.tdb.db().sqlite_pool().expect("sqlite"))
        .await
        .expect("seed pseud (sqlite)");
    }

    let pseud: Option<String> = if h.tdb.is_postgres() {
        sqlx::query_scalar("SELECT id::text FROM pseuds LIMIT 1")
            .fetch_one(h.tdb.db().postgres_pool().expect("pg"))
            .await
            .ok()
    } else {
        sqlx::query_scalar("SELECT id FROM pseuds LIMIT 1")
            .fetch_one(h.tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .ok()
    };
    let Some(pseud) = pseud else {
        // No pseud could be seeded. Returning rather than failing: the assertion
        // is about the CHECK constraint, which cannot be reached without a valid
        // FK parent, and a failure here would report the wrong problem.
        return;
    };

    let insert = |propensity: &str| {
        (
            format!(
                "INSERT INTO recommendation_slots (id, pseud_id, work_id, request_id, \
                 position, reasons, blend_score, created_at, propensity) \
                 SELECT lower(hex(randomblob(16))), ?, ?, lower(hex(randomblob(16))), 0, \
                 '[]', 0, '2026-01-01T00:00:00Z', {propensity}"
            ),
            format!(
                "INSERT INTO recommendation_slots (id, pseud_id, work_id, request_id, \
                 position, reasons, blend_score, created_at, propensity) \
                 SELECT gen_random_uuid(), $1::uuid, $2::uuid, gen_random_uuid(), 0, \
                 '[]'::jsonb, 0, now(), {propensity}"
            ),
        )
    };

    // Returns the `Result` rather than `is_ok()`, so a failure message can name
    // the actual database error. Collapsing it to a bool turns "refused because
    // of the propensity check" and "refused because of a duplicate key" into
    // the same word, which is how a green-looking assertion ends up proving
    // nothing.
    let try_propensity = async |value: &str| -> Result<(), sqlx::Error> {
        let (sqlite, postgres) = insert(value);
        if h.tdb.is_postgres() {
            let sql = lorehaven_db::sql_owned(h.db(), sqlite, postgres);
            sqlx::query(&sql)
                .bind(&pseud)
                .bind(work.to_string())
                .execute(h.tdb.db().postgres_pool().expect("pg"))
                .await
                .map(|_| ())
        } else {
            sqlx::query(&sqlite)
                .bind(&pseud)
                .bind(work.to_string())
                .execute(h.tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .map(|_| ())
        }
    };

    // Each rejection is checked for the RIGHT reason. A `created_at` type error
    // also makes the insert fail, so `is_err()` alone would pass for the wrong
    // reason -- which is exactly what happened in the first version of this
    // test: `created_at` was bound as TEXT against a `timestamptz` column, the
    // zero and 1.5 cases "passed" on that error, and only the 1.0 boundary case
    // exposed it.
    let zero = try_propensity("0.0").await;
    let zero_err = zero.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(
        zero_err.contains("check constraint") || zero_err.contains("CHECK"),
        "the zero-propensity insert failed for the wrong reason, so this \
         assertion proves nothing: {zero_err}"
    );
    let over = try_propensity("1.5").await;
    let over_err = over.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(
        over_err.contains("check constraint") || over_err.contains("CHECK"),
        "the over-1 insert failed for the wrong reason: {over_err}"
    );
    let boundary = try_propensity("1.0").await;
    assert!(
        boundary.is_ok(),
        "the database refused a propensity of 1.0, which is the inclusive \
         boundary and is legal. The actual error was: {boundary:?}"
    );
}

// ---------------------------------------------------------------------------
// §47.4 earned vs incentivized, at the database
// ---------------------------------------------------------------------------

/// An incentivized interaction is stored as `incentivized` and the distinction
/// survives the round trip on both engines.
///
/// The unit test proves `for_source` classifies correctly. This one proves the
/// classification *reaches the database* — which is the part that is per-engine,
/// since the UPDATE is dialect-branched and a swapped bind order would compile,
/// run, and update zero rows.
#[tokio::test]
async fn the_interaction_kind_survives_the_round_trip() {
    let h = Harness::new("rank_interaction_kind").await;
    let work = h.work(2).await;
    let now = "2026-01-01T00:00:00Z".to_owned();
    let account = "77777777-7777-7777-7777-777777777777";

    // work_kudos needs a real account (FK) and work (FK), so seed both.
    if h.tdb.is_postgres() {
        sqlx::query(
            "INSERT INTO accounts (id, status, email, created_at, updated_at) \
             VALUES ($1::uuid, 'active', $2, now()::text, now()::text) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(account)
        .bind(format!("{account}@t.test"))
        .execute(h.tdb.db().postgres_pool().expect("pg"))
        .await
        .expect("seed account (pg)");
        sqlx::query(
            "INSERT INTO work_kudos (work_id, account_id, created_at) \
             VALUES ($1::uuid, $2::uuid, $3) ON CONFLICT DO NOTHING",
        )
        .bind(work.to_string())
        .bind(account)
        .bind(&now)
        .execute(h.tdb.db().postgres_pool().expect("pg"))
        .await
        .expect("seed kudos (pg)");
    } else {
        sqlx::query(
            "INSERT INTO accounts (id, status, email, created_at, updated_at) \
             VALUES (?, 'active', ?, ?, ?) ON CONFLICT (id) DO NOTHING",
        )
        .bind(account)
        .bind(format!("{account}@t.test"))
        .bind(&now)
        .bind(&now)
        .execute(h.tdb.db().sqlite_pool().expect("sqlite"))
        .await
        .expect("seed account (sqlite)");
        sqlx::query(
            "INSERT INTO work_kudos (work_id, account_id, created_at) \
             VALUES (?, ?, ?) ON CONFLICT (work_id, account_id) DO NOTHING",
        )
        .bind(work.to_string())
        .bind(account)
        .bind(&now)
        .execute(h.tdb.db().sqlite_pool().expect("sqlite"))
        .await
        .expect("seed kudos (sqlite)");
    }

    // The default a migration 0098 row was born with.
    assert_eq!(
        h.kudos_kind(&work, account).await.expect("read kind"),
        "earned",
        "a row written before 0098's distinction should read as earned"
    );

    // Reclassify it as a bounty interaction. `bounty` is incentivized BY
    // DEFINITION (section 47.4): the reader was routed there by the incentive.
    let returned = lorehaven_db::ranking::record_interaction(
        h.db(),
        &lorehaven_db::ranking::InteractionRow::Kudos {
            work_id: work.to_string(),
            account_id: account.to_owned(),
        },
        "bounty",
        0.4,
    )
    .await
    .expect("record incentivized kudos");
    assert_eq!(returned, InteractionKind::Incentivized);

    assert_eq!(
        h.kudos_kind(&work, account).await.expect("read kind"),
        "incentivized",
        "the distinction did not survive the round trip on {}",
        if h.tdb.is_postgres() {
            "postgres"
        } else {
            "sqlite"
        }
    );

    // And the obscurity recorded at read time is the one stored, not
    // recomputed: 0.4, not a value derived from anything later.
    let obscurity = h
        .kudos_obscurity(&work, account)
        .await
        .expect("read obscurity");
    assert!(
        (obscurity - 0.4).abs() < 1e-9,
        "stored obscurity was {obscurity}"
    );
}

// ---------------------------------------------------------------------------
// §47.7 scout value
// ---------------------------------------------------------------------------

/// Scout value is a pure function, so this test is about the *contract*: an
/// engagement's value is fixed by the obscurity in force when it happened.
#[test]
fn a_scouted_obscure_work_keeps_its_value_after_becoming_popular() {
    let obscurity_at_read = 0.95_f64;
    let rating_later = 0.9_f64;
    let recorded = scout_value(obscurity_at_read, rating_later);

    // "Later" the work is famous. Recomputing with today's obscurity — the bug
    // §47.7 names — would pay almost nothing, because obscurity is now 0.
    let obscurity_today = 0.0_f64;
    let recomputed_wrongly = scout_value(obscurity_today, rating_later);

    assert!(
        recorded > 0.8,
        "expected a high scout value, got {recorded}"
    );
    assert_eq!(
        recorded, 0.855,
        "the recorded value is not reproducible from the stored inputs"
    );
    assert!(
        recomputed_wrongly < recorded / 10.0,
        "recomputing from today's obscurity barely differs, so the mechanism is \
         not actually rewarding early reads"
    );
    // And the stored weight is what a credit ledger must use.
    assert!((scout_value(obscurity_at_read, rating_later) - recorded).abs() < f64::EPSILON);
}

// ---------------------------------------------------------------------------
// §47.3 propensity arithmetic, integration level
// ---------------------------------------------------------------------------

/// The propensity helper and the database constraint agree on the range.
#[test]
fn computed_propensities_satisfy_the_database_constraint() {
    for scores in [
        vec![1.0, 1.0, 1.0],
        vec![0.0, 0.0],
        vec![-3.0, 2.0, 8.0],
        vec![f64::MIN_POSITIVE, 1.0],
    ] {
        for propensity in ranked_propensity(&scores) {
            assert!(
                propensity > 0.0 && propensity <= 1.0,
                "{propensity} from {scores:?} would be rejected by the 0098 constraint"
            );
        }
    }
}

/// Nothing in this file may depend on the ordering of the test harness's global
/// rate limiter: the pure-logic tests above and below run without an HTTP client,
/// so they cannot be starved by an auth quota from a neighbouring binary.
#[test]
fn this_binary_needs_no_sign_ins() {
    assert!(
        !TEST_PASSWORD.is_empty(),
        "the shared password constant is unset"
    );
}
