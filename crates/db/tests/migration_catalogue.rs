//! The embedded migration catalogue must match the migrations on disk.
//!
//! This test exists because of a bug it would have caught immediately.
//!
//! `crates/db/build.rs` declared `cargo:rerun-if-changed=<migrations dir>` —
//! one entry for the *directory*. Cargo watches a directory entry for its own
//! mtime, and adding a file inside a directory does not change the directory's
//! mtime. So the build script ran once, was then permanently satisfied, and the
//! embedded catalogue silently lagged behind the repository: ten migrations
//! (0074–0083) sat on disk while the catalogue stopped at 0073.
//!
//! The failure surfaced far from its cause. `crates/db/src/external.rs` had
//! started writing `api_tokens.kind`, so `POST /api/v1/me/tokens` returned
//! 500 with "table api_tokens has no column named kind" — a message naming a
//! table that migration 0083 clearly created a column for.
//!
//! Nothing in the test suite caught it, because every Lorehaven test builds its
//! own database by running the same build-script-generated catalogue. A stale
//! catalogue produces a *consistent* wrong schema, and consistency is what the
//! tests were checking. The only thing that notices is comparing the catalogue
//! against the filesystem.

use std::path::{Path, PathBuf};

// `Backend` lives in the crate root, not in `migrate`, and `Migration`'s fields
// are `&'static str` — the catalogue is generated data, so nothing allocates.
use lorehaven_db::migrate::catalogue;
use lorehaven_db::Backend;

/// The migrations directory, from this crate's manifest.
fn migrations_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/db lives two levels under the workspace root")
        .join("migrations")
}

/// Every `.sql` file in a dialect directory, as `(version, name)`.
fn on_disk(dialect: &str) -> Vec<(String, String)> {
    let dir = migrations_root().join(dialect);
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("a file stem");
        let (version, name) = stem.split_once('_').unwrap_or((stem, stem));
        out.push((version.to_owned(), name.to_owned()));
    }
    out.sort();
    out
}

/// The catalogue as built into the binary, as `(version, name)`.
fn embedded(backend: Backend) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = catalogue(backend)
        .iter()
        .map(|m| (m.version.to_owned(), m.name.to_owned()))
        .collect();
    out.sort();
    out
}

#[test]
fn the_sqlite_catalogue_matches_the_directory() {
    let on_disk = on_disk("sqlite");
    let embedded = embedded(Backend::Sqlite);
    assert_eq!(
        on_disk, embedded,
        "the embedded SQLite catalogue and migrations/sqlite/ disagree. A new \\
         migration on disk that is missing from the binary means the schema the \\
         application runs is older than the repository describes — which is how \\
         `api_tokens.kind` went missing and POST /me/tokens started returning \\
         500. If this fires after adding a migration, rebuild; if it fires \\
         without, see the rerun-if-changed note in crates/db/build.rs."
    );
}

#[test]
fn the_postgres_catalogue_matches_the_directory() {
    let on_disk = on_disk("postgres");
    let embedded = embedded(Backend::Postgres);
    assert_eq!(
        on_disk, embedded,
        "the embedded PostgreSQL catalogue and migrations/postgres/ disagree"
    );
}

#[test]
fn the_two_dialects_declare_the_same_migrations() {
    // Not the same SQL — the same *set*, in the same order. A migration that
    // exists for one dialect only is a schema that works on SQLite and fails on
    // PostgreSQL, which is the failure §4 warns about.
    let sqlite = on_disk("sqlite");
    let postgres = on_disk("postgres");
    let sqlite_ids: Vec<&str> = sqlite.iter().map(|(v, _)| v.as_str()).collect();
    let postgres_ids: Vec<&str> = postgres.iter().map(|(v, _)| v.as_str()).collect();
    assert_eq!(
        sqlite_ids, postgres_ids,
        "the dialects disagree on which migrations exist, or in what order"
    );
}

#[test]
fn the_catalogue_is_not_empty_and_covers_everything_but_nothing() {
    // A build script that resolved the wrong directory produces an empty
    // catalogue, and a run of zero migrations looks exactly like a fresh
    // database. Asserted explicitly, because "no migrations found" is only a
    // cargo warning and nothing treats it as fatal.
    for (backend, dialect) in [(Backend::Sqlite, "sqlite"), (Backend::Postgres, "postgres")] {
        let embedded = embedded(backend);
        assert!(
            embedded.len() > 50,
            "{dialect} embedded only {} migrations — the build script is \\
             probably reading the wrong directory",
            embedded.len()
        );
    }
}

#[test]
fn every_embedded_migration_has_a_distinct_version() {
    // A duplicate version means one silently shadows the other, and which one
    // wins depends on sort order — a schema that differs between machines.
    for backend in [Backend::Sqlite, Backend::Postgres] {
        let mut versions: Vec<&str> = catalogue(backend).iter().map(|m| m.version).collect();
        let before = versions.len();
        versions.sort_unstable();
        versions.dedup();
        assert_eq!(
            versions.len(),
            before,
            "{backend:?} has a duplicate migration version"
        );
    }
}

#[test]
fn the_build_script_watches_every_file_and_not_only_the_directory() {
    // The guard on the guard. This reads `crates/db/build.rs` as text and
    // asserts that it emits one `rerun-if-changed` per file, because the bug
    // this file documents is a *missing* build-script input rather than wrong
    // code — and code that is correct but never re-runs is indistinguishable
    // from code that is wrong.
    let build_rs = Path::new(env!("CARGO_MANIFEST_DIR")).join("build.rs");
    let source = std::fs::read_to_string(&build_rs).expect("build.rs is readable");

    assert!(
        source.contains("for (_, _, path) in &files")
            && source.contains("rerun-if-changed={}\", path"),
        "build.rs must emit a rerun-if-changed per migration file, not only for \\
         the directory. A directory watch does not fire when a file is added \\
         inside it, which is how the embedded catalogue ended up ten \\
         migrations behind."
    );

    // And the directory watch stays, for the directory itself being created.
    assert!(
        source.contains("rerun-if-changed={}\", dir.display())"),
        "build.rs must still watch the directory, for the case where it is \\
         created rather than populated"
    );
}
