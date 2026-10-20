//! The harness leaks its scratch directories, and that is pinned here.
//!
//! **This test asserts a defect.** It asserts that a scratch directory OUTLIVES
//! the test that made it, which is the current behaviour — and the reason the
//! entry exists is that a fix has to turn this test **red**, not leave it
//! quietly green.
//!
//! The defect is real and it is expensive: a full workspace run leaves thousands of
//! `darkhold-test-*` directories under `TMPDIR`, and on the development host `/tmp`
//! is a **16 GB tmpfs**. A run that leaves 1,399 of them, 6.9 GB, fills the
//! filesystem and the suite fails with
//!
//! ```text
//! error returned from database: (code: 13) database or disk is full
//! ```
//!
//! which reads like a storage problem and is a tmpfs problem. Twelve suites of
//! M1's gate run failed that way.
//!
//! The fix is a shared template directory plus a sweep at exit, and **not** a
//! naive `remove_dir_all` in a `Drop`: a `Drop` that deletes a directory a
//! still-running test holds a handle to would be a worse bug than the leak.

use test_support::{scratch_dir, TestDb};

#[tokio::test]
async fn a_scratch_directory_outlives_its_test() {
    let dir = scratch_dir("leak-pin");
    assert!(dir.exists(), "the scratch directory was created");

    // A real database in it. `connect_with_dir` migrates as it goes, so the
    // directory is a real one and not an empty shell.
    let db = TestDb::connect_with_dir("leak-pin", &dir).await;
    assert!(
        !db.applied_migrations().is_empty(),
        "the database was migrated, so the directory has something in it"
    );

    // The database is dropped at the end of the test — and the DIRECTORY is not.
    drop(db);

    assert!(
        dir.exists(),
        "the scratch directory is cleaned up: the harness no longer leaks it, and the \
         known-gaps entry M1-D01 can be closed"
    );
}
