//! Reclaim SQLite scratch databases left behind by test runs.
//!
//! Run with `just clean-scratch`, or directly:
//!
//! ```text
//! cargo run -p test-support --example sweep-scratch
//! ```
//!
//! ## Why this exists
//!
//! 134 of the 182 test files never call `TestDb::cleanup`, and nothing else sweeps
//! `std::env::temp_dir()`. One `cargo test --workspace` left 6,925 `lorehaven-*`
//! directories totalling **29GB** — on a filesystem that was already 15GB into swap.
//! That pressure is not cosmetic: it is what made pool construction outrun the
//! 10s `acquire_timeout` and fail 12–17 tests with
//! `pool timed out while waiting for an open connection`, none of which was a bug in
//! any suite. The measurements are in `test_db_config`'s doc comment.
//!
//! ## When it is safe
//!
//! **Between test runs, not during one.** The liveness test is the file's
//! modification time with a five-minute threshold, because `std` offers no way to
//! ask a SQLite file whether anyone currently holds it open — unlike the PostgreSQL
//! sweep, which asks `pg_database` about attached backends and is authoritative.
//!
//! The bias is deliberately one way: calling a live database dead leaves a directory
//! behind (reclaimed next run), while calling a dead one live only delays cleanup.
//! Getting that backwards would delete a running test's database and produce a
//! missing-database failure in an unrelated suite.
//!
//! So do not wire this into the test run itself. It is a housekeeping command.

fn main() {
    let removed = test_support::sweep_stale_sqlite();
    println!(
        "removed {removed} stale scratch director{}",
        if removed == 1 { "y" } else { "ies" }
    );
    if removed == 0 {
        println!("nothing to reclaim");
    }
}
