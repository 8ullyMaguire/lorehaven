//! The SQL side of vote decay, and the reason it is a separate module from
//! the curve itself.
//!
//! `vote_decay::decay` computes a multiplier in Rust. The score query has to
//! compute the same multiplier in SQL, because the score is recomputed on read
//! and not stored (§3.3 of the amendment). Two implementations of one rule is
//! the arrangement; this module is the *other* one, and
//! `crates/app/tests/vote_decay_parity.rs` is what stops them drifting.
//!
//! # Why this is constructed rather than written
//!
//! Three dialect facts, each of which fails differently and none of which a
//! SQLite-only test would surface:
//!
//! 1. **sqlx's bundled SQLite has no math functions.** `POWER`, `exp`, `ln`
//!    and `sqrt` all return "no such function". `SQLITE_ENABLE_MATH_FUNCTIONS`
//!    is a compile-time flag the bundled build does not set, and a system
//!    SQLite that *does* have it would pass a test the bundled one fails — or
//!    the reverse. So the exponent is a product of repeated factors, which is
//!    what an integer power is.
//!
//! 2. **PostgreSQL has no scalar two-argument `min`/`max`.** Those are
//!    aggregate functions: `max(double precision, double precision)` does not
//!    exist and the query fails at *plan* time with 42883. The scalar spellings
//!    are `LEAST` and `GREATEST`.
//!
//! 3. **PostgreSQL's bare decimal literals are `NUMERIC`,** not float, so an
//!    uncast `0.0` leaves `GREATEST` with no resolvable overload — the same
//!    42883 from a different cause.
//!
//! # The clamp is load-bearing, not defensive
//!
//! Without clamping the age into `[0, 1]` first, a vote older than the cutoff
//! gives a *negative* base raised to an even power, which is positive on both
//! engines. A 400-day-old vote with a 60-day cutoff scores `(1 - 400/60)^2 =
//! 28.4` — one stale vote outweighing 28 fresh ones — and because both
//! backends agree on the arithmetic, no dialect-parity test would report it.
//! It is asserted structurally in the parity test for that reason.

use crate::vote_decay::Decay;

/// Which SQL dialect to render for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
}

/// Age in days between `now` and the vote's `voted_at`, as a float.
///
/// Fractional in both dialects on purpose: an integer day count would make the
/// curve a staircase, so a vote would sit at full weight for 24 hours and then
/// step down, which is not the rule being asked for.
///
/// `alias` qualifies the column. It is not optional: the score query wraps this
/// in a subquery over `directory_votes AS v`, and PostgreSQL cannot resolve a
/// bare `voted_at` inside it — the name is not in scope. SQLite resolves it
/// against the outer query and appears to work, so the unqualified form is a
/// silent failure that only PostgreSQL reports, and it reports it by returning
/// the *undecayed* sum rather than by erroring.
fn age_expr(d: Dialect, alias: &str) -> String {
    match d {
        // `voted_at` is RFC 3339 TEXT in both schemas, so julianday reads it
        // directly.
        Dialect::Sqlite => format!("(julianday('now') - julianday({alias}.voted_at))"),
        // The CAST is required: EXTRACT(EPOCH ...) returns NUMERIC on
        // PostgreSQL, which is the input that the clamp below cannot resolve.
        Dialect::Postgres => format!(
            "(CAST(EXTRACT(EPOCH FROM (NOW() - CAST({alias}.voted_at AS TIMESTAMPTZ))) / 86400.0 AS DOUBLE PRECISION))"
        ),
    }
}

/// The decay multiplier for the `voted_at` column of the enclosing query.
///
/// Returns a parenthesised expression: `((1.0 - MIN(...)) * (1.0 - MIN(...)))`
/// for exponent 2, and `(1.0 - MIN(...))` for exponent 1.
#[must_use]
pub fn decay_sql(cfg: &Decay, d: Dialect, alias: &str) -> String {
    // With decay off the multiplier is the constant 1, and the caller is better
    // served by a query that does not mention the age at all. Returning "1.0"
    // keeps the statement shape identical across the config change, so there
    // is no second code path to keep in sync.
    if !cfg.enabled {
        return "1.0".to_string();
    }

    let (one, zero) = match d {
        Dialect::Sqlite => ("1.0", "0.0"),
        Dialect::Postgres => (
            "CAST(1.0 AS DOUBLE PRECISION)",
            "CAST(0.0 AS DOUBLE PRECISION)",
        ),
    };
    let (hi, lo) = match d {
        Dialect::Sqlite => ("MIN", "MAX"),
        Dialect::Postgres => ("LEAST", "GREATEST"),
    };
    let remaining = format!(
        "(1.0 - {hi}({one}, {lo}({zero}, {age} / {cutoff})))",
        age = age_expr(d, alias),
        // The cutoff is an operator-configured float interpolated into the
        // statement rather than bound. It is validated to be > 0 by
        // `Decay::from_config`, and a bound parameter cannot be used here
        // without changing the shape of the query per call site. `cutoff` is
        // rendered with `{cutoff}` formatting of an f64, which cannot produce
        // SQL syntax: Rust's float Display is not a valid SQL injection vector
        // here because the value never came from a request.
        cutoff = cfg.cutoff_days,
    );

    // `0..n` and not `1..n`: this is how many *copies* of the factor appear, and
    // an off-by-one here silently turns the squared curve into a linear one —
    // which still evaluates, still declines monotonically, and quietly halves
    // every voter's weight.
    std::iter::repeat_n(remaining.as_str(), cfg.exponent.max(1) as usize)
        .collect::<Vec<_>>()
        .join(" * ")
}

/// A vote's contribution to an entry's score: `vote_value × base_weight × decay`.
///
/// `alias` qualifies the vote table so this can be dropped into a query that
/// joins votes to entries without the subquery seeing two `voted_at` columns.
#[must_use]
pub fn vote_contribution_sql(alias: &str, cfg: &Decay, d: Dialect) -> String {
    format!(
        "({alias}.vote_value * {alias}.base_weight * {decay})",
        decay = decay_sql(cfg, d, alias)
    )
}

/// A correlated subquery summing the decayed votes for one entry.
///
/// `entry_param` is the placeholder that names the entry. It is a parameter
/// rather than baked in because the caller owns the numbering: `Database::sql`
/// rewrites `?` to `$n` and would not know which `$n` this is, and a statement
/// that carried its own literal `$1` would be a `?` that never gets rewritten.
///
/// Returns the expression alone, so the caller controls the surrounding
/// `FROM`/`WHERE` and the cast that makes the `SUM` decodable.
#[must_use]
pub fn decayed_score_sum_sql(cfg: &Decay, d: Dialect, entry_param: &str) -> String {
    // The inner subquery aliases the vote table `v` so the decay expression
    // sees exactly one `voted_at` column regardless of what the outer query
    // joins.
    let contribution = vote_contribution_sql("v", cfg, d);
    let alias = match d {
        Dialect::Sqlite => " v",
        Dialect::Postgres => " AS v",
    };
    // `COALESCE(..., 0)` for an entry with no votes. Note the literal is
    // deliberately bare: on PostgreSQL it is NUMERIC, but it is the whole
    // COALESCE result that gets cast by the caller, and casting the inner SUM
    // is what keeps the result decodable into f64.
    format!(
        "COALESCE((SELECT SUM({contribution}) FROM directory_votes{alias} WHERE v.entry_id = {entry_param}), 0)"
    )
}

/// How many vote rows an entry has.
///
/// **Every** row, including ones whose contribution has decayed to zero.
/// Counting only the live ones makes the `min_votes` threshold a function of
/// the clock: an entry sits just above the threshold while its votes are alive
/// and drops below it the instant they all expire, at which point it becomes
/// exempt and its score jumps back to full weight. Counting rows makes the
/// threshold a property of the entry, so ageing cannot cross it.
///
/// `entry_param` is the caller's placeholder for the entry id, for the same
/// reason as [`decayed_score_sum_sql`]. `COUNT(*)` is `int8` on both engines,
/// so the `CAST` is belt-and-braces rather than a requirement.
#[must_use]
pub fn vote_count_sql(d: Dialect, entry_param: &str, wrapped: bool) -> String {
    // `AS v` on PostgreSQL because `v.entry_id` needs a name to qualify
    // against, and the alias is spelled per dialect rather than shared: the
    // SQLite form without it is also valid on PostgreSQL, but having one
    // spelling removes a difference to keep in sync.
    let alias = match d {
        Dialect::Sqlite => " v",
        Dialect::Postgres => " AS v",
    };
    // `wrapped` exists because the two uses need different text. As the left
    // operand of `CASE WHEN <here> >= n` a bare SELECT is a syntax error on
    // both engines, so that caller needs the parentheses; as a statement in its
    // own right a leading `(` is a syntax error on SQLite. Asking the builder
    // which shape is wanted is clearer than stripping a character afterwards
    // and hoping the string was what the caller thought it was.
    let stmt = format!(
        "SELECT CAST(COUNT(*) AS BIGINT) FROM directory_votes{alias} WHERE v.entry_id = {entry_param}"
    );
    if wrapped {
        format!("({stmt})")
    } else {
        stmt
    }
}
