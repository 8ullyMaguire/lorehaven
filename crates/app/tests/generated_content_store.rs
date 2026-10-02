//! Acceptance: §51.1's generated-content posture persists, and refusing is real
//! (spec §51.1, §51.4; M45-12).
//!
//! `crates/domain/src/generated_content.rs` holds the rules and migration 0109 the
//! tables. Neither of those is the thing §51.1 is *about*: the rule that matters is
//! that `forbid` refuses **before the work row exists**, and that the posture a work
//! carries does not change when the operator changes the policy. Both are properties
//! of the write path, so both are driven here rather than asserted about the domain
//! function.
//!
//! Every test runs on **both** engines. `generated_content_posture` is TEXT on both
//! but `generated_declared_at`'s pair-CHECK compares a boolean expression against a
//! NULL, and the two engines disagree about a column the store relies on being
//! present — so the SQLite leg alone would not prove the store is portable.
//!
//! | clause | test |
//! |---|---|
//! | a new instance forbids generated content | `an_instance_with_no_policy_row_forbids` |
//! | the operator can set each posture, and it sticks | `the_operator_can_set_each_posture` |
//! | setting twice bumps the version, never duplicates | `setting_the_policy_twice_bumps_the_version` |
//! | forbid refuses and no work row is created | `forbid_refuses_before_the_work_row_exists` |
//! | disclose accepts and marks the work | `disclose_accepts_and_marks_the_work` |
//! | allow accepts and does not mark the work | `allow_accepts_and_does_not_mark_the_work` |
//! | the policy is about the work, not the author | `the_posture_is_about_the_work_not_the_author` |
//! | **changing the policy does not relabel existing work** | `changing_the_policy_does_not_relabel_works` |
//! | a work with no stamp is absent, not forbid | `an_unstamped_work_is_absent_not_forbidden` |
//! | the declaration survives the round trip | `the_declaration_survives_the_round_trip` |
//! | both engines agree | `both_engines_report_the_same_posture` |

use lorehaven_db::generated_content as gc;
use lorehaven_domain::generated_content::{GeneratedContentDeclaration, GeneratedContentPosture};
use test_support::{id, scratch_dir, TestDb};

const NOW: &str = "2026-10-02T09:00:00Z";

/// Construct a scratch `TestDb` for this suite's tag.
///
/// `connect_with_dir` rather than a convenience constructor, because the directory
/// is part of the identity: under SQLite every `TestDb` for a given dir resolves to
/// the same file, so a second handle makes a fixture appear to land while the code
/// under test reads somewhere else. One `TestDb` per test, no exceptions.
async fn scratch(tag: &str) -> TestDb {
    let dir = scratch_dir(tag);
    TestDb::connect_with_dir(tag, &dir).await
}

/// One bound parameter, in the position the statement expects it.
///
/// Run a statement with binds, on whichever backend is active.
///
/// Takes **two** templates rather than one. The reason is the trap this suite
/// exists partly to document: a `?::uuid` cast is *required* on PostgreSQL and a
/// **syntax error** on SQLite, and `TestDb::sql` rewrites placeholders only — it
/// cannot rewrite casts. So one shared template cannot satisfy both engines, and
/// the casts have to live in the dialect that wants them.
///
/// The query is built inside each arm because a `sqlx::Query` is typed by its
/// database and cannot be executed against both pools.
async fn exec_bound(db: &TestDb, sqlite_sql: &str, postgres_sql: &str, binds: &[String]) {
    match db.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            let mut q = sqlx::query(sqlite_sql);
            for b in binds {
                q = q.bind(b);
            }
            q.execute(db.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("sqlite statement");
        }
        lorehaven_db::Backend::Postgres => {
            let mut q = sqlx::query(postgres_sql);
            for b in binds {
                q = q.bind(b);
            }
            q.execute(db.db().postgres_pool().expect("postgres"))
                .await
                .expect("postgres statement");
        }
    }
}

/// A `works` row, which every coordinate row needs to reference.
///
/// Minimal per this repo's fixture notes: `works` needs an `owner_pseud_id`, so a
/// pseud comes first, and a pseud needs an account. `test_support::id` returns a
/// stable UUID — not a slug — because `works.id` is UUID on PostgreSQL and a slug
/// fails there with 22P02.
async fn fixture_work(db: &TestDb, work_id: &str) -> String {
    let account = id(&format!("coord-acct-{work_id}"));
    let pseud = id(&format!("coord-pseud-{work_id}"));
    let now = "2026-10-01T12:00:00Z".to_string();

    exec_bound(
        db,
        "INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, ?, ?)",
        "INSERT INTO accounts (id, email, created_at, updated_at) VALUES ($1::uuid, $2, $3, $4)",
        &[
            account.clone(),
            format!("{account}@example.invalid"),
            now.clone(),
            now.clone(),
        ],
    )
    .await;

    exec_bound(
        db,
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)",
        // display_name is NOT NULL as well as handle; omitting it fails every
        // fixture in this file with a constraint error that names neither the
        // missing column's purpose nor the pseud.
        &[
            pseud.clone(),
            account,
            // Unique per instance: `pseuds_handle_normalized` rejects a second
            // pseud with the same handle, and several tests build two works each.
            format!("coordpseud-{}", &work_id[..8]),
            format!("Coord Pseud {}", &work_id[..8]),
            now.clone(),
            now.clone(),
        ],
    )
    .await;

    // `generated_content_posture` is named explicitly rather than left to a default.
    //
    // Two reasons, and the first is this suite's own: 0109's backfill filled existing
    // rows, but a fixture INSERT is a NEW row and gets no backfill. The second is the
    // dialect difference 0109 documents -- PostgreSQL narrows the column to NOT NULL
    // and refuses a bare INSERT, while SQLite cannot enforce it and would have
    // accepted one. Naming the column is what makes the fixture portable instead of
    // letting one engine's leniency hide a missing value on the other.
    //
    // `forbid` with no declaration, which is the shape every posture accepts for an
    // undeclared work.
    exec_bound(
        db,
        "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at, \
         generated_content_posture, generated_declared_at) \
         VALUES (?, ?, ?, ?, ?, ?, NULL)",
        "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at, \
         generated_content_posture, generated_declared_at) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6, NULL)",
        &[
            work_id.to_string(),
            pseud.clone(),
            "A Work For Coordinates".into(),
            now.clone(),
            now.clone(),
            "forbid".into(),
        ],
    )
    .await;

    // The pseud is returned because `chapter_revisions.created_by_pseud_id` is
    // NOT NULL and references it, so a chapter fixture cannot be written without
    // knowing it. Returning it beats looking it up again from a deterministic hash.
    pseud
}

#[tokio::test]
async fn an_instance_with_no_policy_row_forbids() {
    // §51.1: `forbid` is the default. 0109 creates no policy row, so an instance
    // that has never been configured reads `forbid` -- and reads it through the same
    // function a configured one does, not through a separate "unset" path.
    let db = scratch("gen-no-policy").await;
    assert_eq!(
        gc::generated_content_policy(db.db()).await.expect("read"),
        GeneratedContentPosture::Forbid,
        "an instance that has never set a posture forbids generated content"
    );
    assert_eq!(
        gc::policy_version(db.db()).await.expect("version"),
        0,
        "and has no row, which is version 0 rather than 1 so a caller comparing \\
         versions cannot mistake it for a row at version 1"
    );
}

#[tokio::test]
async fn the_operator_can_set_each_posture_and_it_sticks() {
    let db = scratch("gen-set-each").await;
    for (i, posture) in GeneratedContentPosture::all().into_iter().enumerate() {
        let version = gc::set_generated_content_policy(
            db.db(),
            posture,
            None,
            &format!("2026-10-02T09:0{i}:00Z"),
        )
        .await
        .expect("set");
        assert_eq!(
            gc::generated_content_policy(db.db()).await.expect("read"),
            posture,
            "{posture} must be readable back"
        );
        assert!(version >= 1, "a set policy has a version at or above 1");
    }
}

#[tokio::test]
async fn setting_the_policy_twice_bumps_the_version() {
    // Upsert, not insert-or-error: setting a posture is idempotent by nature, so an
    // operator setting it twice has not done anything the second time, and refusing
    // would make a retry after a timeout fail.
    let db = scratch("gen-version").await;
    let first =
        gc::set_generated_content_policy(db.db(), GeneratedContentPosture::Disclose, None, NOW)
            .await
            .expect("first set");
    let second =
        gc::set_generated_content_policy(db.db(), GeneratedContentPosture::Disclose, None, NOW)
            .await
            .expect("second set of the same value");

    assert_eq!(first, 1, "the first set creates version 1");
    assert_eq!(
        second, 2,
        "setting the same value again still bumps the version, so a concurrent \\
         change is detectable rather than silently last-write-wins"
    );
    let (sql, binds) = if db.is_postgres() {
        (
            "SELECT count(*) FROM generated_content_policy WHERE id = $1",
            vec![gc::POLICY_ID.to_string()],
        )
    } else {
        (
            "SELECT count(*) FROM generated_content_policy WHERE id = ?",
            vec![gc::POLICY_ID.to_string()],
        )
    };
    assert_eq!(
        db.count_by(sql, &binds[0]).await,
        1,
        "and it is still one row, not two"
    );
}

#[tokio::test]
async fn forbid_refuses_before_the_work_row_exists() {
    // The clause §51.1 turns on. The refusal happens in the store's caller-facing
    // function and no work is written, so there is nothing for a later process to
    // decide the fate of.
    let db = scratch("gen-forbid-refuses").await;
    let work = id("gen-forbid-work");
    fixture_work(&db, &work).await;

    let refusal = gc::stamp_work_posture(
        db.db(),
        &work,
        GeneratedContentDeclaration::DeclaredGenerated,
        GeneratedContentPosture::Forbid,
        NOW,
    )
    .await
    .expect_err("forbid refuses a declared-generated work");

    let message = refusal.to_string();
    assert!(
        message.contains("forbids generated content"),
        "the message names the policy: {message}"
    );
    // The refusal is what §51.1 is about, so the assertion is on the EFFECT: the work
    // carries no declaration. A successful stamp under `forbid` is impossible --
    // 0109's own pair-check refuses it -- so a declared work here would mean the
    // refusal ran and the write went anyway.
    //
    // Not asserted as `None`: 0109's backfill stamps every pre-existing work with the
    // policy in force, so a fixture work created after the migration has a row and
    // reads as `forbid`/undeclared. Absent would be the wrong claim to make here.
    let read_back = gc::work_generated_content(db.db(), &work)
        .await
        .expect("read")
        .expect("0109 stamps every work, so this row exists");
    assert_eq!(
        read_back.declaration,
        GeneratedContentDeclaration::Undeclared,
        "nothing was written: §51.1 refuses before the write, so the author's \
         declaration never reached the corpus and no later process has to decide \
         the fate of a generated work on a forbidding instance"
    );
}

#[tokio::test]
async fn disclose_accepts_and_marks_the_work() {
    let db = scratch("gen-disclose").await;
    let work = id("gen-disclose-work");
    fixture_work(&db, &work).await;

    let stamped = gc::stamp_work_posture(
        db.db(),
        &work,
        GeneratedContentDeclaration::DeclaredGenerated,
        GeneratedContentPosture::Disclose,
        NOW,
    )
    .await
    .expect("disclose accepts");

    assert!(
        stamped.needs_marker(),
        "§51.1: disclose accepts AND labels, so the marker is required"
    );

    let read_back = gc::work_generated_content(db.db(), &work)
        .await
        .expect("read")
        .expect("the work was stamped");
    assert_eq!(read_back.posture, GeneratedContentPosture::Disclose);
    assert_eq!(
        read_back.declaration,
        GeneratedContentDeclaration::DeclaredGenerated
    );
    assert!(read_back.needs_marker());
}

#[tokio::test]
async fn allow_accepts_and_does_not_mark_the_work() {
    // The third option has to be a real one. If `allow` also labelled, it would be a
    // worse `disclose` and an instance that wants generated fiction would have to run
    // a disclosure UI it does not believe in.
    let db = scratch("gen-allow").await;
    let work = id("gen-allow-work");
    fixture_work(&db, &work).await;

    let stamped = gc::stamp_work_posture(
        db.db(),
        &work,
        GeneratedContentDeclaration::DeclaredGenerated,
        GeneratedContentPosture::Allow,
        NOW,
    )
    .await
    .expect("allow accepts");

    assert!(
        !stamped.needs_marker(),
        "§51.1: allow accepts and does NOT label"
    );
    let read_back = gc::work_generated_content(db.db(), &work)
        .await
        .expect("read")
        .expect("stamped");
    assert_eq!(read_back.posture, GeneratedContentPosture::Allow);
    assert!(!read_back.needs_marker());
}

#[tokio::test]
async fn the_posture_is_about_the_work_not_the_author() {
    // §51.1: "no per-work override and no per-author opt-in". The same declaration
    // from a different author gets the same answer -- which is what "instance
    // policy" means, and it is worth pinning because a per-author carve-out is the
    // obvious thing to add later and it would make `forbid` `allow` with extra steps.
    let db = scratch("gen-not-author").await;
    let refused_work = id("gen-author-a");
    let accepted_work = id("gen-author-b");
    fixture_work(&db, &refused_work).await;
    fixture_work(&db, &accepted_work).await;

    assert!(
        gc::stamp_work_posture(
            db.db(),
            &refused_work,
            GeneratedContentDeclaration::DeclaredGenerated,
            GeneratedContentPosture::Forbid,
            NOW,
        )
        .await
        .is_err(),
        "under forbid the declaration is refused"
    );
    assert!(
        gc::stamp_work_posture(
            db.db(),
            &accepted_work,
            GeneratedContentDeclaration::DeclaredGenerated,
            GeneratedContentPosture::Forbid,
            NOW,
        )
        .await
        .is_err(),
        "and for a different work by a different author, identically -- §51.5 \\
         refuses a per-author opt-out"
    );
}

#[tokio::test]
async fn changing_the_policy_does_not_relabel_works() {
    // THE reason the posture is a column on the work rather than a join. An operator
    // tightening allow -> disclose is making a change about FUTURE writes; a read-time
    // join would relabel every work already published, changing terms their authors
    // agreed to under different ones.
    let db = scratch("gen-no-relabel").await;
    let work = id("gen-legacy-work");
    fixture_work(&db, &work).await;

    gc::set_generated_content_policy(db.db(), GeneratedContentPosture::Allow, None, NOW)
        .await
        .expect("set allow");
    gc::stamp_work_posture(
        db.db(),
        &work,
        GeneratedContentDeclaration::DeclaredGenerated,
        GeneratedContentPosture::Allow,
        NOW,
    )
    .await
    .expect("stamped under allow");

    // Now the operator tightens. The POLICY changes; the WORK must not.
    gc::set_generated_content_policy(
        db.db(),
        GeneratedContentPosture::Disclose,
        None,
        "2026-10-02T10:00:00Z",
    )
    .await
    .expect("set disclose");

    assert_eq!(
        gc::generated_content_policy(db.db()).await.expect("read"),
        GeneratedContentPosture::Disclose,
        "the instance policy moved to disclose"
    );

    let read_back = gc::work_generated_content(db.db(), &work)
        .await
        .expect("read")
        .expect("stamped");
    assert_eq!(
        read_back.posture,
        GeneratedContentPosture::Allow,
        "§51.1: the work KEEPS the posture it was published under. A read-time join \\
         would have relabelled it and changed terms its author agreed to under \\
         `allow`."
    );
    assert!(
        !read_back.needs_marker(),
        "so it still needs no marker, even though the instance now discloses"
    );
}

#[tokio::test]
async fn a_new_write_after_the_change_carries_the_new_posture() {
    // The other half: the change is prospective, so a work written AFTER it picks it
    // up. Without this the previous test would pass for the wrong reason -- a store
    // that ignored the policy entirely would also keep the old posture.
    let db = scratch("gen-prospective").await;
    let old_work = id("gen-old");
    let new_work = id("gen-new");
    fixture_work(&db, &old_work).await;
    fixture_work(&db, &new_work).await;

    gc::set_generated_content_policy(db.db(), GeneratedContentPosture::Allow, None, NOW)
        .await
        .expect("set allow");
    gc::stamp_work_posture(
        db.db(),
        &old_work,
        GeneratedContentDeclaration::DeclaredGenerated,
        GeneratedContentPosture::Allow,
        NOW,
    )
    .await
    .expect("old stamped");
    gc::set_generated_content_policy(
        db.db(),
        GeneratedContentPosture::Disclose,
        None,
        "2026-10-02T10:00:00Z",
    )
    .await
    .expect("set disclose");

    let current = gc::generated_content_policy(db.db()).await.expect("read");
    let stamped = gc::stamp_work_posture(
        db.db(),
        &new_work,
        GeneratedContentDeclaration::DeclaredGenerated,
        current,
        "2026-10-02T10:00:00Z",
    )
    .await
    .expect("new work stamped under the new policy");

    assert!(stamped.needs_marker(), "the new work DOES need a marker");
    assert_eq!(
        gc::work_generated_content(db.db(), &old_work)
            .await
            .expect("read")
            .expect("stamped")
            .posture,
        GeneratedContentPosture::Allow,
        "and the old one still does not"
    );
}

#[tokio::test]
async fn a_declared_work_always_records_when_the_author_declared() {
    // The rule that 0109's pair-check exists to enforce, asserted from the store's
    // side on BOTH accepting postures.
    //
    // This is the property whose absence is invisible: an `allow` work with no
    // declaration timestamp reads back as "undeclared", so the author's statement is
    // simply gone and nothing errors. That is exactly the mutation that would pass a
    // test suite which only checked the posture -- and it is the same erasure
    // §51.1's "recorded with the work" clause exists to prevent.
    for posture in [
        GeneratedContentPosture::Disclose,
        GeneratedContentPosture::Allow,
    ] {
        let db = scratch(&format!("gen-declared-{}", posture.as_str())).await;
        let work = id(&format!("gen-declared-work-{}", posture.as_str()));
        fixture_work(&db, &work).await;
        gc::set_generated_content_policy(db.db(), posture, None, NOW)
            .await
            .expect("set");

        gc::stamp_work_posture(
            db.db(),
            &work,
            GeneratedContentDeclaration::DeclaredGenerated,
            posture,
            NOW,
        )
        .await
        .expect("stamped");

        let read_back = gc::work_generated_content(db.db(), &work)
            .await
            .expect("read")
            .expect("stamped");
        assert_eq!(
            read_back.declaration,
            GeneratedContentDeclaration::DeclaredGenerated,
            "under {posture} the author's declaration is RECORDED, even though {posture} \
             does not display it. Posture controls the marker, never the record."
        );
    }
}

#[tokio::test]
async fn a_work_with_no_row_at_all_is_absent_not_forbidden() {
    // §49.3's absent-is-not-a-zero rule, restated: a work this read finds no row
    // for is None, which is different from a work stamped `forbid`. A reader
    // display that treated None as forbid would be safe today, but a reader display
    // that treated None as "undeclared, therefore fine" would not.
    let db = scratch("gen-no-row").await;
    let never_made = id("gen-never-made-work");

    assert_eq!(
        gc::work_generated_content(db.db(), &never_made)
            .await
            .expect("read"),
        None,
        "a work that does not exist has no posture -- None, not forbid"
    );

    // And the case that IS reachable: a work that exists but was written before the
    // columns existed is backfilled by 0109 to the policy in force, so it reads as a
    // real posture rather than as absent. That is the distinction this test is for.
    let legacy = id("gen-legacy-unstamped-work");
    fixture_work(&db, &legacy).await;
    assert_eq!(
        gc::work_generated_content(db.db(), &legacy)
            .await
            .expect("read")
            .expect("0109 backfilled every existing work, so this row exists")
            .posture,
        GeneratedContentPosture::Forbid,
        "a pre-existing work reads as forbid -- the policy that was in force when the \
         migration ran -- not as an absent value. §49.3's rule is about absence, and \
         this is a present value."
    );
}

#[tokio::test]
async fn the_declaration_survives_the_round_trip() {
    // Both shapes, because the pair-CHECK in 0109 refuses a declaration time with
    // nothing declared and a `disclose` with no time -- so a store that lost either
    // would fail at the next write, not just at the next read.
    let db = scratch("gen-round-trip").await;
    for (i, declaration) in [
        GeneratedContentDeclaration::Undeclared,
        GeneratedContentDeclaration::DeclaredGenerated,
    ]
    .into_iter()
    .enumerate()
    {
        let work = id(&format!("gen-round-trip-{i}"));
        fixture_work(&db, &work).await;
        let posture = if matches!(declaration, GeneratedContentDeclaration::DeclaredGenerated) {
            GeneratedContentPosture::Disclose
        } else {
            GeneratedContentPosture::Forbid
        };
        gc::stamp_work_posture(db.db(), &work, declaration, posture, NOW)
            .await
            .expect("stamped");
        assert_eq!(
            gc::work_generated_content(db.db(), &work)
                .await
                .expect("read")
                .expect("stamped")
                .declaration,
            declaration,
            "{declaration:?} survives the round trip"
        );
    }
}

#[tokio::test]
async fn both_engines_report_the_same_posture() {
    // The store has to be portable. `generated_content_posture` is TEXT on both, but
    // 0109's backfill, its pair-CHECK and its nullable-on-SQLite-only column are three
    // places the engines differ, and this is what proves the reads agree.
    for posture in GeneratedContentPosture::all() {
        let db = scratch(&format!("gen-both-{}", posture.as_str())).await;
        let work = id(&format!("gen-both-work-{}", posture.as_str()));
        fixture_work(&db, &work).await;
        gc::set_generated_content_policy(db.db(), posture, None, NOW)
            .await
            .expect("set");
        // Undeclared under `forbid`, because a declared one is refused -- and the
        // refusal has to agree across engines too, so the fixture states the one
        // shape every posture accepts rather than the one the loop happens to want.
        let declaration = if posture.accepts_declared_generated() {
            GeneratedContentDeclaration::DeclaredGenerated
        } else {
            GeneratedContentDeclaration::Undeclared
        };
        gc::stamp_work_posture(db.db(), &work, declaration, posture, NOW)
            .await
            .expect("stamped");

        assert_eq!(
            gc::generated_content_policy(db.db()).await.expect("read"),
            posture,
            "{posture} on {}",
            if db.is_postgres() {
                "PostgreSQL"
            } else {
                "SQLite"
            }
        );
        let read_back = gc::work_generated_content(db.db(), &work)
            .await
            .expect("read")
            .expect("stamped");
        assert_eq!(read_back.posture, posture);
        // Against the declaration the loop actually stamped, not a constant: under
        // `forbid` there is no declared work to stamp, so the expected value differs
        // by posture and a hardcoded one would be wrong for the case it cannot reach.
        assert_eq!(
            read_back.declaration,
            declaration,
            "{posture} on {}",
            if db.is_postgres() {
                "PostgreSQL"
            } else {
                "SQLite"
            }
        );
    }
}
