//! Acceptance: the canon-agnostic class persists, reads back, and stays a class
//! (spec §50.2, §50.3, §49.3; M45-31).
//!
//! `crates/domain/src/canon.rs` classifies the text and `migrations/0108` gives the
//! class a home. Neither of those is the thing that was missing: 0106 created the
//! table and NOTHING wrote to it or read from it, so §50.2's eligibility rule was
//! spec-only. These tests drive the store, because a class nothing can read is the
//! shape `docs/goal.md` names as a definition of not-complete.
//!
//! Every test runs on **both** engines, and that is not optional. §50.3 requires the
//! class be reproducible on both; `canon_dependent` is `INTEGER` on SQLite and
//! `BOOLEAN` on PostgreSQL, so a shared decode compiles on the default test engine
//! and fails on the first row of the other one — the same shape of bug M45-24's
//! `kudos_reason` had.
//!
//! | clause | test |
//! |---|---|
//! | the class persists and reads back unchanged | `a_canon_agnostic_work_reads_back_identically` |
//! | a canon-dependent work is distinguishable from an unmeasured one | `a_canon_dependent_work_is_not_an_unmeasured_one` |
//! | absent is not canon-agnostic, at every level | `an_unmeasured_work_is_absent_not_canon_agnostic` |
//! | too-short is unclassified, not canon-agnostic | `a_work_too_short_to_classify_is_unclassified_not_agnostic` |
//! | a work with no text writes no row at all | `a_work_with_no_chapters_writes_no_row` |
//! | eligibility asks "known to be", not "not known to be otherwise" | `eligibility_distinguishes_unclassified_from_agnostic` |
//! | the class is measured from the work's own stored text | `the_class_is_measured_from_the_work_s_own_stored_chapters` |
//! | re-measuring replaces rather than duplicating | `a_recomputation_replaces_the_previous_row` |
//! | the reason survives the round trip | `the_unclassified_reason_survives_the_round_trip` |
//! | the measures survive the round trip, for an audit | `the_measures_survive_the_round_trip` |
//! | the two engines agree | `both_engines_report_the_same_class` |

use lorehaven_db::canon_agnostic::{self as canon_store, CanonVerdict};
use lorehaven_domain::canon::CanonClass;
use test_support::{id, scratch_dir, TestDb};

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

/// Run a statement with binds, on whichever backend is active, where one bind is an
/// integer rather than text.
///
/// Split out because `order_key` is BIGINT on PostgreSQL and INTEGER on SQLite:
/// binding a `String` produces `column "order_key" is of type bigint but expression
/// is of type text` (42804) on one engine and works perfectly on the other. A single
/// `exec_bound` that binds everything as text cannot satisfy both, and the error
/// names neither the fixture nor the dialect that caused it.
async fn exec_bound_mixed(db: &TestDb, sqlite_sql: &str, postgres_sql: &str, binds: &[Bind]) {
    // `Bind` is a tiny two-variant enum rather than a trait object: there are
    // exactly two kinds of bind in this suite and a trait would buy nothing.
    //
    // The binds are passed **already in positional order** and applied one at a
    // time. That is the whole point of the helper. An earlier version took
    // `&[String]` plus `&[i64]` and applied all the strings before the integers,
    // which is wrong whenever the integer is not the last parameter --
    // `chapters.order_key` is `$4`, in the middle. PostgreSQL binds by `$n`, so
    // that version sent a `String` to `$4` and produced
    //
    //     column "order_key" is of type bigint but expression is of type text
    //
    // which names the column and not the mistake, and cost a full round of
    // debugging for a two-line fix. Order is the caller's to get right, and making
    // it visible in a single list is how it gets got right.
    match db.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            let mut q = sqlx::query(sqlite_sql);
            for b in binds {
                q = match b {
                    Bind::Text(t) => q.bind(t.clone()),
                    Bind::Int(i) => q.bind(*i),
                };
            }
            q.execute(db.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("sqlite statement");
        }
        lorehaven_db::Backend::Postgres => {
            let mut q = sqlx::query(postgres_sql);
            for b in binds {
                q = match b {
                    Bind::Text(t) => q.bind(t.clone()),
                    Bind::Int(i) => q.bind(*i),
                };
            }
            q.execute(db.db().postgres_pool().expect("postgres"))
                .await
                .expect("postgres statement");
        }
    }
}

/// One bound parameter, in the position the statement expects it.
///
/// `Int` exists because `chapters.order_key` is BIGINT on PostgreSQL and INTEGER
/// on SQLite: binding a string is a type error on one engine and accepted on the
/// other, and the error names the column rather than the fixture.
#[derive(Debug, Clone)]
enum Bind {
    Text(String),
    Int(i64),
}

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

    exec_bound(
        db,
        "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
        "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5)",
        &[work_id.to_string(), pseud.clone(), "A Work For Coordinates".into(), now.clone(), now.clone()],
    )
    .await;

    // The pseud is returned because `chapter_revisions.created_by_pseud_id` is
    // NOT NULL and references it, so a chapter fixture cannot be written without
    // knowing it. Returning it beats looking it up again from a deterministic hash.
    pseud
}

/// `chapters` + `chapter_revisions` rows whose prose is `text`.
///
/// The generated-prose helper above cannot express this suite's subject, which is
/// proper nouns: it emits `w0 w1 w2 ...`, which has none. So this takes the prose
/// directly, and the caller builds it.
async fn fixture_chapter_with_text(
    db: &TestDb,
    work_id: &str,
    author_pseud: &str,
    text: &str,
    word_count: usize,
) {
    let now = "2026-10-01T12:00:00Z".to_string();
    let chapter_id = id(&format!("canon-chapter-{work_id}"));
    let revision_id = id(&format!("canon-revision-{work_id}"));

    exec_bound_mixed(
        db,
        "INSERT INTO chapters (id, work_id, title, order_key, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
        "INSERT INTO chapters (id, work_id, title, order_key, created_at, updated_at) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)",
        &[
            Bind::Text(chapter_id.clone()),
            Bind::Text(work_id.to_string()),
            Bind::Text("Chapter 0".to_string()),
            Bind::Int(0),
            Bind::Text(now.clone()),
            Bind::Text(now.clone()),
        ],
    )
    .await;

    exec_bound_mixed(
        db,
        "INSERT INTO chapter_revisions \
         (id, chapter_id, revision_number, document_json, sanitized_html, \
          plain_text, word_count, created_by_pseud_id, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO chapter_revisions \
         (id, chapter_id, revision_number, document_json, sanitized_html, \
          plain_text, word_count, created_by_pseud_id, created_at) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6, $7, $8::uuid, $9)",
        &[
            Bind::Text(revision_id),
            Bind::Text(chapter_id),
            Bind::Int(1),
            Bind::Text("{\"type\":\"doc\",\"content\":[]}".to_string()),
            Bind::Text(format!("<p>{}</p>", text)),
            Bind::Text(text.to_string()),
            Bind::Int(word_count as i64),
            Bind::Text(author_pseud.to_string()),
            Bind::Text(now.clone()),
        ],
    )
    .await;
}

/// Prose of `words` tokens, with `names` mid-sentence proper nouns.
///
/// Terminators matter: `classify` excludes a capital that starts a sentence, so a
/// fixture without them measures zero unexplained names however many capitals it
/// contains. Slot 0 of each four-token group carries no terminator, which is what
/// makes slot 1 the mid-sentence slot.
fn prose_with_names(words: usize, names: usize) -> String {
    let mut left = names;
    let mut out = String::new();
    for i in 0..words {
        out.push_str(&match i % 4 {
            0 => format!("word{i} "),
            1 if left > 0 => {
                left -= 1;
                format!("Name{i}. ")
            }
            _ => format!("word{i}. "),
        });
    }
    out
}

/// Count `canon_agnostic_works` rows for a work, on either engine.
async fn count_canon_rows(db: &TestDb, work_id: &str) -> i64 {
    if db.is_postgres() {
        db.count_by(
            "SELECT count(*) FROM canon_agnostic_works WHERE work_id = $1::uuid",
            work_id,
        )
        .await
    } else {
        db.count_by(
            "SELECT count(*) FROM canon_agnostic_works WHERE work_id = ?",
            work_id,
        )
        .await
    }
}

#[tokio::test]
async fn a_canon_agnostic_work_reads_back_identically() {
    let db = scratch("canon-agnostic").await;
    let work = id("canon-agnostic-work");
    let pseud = fixture_work(&db, &work).await;
    // Above the threshold with no names in it: measured, and canon-agnostic.
    let text = prose_with_names(1000, 0);
    fixture_chapter_with_text(&db, &work, &pseud, &text, 1000).await;

    let stored = canon_store::measure_work_and_store(db.db(), &work, "2026-10-01T12:00:00Z")
        .await
        .expect("measure")
        .expect("a work with chapters is measured");

    assert_eq!(
        stored.verdict.class(),
        Some(CanonClass::CanonAgnostic),
        "plain prose has no unexplained proper nouns"
    );

    let read_back = canon_store::stored_canon(db.db(), &work)
        .await
        .expect("read")
        .expect("a measured work has a row");
    assert_eq!(
        read_back.verdict, stored.verdict,
        "the class survives the round trip"
    );
    assert_eq!(read_back.text_version, stored.text_version);
}

#[tokio::test]
async fn a_canon_dependent_work_is_not_an_unmeasured_one() {
    // The distinction 0108 exists for. Before it, the table could only record "this
    // work is canon-agnostic", so a canon-dependent work and a work nobody had
    // measured were the same absence -- and a reader could not tell a work that was
    // ruled out from a work that was never looked at.
    let db = scratch("canon-dependent").await;
    let work = id("canon-dependent-work");
    let pseud = fixture_work(&db, &work).await;
    let text = prose_with_names(1000, 100);
    fixture_chapter_with_text(&db, &work, &pseud, &text, 1000).await;

    let stored = canon_store::measure_work_and_store(db.db(), &work, "2026-10-01T12:00:00Z")
        .await
        .expect("measure")
        .expect("measured");

    assert_eq!(stored.verdict.class(), Some(CanonClass::CanonDependent));
    assert!(canon_store::is_canon_dependent(db.db(), &work)
        .await
        .expect("eligible"));
    assert!(!canon_store::is_canon_agnostic(db.db(), &work)
        .await
        .expect("eligible"));
}

#[tokio::test]
async fn an_unmeasured_work_is_absent_not_canon_agnostic() {
    let db = scratch("canon-absent").await;
    let work = id("canon-absent-work");
    fixture_work(&db, &work).await;

    assert_eq!(
        canon_store::stored_canon(db.db(), &work)
            .await
            .expect("read"),
        None,
        "no row means nobody measured it"
    );
    assert!(
        !canon_store::is_canon_agnostic(db.db(), &work)
            .await
            .expect("eligible"),
        "§49.3: an absent value is not a zero, and not a class either"
    );
    assert!(!canon_store::is_canon_dependent(db.db(), &work)
        .await
        .expect("eligible"));
}

#[tokio::test]
async fn a_work_too_short_to_classify_is_unclassified_not_agnostic() {
    // Below §50.2's threshold there is no answer, and the answer is NOT "yes it is
    // canon-agnostic" -- that would admit a work into fandom-blind discovery on the
    // strength of a text too short to have said anything.
    let db = scratch("canon-too-short").await;
    let work = id("canon-too-short-work");
    let pseud = fixture_work(&db, &work).await;
    let text = prose_with_names(100, 0);
    fixture_chapter_with_text(&db, &work, &pseud, &text, 100).await;

    let stored = canon_store::measure_work_and_store(db.db(), &work, "2026-10-01T12:00:00Z")
        .await
        .expect("measure")
        .expect("measured");

    assert_eq!(
        stored.verdict,
        CanonVerdict::Unclassified {
            reason: canon_store::TOO_SHORT.into()
        },
        "too little text is unclassified, with the reason recorded"
    );
    assert_eq!(stored.verdict.class(), None);
}

#[tokio::test]
async fn a_work_with_no_chapters_writes_no_row() {
    // `measure_work_and_store` returns None for a work with no text rather than
    // recording "unclassified". Storing a reason here would be a claim that somebody
    // looked at a work that has nothing to look at.
    let db = scratch("canon-no-chapters").await;
    let work = id("canon-no-chapters-work");
    fixture_work(&db, &work).await;

    let stored = canon_store::measure_work_and_store(db.db(), &work, "2026-10-01T12:00:00Z")
        .await
        .expect("measure");

    assert_eq!(stored, None, "nothing was classified, so nothing is stored");
    assert_eq!(
        count_canon_rows(&db, &work).await,
        0,
        "and specifically: not one row claiming unclassified"
    );
}

#[tokio::test]
async fn eligibility_distinguishes_unclassified_from_agnostic() {
    // Both readings of "is this eligible?" must be false for a work nobody could
    // classify, and the two false reasons are different. If a future change made
    // `is_canon_agnostic` answer "true unless known canon-dependent", the
    // too-short work would enter fandom-blind discovery.
    let db = scratch("canon-eligibility").await;
    let short = id("canon-eligibility-short");
    let short_pseud = fixture_work(&db, &short).await;
    let text = prose_with_names(100, 0);
    fixture_chapter_with_text(&db, &short, &short_pseud, &text, 100).await;
    canon_store::measure_work_and_store(db.db(), &short, "2026-10-01T12:00:00Z")
        .await
        .expect("measure")
        .expect("measured");

    assert!(!canon_store::is_canon_agnostic(db.db(), &short)
        .await
        .expect("eligible"));
    assert!(!canon_store::is_canon_dependent(db.db(), &short)
        .await
        .expect("eligible"));
}

#[tokio::test]
async fn the_class_is_measured_from_the_work_s_own_stored_chapters() {
    // The gap this closes. `classify` is a pure function over text, so a test that
    // only called it proved nothing about the store; the point is that the TEXT IS
    // READ FROM THE DATABASE, which is why the fixture writes prose and the test
    // never mentions a Corpus.
    let db = scratch("canon-from-chapters").await;
    let work = id("canon-from-chapters-work");
    let pseud = fixture_work(&db, &work).await;
    let text = prose_with_names(1000, 60);
    fixture_chapter_with_text(&db, &work, &pseud, &text, 1000).await;

    let stored = canon_store::measure_work_and_store(db.db(), &work, "2026-10-01T12:00:00Z")
        .await
        .expect("measure")
        .expect("measured");

    let measures = stored
        .verdict
        .measures()
        .expect("a classified work carries its measures");
    assert_eq!(
        measures.unexplained_names, 60,
        "all sixty planted names are unexplained: each is capitalised, mid-sentence, \
         and the fixture gives no lowercase form to establish any of them"
    );
    assert_eq!(
        measures.word_count, 1000,
        "the count is the stored text's own"
    );
    assert!(
        (measures.density - 0.06).abs() < 1e-9,
        "60 in 1000 is 0.06, and the density is stored rather than recomputed"
    );
}

#[tokio::test]
async fn a_recomputation_replaces_the_previous_row() {
    // Upsert, so a backfill re-run after a crash is safe: one row, the newest
    // values. Two rows would be two opinions about the same work.
    let db = scratch("canon-recompute").await;
    let work = id("canon-recompute-work");
    let pseud = fixture_work(&db, &work).await;
    let text = prose_with_names(1000, 0);
    fixture_chapter_with_text(&db, &work, &pseud, &text, 1000).await;

    canon_store::measure_work_and_store(db.db(), &work, "2026-10-01T12:00:00Z")
        .await
        .expect("first measure")
        .expect("measured");
    let second = canon_store::measure_work_and_store(db.db(), &work, "2026-10-02T12:00:00Z")
        .await
        .expect("second measure")
        .expect("measured");

    assert_eq!(
        count_canon_rows(&db, &work).await,
        1,
        "re-measuring replaces the row rather than adding a second"
    );
    let read_back = canon_store::stored_canon(db.db(), &work)
        .await
        .expect("read")
        .expect("a row");
    assert_eq!(
        read_back.declared_at, second.declared_at,
        "the newest measurement wins"
    );
}

#[tokio::test]
async fn a_recomputation_can_change_the_class() {
    // The recompute that matters: same work, longer text, different answer. If the
    // write path could only ever set "canon-agnostic", this would be impossible.
    let db = scratch("canon-class-flip").await;
    let work = id("canon-class-flip-work");
    let pseud = fixture_work(&db, &work).await;
    let clean = prose_with_names(1000, 0);
    fixture_chapter_with_text(&db, &work, &pseud, &clean, 1000).await;
    let first = canon_store::measure_work_and_store(db.db(), &work, "2026-10-01T12:00:00Z")
        .await
        .expect("first")
        .expect("measured");
    assert_eq!(first.verdict.class(), Some(CanonClass::CanonAgnostic));

    // A second chapter full of names changes the answer for the same work.
    let dense = prose_with_names(1000, 400);
    let chapter = id(&format!("canon-flip-chapter-{work}"));
    exec_bound_mixed(
        &db,
        "INSERT INTO chapters (id, work_id, title, order_key, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
        "INSERT INTO chapters (id, work_id, title, order_key, created_at, updated_at) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)",
        &[
            Bind::Text(chapter),
            Bind::Text(work.clone()),
            Bind::Text("Chapter 1".into()),
            Bind::Int(1),
            Bind::Text("2026-10-01T12:00:00Z".into()),
            Bind::Text("2026-10-01T12:00:00Z".into()),
        ],
    )
    .await;
    let rev = id(&format!("canon-flip-revision-{work}"));
    exec_bound_mixed(
        &db,
        "INSERT INTO chapter_revisions (id, chapter_id, revision_number, document_json, \
         sanitized_html, plain_text, word_count, created_by_pseud_id, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO chapter_revisions (id, chapter_id, revision_number, document_json, \
         sanitized_html, plain_text, word_count, created_by_pseud_id, created_at) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6, $7, $8::uuid, $9)",
        &[
            Bind::Text(rev),
            Bind::Text(id(&format!("canon-flip-chapter-{work}"))),
            Bind::Int(1),
            Bind::Text("{\"type\":\"doc\",\"content\":[]}".into()),
            Bind::Text(format!("<p>{dense}</p>")),
            Bind::Text(dense.clone()),
            Bind::Int(1000),
            Bind::Text(pseud.clone()),
            Bind::Text("2026-10-01T12:00:00Z".into()),
        ],
    )
    .await;

    let second = canon_store::measure_work_and_store(db.db(), &work, "2026-10-02T12:00:00Z")
        .await
        .expect("second")
        .expect("measured");
    assert_eq!(
        second.verdict.class(),
        Some(CanonClass::CanonDependent),
        "400 unexplained names across 2000 words is 0.2, far over the 2% ceiling"
    );
    assert_eq!(
        count_canon_rows(&db, &work).await,
        1,
        "the class was replaced, not added"
    );
}

#[tokio::test]
async fn the_unclassified_reason_survives_the_round_trip() {
    let db = scratch("canon-reason").await;
    let work = id("canon-reason-work");
    let pseud = fixture_work(&db, &work).await;
    let text = prose_with_names(100, 0);
    fixture_chapter_with_text(&db, &work, &pseud, &text, 100).await;
    canon_store::measure_work_and_store(db.db(), &work, "2026-10-01T12:00:00Z")
        .await
        .expect("measure")
        .expect("measured");

    let read_back = canon_store::stored_canon(db.db(), &work)
        .await
        .expect("read")
        .expect("a row");
    assert_eq!(
        read_back.verdict,
        CanonVerdict::Unclassified {
            reason: canon_store::TOO_SHORT.into()
        },
        "the reason is stored, so a reader can tell unmeasured from too-short"
    );
    assert_eq!(
        read_back.verdict.measures(),
        None,
        "an unclassified row carries no measures: 0108 pairs the two"
    );
}

#[tokio::test]
async fn the_measures_survive_the_round_trip() {
    // The measures are stored so the verdict can be AUDITED: §50.3 wants the class
    // reproducible from the text, which means somebody has to be able to read the
    // numbers back and compare.
    let db = scratch("canon-measures").await;
    let work = id("canon-measures-work");
    let pseud = fixture_work(&db, &work).await;
    let text = prose_with_names(1000, 30);
    fixture_chapter_with_text(&db, &work, &pseud, &text, 1000).await;
    let stored = canon_store::measure_work_and_store(db.db(), &work, "2026-10-01T12:00:00Z")
        .await
        .expect("measure")
        .expect("measured");

    let read_back = canon_store::stored_canon(db.db(), &work)
        .await
        .expect("read")
        .expect("a row");
    assert_eq!(
        read_back.verdict.measures(),
        stored.verdict.measures(),
        "the audit trail is the stored numbers, not a re-derivation"
    );
    let m = read_back.verdict.measures().expect("measures");
    assert_eq!(m.unexplained_names, 30);
    assert_eq!(m.word_count, 1000);
    assert!((m.density - 0.03).abs() < 1e-9, "30 in 1000 is 0.03");
}

#[tokio::test]
async fn both_engines_report_the_same_class() {
    // The point of running on both: `canon_dependent` is INTEGER on SQLite and
    // BOOLEAN on PostgreSQL. A decode shared across the two would work here on
    // SQLite and fail on the first row of the PostgreSQL leg.
    for (names, expected) in [
        (0usize, CanonClass::CanonAgnostic),
        (100, CanonClass::CanonDependent),
    ] {
        let db = scratch(&format!("canon-both-{names}")).await;
        let work = id(&format!("canon-both-work-{names}"));
        let pseud = fixture_work(&db, &work).await;
        let text = prose_with_names(1000, names);
        fixture_chapter_with_text(&db, &work, &pseud, &text, 1000).await;

        let stored = canon_store::measure_work_and_store(db.db(), &work, "2026-10-01T12:00:00Z")
            .await
            .expect("measure")
            .expect("measured");
        let read_back = canon_store::stored_canon(db.db(), &work)
            .await
            .expect("read")
            .expect("a row");

        assert_eq!(
            read_back.verdict.class(),
            Some(expected),
            "{names} planted names"
        );
        assert_eq!(
            read_back.verdict.measures(),
            stored.verdict.measures(),
            "the flag decodes the same way on {}",
            if db.is_postgres() {
                "PostgreSQL"
            } else {
                "SQLite"
            }
        );
    }
}
