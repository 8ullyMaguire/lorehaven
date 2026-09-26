//! The decay curve has to exist in two places, and this is the file that makes
//! that survivable.
//!
//! `vote_decay::decay` computes the multiplier in Rust; the score query
//! computes it in SQL. Those are two implementations of one rule, and the
//! failure mode is not a crash — it is a directory that quietly ranks by a
//! slightly different curve than the one an operator configured and a reader
//! was told about. Nothing errors. The score is just wrong by a few percent,
//! permanently, in a way no test notices unless a test compares the two.
//!
//! So this test renders the SQL expression and evaluates it against the Rust
//! function at a spread of ages and parameter sets. If someone changes one
//! and not the other, this fails.

use lorehaven_db::{Backend, Database};
use lorehaven_domain::vote_decay::{decay, Decay};
use test_support::TestDb;

/// A unique scratch directory, so the two tests do not share one SQLite file.
fn scratch() -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("lh-vote-decay-{n}"))
}

/// The age-in-days expression, per dialect.
///
/// `voted_at` is RFC 3339 TEXT in both schemas, so SQLite can use `julianday`
/// directly and PostgreSQL needs a parse before it can subtract. Both produce
/// a *fractional* day count -- an integer day count would make the curve a
/// staircase, and "this vote is full weight for 24 hours then drops" is not
/// the rule anyone asked for.
fn age_expr(backend: Backend) -> String {
    match backend {
        Backend::Sqlite => "(julianday('now') - julianday(voted_at))".to_string(),
        // The CAST is not optional. `EXTRACT(EPOCH ...)` returns NUMERIC on
        // PostgreSQL, and there is no `max(numeric, numeric)` overload, so the
        // clamp below fails with 42883 until the result is a float. SQLite's
        // julianday is already REAL and needs no cast, so this is a real
        // divergence that forces the expression to be per-dialect rather than
        // one shared string.
        Backend::Postgres => {
            "(CAST(EXTRACT(EPOCH FROM (NOW() - CAST(voted_at AS TIMESTAMPTZ))) / 86400.0 AS DOUBLE PRECISION))"
                .to_string()
        }
    }
}

/// The score-query decay expression, parameterised by cutoff and exponent.
///
/// The clamp is the load-bearing part and is identical in both dialects.
fn sql_decay_expr(backend: Backend, cutoff: f64, exponent: u32) -> String {
    // Three dialect facts force this expression to be built rather than
    // written, and each one fails differently:
    //
    //  1. **No math functions in SQLite.** sqlx's bundled SQLite reports
    //     "no such function" for `POWER`, `exp`, `ln` and `sqrt` alike --
    //     `ENABLE_MATH_FUNCTIONS` is a compile-time flag the bundled build does
    //     not set. So the curve is repeated multiplication (`t * t`), which is
    //     what an integer power *is*. This is also why `Decay::exponent` is a
    //     `u32`: a fractional exponent would be expressible in Rust and
    //     unreachable here, and the two would silently disagree.
    //
    //  2. **PostgreSQL has no scalar two-argument min/max.** Those are
    //     aggregates: `max(double precision, double precision)` does not exist
    //     and the query dies at plan time with 42883. The scalar forms are
    //     `LEAST`/`GREATEST`.
    //
    //  3. **PostgreSQL's bare decimal literals are NUMERIC**, so an uncast
    //     `0.0` makes `GREATEST` fail to resolve against a double. The age is
    //     already cast, so the literals are cast to match.
    let (one, zero) = match backend {
        Backend::Sqlite => ("1.0", "0.0"),
        Backend::Postgres => (
            "CAST(1.0 AS DOUBLE PRECISION)",
            "CAST(0.0 AS DOUBLE PRECISION)",
        ),
    };
    let (hi, lo) = match backend {
        Backend::Sqlite => ("MIN", "MAX"),
        Backend::Postgres => ("LEAST", "GREATEST"),
    };
    let remaining = format!(
        "(1.0 - {hi}({one}, {lo}({zero}, {age} / {cutoff})))",
        age = age_expr(backend)
    );
    // The power as a product, not a call. `t * t` for exponent 2. Note `0..n`
    // and not `1..n`: the range is how many *copies* of `t` appear, and an
    // off-by-one here silently turns the squared curve into a linear one --
    // which still passes a "did it evaluate" test and quietly halves every
    // voter's weight.
    let powered = std::iter::repeat_n(remaining.as_str(), exponent.max(1) as usize)
        .collect::<Vec<_>>()
        .join(" * ");
    format!("({powered})")
}

struct Case {
    label: &'static str,
    cutoff_days: f64,
    exponent: u32,
    ages_days: &'static [f64],
}

const CASES: &[Case] = &[
    Case {
        label: "defaults (60d, squared)",
        cutoff_days: 60.0,
        exponent: 2,
        ages_days: &[0.0, 1.0, 7.0, 14.0, 30.0, 45.0, 59.0, 60.0, 61.0, 400.0],
    },
    Case {
        label: "a two-week cutoff",
        cutoff_days: 14.0,
        exponent: 2,
        ages_days: &[0.0, 1.0, 3.0, 7.0, 13.0, 14.0, 20.0],
    },
    Case {
        label: "a linear ramp",
        cutoff_days: 60.0,
        exponent: 1,
        ages_days: &[0.0, 5.0, 20.0, 50.0, 60.0],
    },
    Case {
        label: "an aggressive curve",
        cutoff_days: 90.0,
        exponent: 3,
        ages_days: &[0.0, 10.0, 45.0, 80.0, 90.0],
    },
];

/// Evaluate the decay expression against the real engine and return the
/// weight, so Rust and SQL are compared on the engine's own arithmetic rather
/// than on a reimplementation of it.
async fn sql_weight(
    db: &Database,
    backend: Backend,
    age_days: f64,
    cutoff: f64,
    exponent: u32,
) -> f64 {
    let expr = sql_decay_expr(backend, cutoff, exponent);
    match backend {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite pool");
            // Seconds with three decimals, not `-1 days`. SQLite's
            // datetime() modifier truncates to whole seconds, so a day-boundary
            // age lands on 0.9999... or 1.0000... depending on the sub-second
            // clock, and a test that passes on one machine fails on another
            // for a difference of 1e-4 in the curve.
            sqlx::query_scalar(&format!(
                "SELECT {expr} FROM (SELECT datetime('now', ?) AS voted_at)"
            ))
            .bind(format!("-{:.3} seconds", age_days * 86_400.0))
            // `datetime('now', '-N seconds')` IS the past, so the SQLite arm
            // keeps the minus and PostgreSQL drops it. Same instant, opposite
            // sign, because SQLite's modifier subtracts and PostgreSQL's
            // interval is added to reach the past.
            .fetch_one(pool)
            .await
            .expect("decay expression must evaluate on SQLite")
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres pool");
            sqlx::query_scalar(&format!(
                // A literal `$1`, not `?`: this calls `sqlx::query` directly
                // and so gets none of the `?` -> `$n` rewriting that
                // `Database::sql_owned` owns. A `?::interval` reaches the
                // server as a literal question mark.
                //
                // Seconds, not `-1 days`. An interval of `-1 days` is *exactly*
                // one day, so the age comes out as 0.0 and every row in the
                // table reads as a fresh vote -- the curve is never exercised.
                // A fractional multiplier is the only way to express a
                // fractional age on this engine.
                "SELECT {expr} FROM (SELECT (NOW() - (($1::double precision) * INTERVAL '1 second')) AS voted_at)"
            ))
            // `voted_at` is in the PAST, so the multiplier is positive. The
            // age expression is `NOW() - voted_at`; a negative multiplier
            // would put `voted_at` in the future, make the age negative, and
            // let the clamp floor it to zero -- which is exactly the "every
            // age reads as a fresh vote" failure the table exists to catch.
            .bind(format!("{:.3}", age_days * 86_400.0))
            .fetch_one(pool)
            .await
            .expect("decay expression must evaluate on PostgreSQL")
        }
    }
}

fn backend_of(tdb: &TestDb) -> Backend {
    if tdb.db().postgres_pool().is_some() {
        Backend::Postgres
    } else {
        Backend::Sqlite
    }
}

#[test]
fn the_sql_expression_clamps_the_age_before_the_power() {
    // Stated per dialect, but the clamp is identical in both, and it is the
    // whole reason the SQL is not literally "1 - age/cutoff".
    //
    // Without it, a vote from the future (clock skew between hosts) gives a
    // negative base, and an age past the cutoff gives a negative base raised
    // to 2.0 -- which is POSITIVE on both engines. A vote 400 days old would
    // evaluate to (1 - 400/60)^2 = 28.4: one stale vote worth 28 fresh ones,
    // silently, and the two backends would still agree so no dialect test
    // would catch it.
    for backend in [Backend::Sqlite, Backend::Postgres] {
        let expr = sql_decay_expr(backend, 60.0, 2);
        let (lo, hi) = match backend {
            Backend::Sqlite => ("MAX(", "MIN("),
            Backend::Postgres => ("GREATEST(", "LEAST("),
        };
        assert!(
            expr.contains(lo),
            "{backend:?}: age not clamped at zero: {expr}"
        );
        assert!(
            expr.contains(hi),
            "{backend:?}: age not clamped at the cutoff: {expr}"
        );
        // No `POWER`/`exp`/`ln`/`sqrt`: sqlx's bundled SQLite has no math
        // functions at all, so a query that evaluates on PostgreSQL would
        // return "no such function" on every SQLite deployment. This is the
        // assertion that would have caught it, and it is a negative assertion
        // on purpose.
        for forbidden in ["POWER(", "exp(", "ln(", "sqrt("] {
            assert!(
                !expr.contains(forbidden),
                "{backend:?}: uses {forbidden}, which sqlx's SQLite does not have: {expr}"
            );
        }
        // The exponent is a product, so the curve is the same operation in
        // both dialects and an off-by-one in the copy count cannot pass.
        assert!(
            expr.contains(" * "),
            "{backend:?}: the exponent did not become a product: {expr}"
        );
    }
}

#[test]
fn the_age_is_fractional_on_both_dialects() {
    // An integer day count makes the curve a staircase: a vote full weight for
    // 24 hours, then a step down. SQLite's julianday and PostgreSQL's EXTRACT
    // both return fractions; this pins that the expression keeps them.
    for backend in [Backend::Sqlite, Backend::Postgres] {
        let expr = age_expr(backend);
        assert!(
            !expr.contains("DATE(") && !expr.contains("DATE_TRUNC"),
            "{backend:?}: the age is truncated to a whole day: {expr}"
        );
        assert!(
            expr.contains("86400.0") || expr.contains("julianday"),
            "{backend:?}: the age is not in days: {expr}"
        );
    }
}

#[test]
fn the_rust_function_never_leaves_the_unit_range() {
    // Stated on the Rust side so it is pinned independently of either engine.
    for case in CASES {
        for age in case.ages_days {
            let w = decay(
                *age,
                &Decay::from_config(true, case.cutoff_days, 0, case.exponent),
            );
            assert!(
                (0.0..=1.0).contains(&w),
                "{}: age {age} produced {w}, outside the unit range",
                case.label
            );
        }
    }
}

#[tokio::test]
async fn the_sql_and_rust_values_agree() {
    // The whole reason this file exists. The age is injected by backdating
    // `voted_at` rather than by an arithmetic placeholder, so the expression
    // under test is the one the production query really runs.
    let tdb = TestDb::connect_with_dir("decay_parity", &scratch()).await;
    let backend = backend_of(&tdb);

    for case in CASES {
        let cfg = Decay::from_config(true, case.cutoff_days, 0, case.exponent);
        for age in case.ages_days {
            let rust = decay(*age, &cfg);
            let sql = sql_weight(tdb.db(), backend, *age, case.cutoff_days, case.exponent).await;
            assert!(
                (rust - sql).abs() <= 1e-3,
                "{} on {backend:?}: at age {age}d Rust says {rust} and SQL says {sql}",
                case.label
            );
        }
    }
}

#[tokio::test]
async fn a_stale_vote_evaluates_to_exactly_zero_in_sql() {
    // The design requires an exact zero at the cutoff. Assert the engine's
    // rendered value rather than trusting the arithmetic -- POWER(0.0, 2.0)
    // is the only place that zero is actually guaranteed.
    let tdb = TestDb::connect_with_dir("decay_zero", &scratch()).await;
    let backend = backend_of(&tdb);
    for age in [60.0, 61.0, 400.0, 3650.0] {
        let w = sql_weight(tdb.db(), backend, age, 60.0, 2).await;
        assert_eq!(
            w, 0.0,
            "{age}d old evaluated to {w} on {backend:?}, not zero"
        );
    }
}

#[tokio::test]
async fn a_fresh_vote_evaluates_to_exactly_one_in_sql() {
    // The other end of the range. A curve that started below 1.0 would
    // penalise everyone for existing, and would be invisible next to the
    // "roughly 3% less" differences the rule is actually about.
    let tdb = TestDb::connect_with_dir("decay_one", &scratch()).await;
    let backend = backend_of(&tdb);
    let w = sql_weight(tdb.db(), backend, 0.0, 60.0, 2).await;
    assert!(
        (w - 1.0).abs() < 1e-6,
        "a vote cast now evaluated to {w} on {backend:?}, not 1.0"
    );
}
