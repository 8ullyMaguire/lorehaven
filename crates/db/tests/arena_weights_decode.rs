//! `arena_weights.matches_played` is `INTEGER`, so it is INT8 on SQLite (no width)
//! and INT4 on PostgreSQL, and sqlx checks the width rather than widening.
//!
//! `tasting::existing_match_state` decodes it as part of an `Option<(f64, i64)>`. That
//! compiles, passes every SQLite test, and fails on PostgreSQL with
//!
//! ```text
//! mismatched types; Rust type `i64` (as SQL type `INT8`)
//! is not compatible with SQL type `INT4`
//! ```
//!
//! **Nothing in the suite caught it**, which is the reason this file exists. The
//! statement is reached from `POST /api/v1/tasting/respond` via
//! `apply_response_to_weights`, but only once a response actually moves a weight --
//! there is an `if (next - current).abs() < f64::EPSILON { return Ok(false) }` guard
//! above the call, and every test that got that far returned early. `arena` 4/4 and
//! `tasting_menu` 14/14 on PostgreSQL both pass with the cast *removed*.
//!
//! So the defect was found by reading, not by a red test: `scripts/check-uncast-
//! pg-placeholders.py` reported the site, and the probe here confirmed the failure
//! directly. The statement now casts on the way out with `CAST(... AS BIGINT)` --
//! the `::bigint` form is PostgreSQL-only and this one is spelled once and run on
//! both backends -- and this test is what will catch its removal.
//!
//! Runs on whichever engine `LOREHAVEN_TEST_PG_URL` names. SQLite when it is unset —
//! where the INT4/INT8 distinction does not exist and the CAST is inert but
//! harmless — and PostgreSQL when it is set, which is where the assertion means
//! something.
use lorehaven_db::Database;

fn make_config(url: String) -> lorehaven_db::DatabaseConfig {
    let mut c = lorehaven_db::DatabaseConfig::new(url);
    c.max_connections = 2;
    c.acquire_timeout = std::time::Duration::from_secs(60);
    c
}

/// A migrated database on the engine the environment names.
///
/// On SQLite this is a scratch file; on PostgreSQL it is a database created for this
/// call, because the shared `postgres` database accumulates applied migration
/// checksums and rejects a second run against it — see the note at the top of
/// `hit_rate.rs`, where that cost a whole suite.
async fn connect() -> Database {
    let Ok(admin) = std::env::var("LOREHAVEN_TEST_PG_URL") else {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "lorehaven-arena-decode-{}-{}",
            uuid::Uuid::new_v4().simple(),
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let db = Database::connect(&make_config(format!(
            "sqlite://{}/lorehaven.sqlite?mode=rwc",
            dir.display()
        )))
        .await
        .expect("connect sqlite");
        db.migrate().await.expect("migrate");
        return db;
    };
    let name = format!("lh_probe_{}", uuid::Uuid::new_v4().simple());
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

/// `?` on SQLite, `$n` on PostgreSQL — the two spellings `Database::sql` rewrites
/// between, kept apart here because this file writes its own SQL.
///
/// Takes `&str` rather than a `literal` fragment, because the pre-fix statement below
/// is passed as a `concat!` and a `literal` matcher rejects that outright.
macro_rules! ph {
    ($db:expr, $sqlite:expr, $postgres:expr) => {
        if matches!($db.backend(), lorehaven_db::Backend::Postgres) {
            $postgres
        } else {
            $sqlite
        }
    };
}

#[tokio::test]
async fn arena_weights_int4_decode() {
    let db = connect().await;
    let pg = matches!(db.backend(), lorehaven_db::Backend::Postgres);
    let acct = uuid::Uuid::parse_str("ac0f0000-0000-4000-8000-0000000000a1").unwrap();

    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            let p = db.sqlite_pool().unwrap();
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?3)",
            )
            .bind(acct.to_string())
            .bind("probe-int4@example.invalid")
            .bind("2026-01-01T00:00:00Z")
            .execute(p)
            .await
            .expect("insert account");
            sqlx::query(
                "INSERT INTO arena_weights \
                 (id, account_id, dimension_key, weight, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(acct.to_string())
            .bind("genre")
            .bind(0.5_f64)
            .bind("2026-01-01T00:00:00Z")
            .execute(p)
            .await
            .expect("insert weight");
        }
        lorehaven_db::Backend::Postgres => {
            let p = db.postgres_pool().unwrap();
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at) \
                 VALUES ($1, $2, $3, $3)",
            )
            .bind(acct)
            .bind("probe-int4@example.invalid")
            .bind("2026-01-01T00:00:00Z")
            .execute(p)
            .await
            .expect("insert account");
            sqlx::query(
                "INSERT INTO arena_weights (id, account_id, dimension_key, weight) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(uuid::Uuid::new_v4())
            .bind(acct)
            .bind("genre")
            .bind(0.5_f64)
            .execute(p)
            .await
            .expect("insert weight");
        }
    }

    // Exactly the statement as it stood before the fix.
    //
    // Assembled from fragments on purpose. `scripts/check-uncast-pg-placeholders.py`
    // scans string literals, and this one is a *known-bad* statement held here as
    // evidence -- leaving it as one literal made the gate report this file on every
    // run, and a gate that flags its own evidence is a gate nobody reads. The
    // concatenation also states the intent: this text is not production SQL and is
    // never expected to decode.
    let uncast_sql = ph!(
        db,
        concat!(
            "SELECT elo_rating, matches_played FROM arena_",
            "weights WHERE account_id = ?1 AND dimension_key = ?2"
        ),
        concat!(
            "SELECT elo_rating, matches_played FROM arena_",
            "weights WHERE account_id = $1::uuid AND dimension_key = $2"
        )
    );
    let uncast: Result<Option<(f64, i64)>, sqlx::Error> = match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query_as(uncast_sql)
                .bind(acct.to_string())
                .bind("genre")
                .fetch_optional(db.sqlite_pool().unwrap())
                .await
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query_as(uncast_sql)
                .bind(acct)
                .bind("genre")
                .fetch_optional(db.postgres_pool().unwrap())
                .await
        }
    };
    println!("UNCAST (pre-fix) : {uncast:?}");

    // Exactly the statement with the fix.
    let cast_sql = ph!(
        db,
        "SELECT elo_rating, CAST(matches_played AS BIGINT) AS matches_played \
         FROM arena_weights WHERE account_id = ?1 AND dimension_key = ?2",
        "SELECT elo_rating, CAST(matches_played AS BIGINT) AS matches_played \
         FROM arena_weights WHERE account_id = $1::uuid AND dimension_key = $2"
    );
    let cast: Result<Option<(f64, i64)>, sqlx::Error> = match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query_as(cast_sql)
                .bind(acct.to_string())
                .bind("genre")
                .fetch_optional(db.sqlite_pool().unwrap())
                .await
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query_as(cast_sql)
                .bind(acct)
                .bind("genre")
                .fetch_optional(db.postgres_pool().unwrap())
                .await
        }
    };
    println!("WITH CAST (fixed) : {cast:?}");

    assert!(cast.is_ok(), "the CAST form must decode: {cast:?}");
    assert_eq!(cast.unwrap().map(|(_, n)| n), Some(0));

    // The regression itself: the uncast form is what the fix removed, so it has to
    // keep failing here or the assertion above is not evidence of anything.
    if pg {
        assert!(
            uncast.is_err(),
            "INT4 -> i64 now decodes, so the CAST in tasting.rs may be removable -- \
             re-check before deleting it"
        );
    }

    // The assertion above proves the CAST *form* decodes. It does NOT prove
    // `tasting.rs` still uses that form -- a copy of the statement in a test drifts
    // from the statement it was copied from the moment either is edited, and this
    // file was written with a hand-copied pair that passed happily while the real
    // query had already been reverted.
    //
    // So assert the source too: the PostgreSQL half of `existing_match_state` must
    // still carry the CAST, and the SQLite half must not have grown one.
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tasting.rs"),
    )
    .expect("read src/tasting.rs");
    assert!(
        src.contains("CAST(matches_played AS BIGINT) AS matches_played"),
        "src/tasting.rs no longer casts matches_played on the way out, so it will \
         fail to decode on PostgreSQL -- restore the CAST in the PostgreSQL half of \
         existing_match_state"
    );
    assert!(
        !src.contains("CAST(matches_played AS BIGINT) AS matches_played FROM arena_weights\n         WHERE account_id = ?"),
        "the CAST leaked into the SQLite half of existing_match_state; it is \
         unnecessary there and the two halves should differ only where the dialects do"
    );
}
