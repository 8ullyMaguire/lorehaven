//! Acceptance: work coordinates persist and read back (spec §49.3, §49.7, §49.8,
//! M45-14; plan `m45-gaps-adoption.md` Phase 2).
//!
//! The four measures were computed by `crates/domain/src/coordinates.rs` and
//! unit-tested there. **Nothing stored them.** These tests drive the store, because
//! a measurement nothing can read is the shape `docs/goal.md` names as a definition
//! of not-complete.
//!
//! Every test here runs on **both** engines. That is not optional: SQLite is
//! dynamically typed, so the `::uuid` bind casts and the `REAL` versus
//! `DOUBLE PRECISION` decode differences are all invisible on the default test
//! engine and fail only on PostgreSQL.
//!
//! | clause | test |
//! |---|---|
//! | coordinates persist and read back unchanged | `a_measured_work_reads_back_identically` |
//! | absent is not a zero, at every level | `an_unmeasurable_work_is_absent_not_zero` |
//! | a single-chapter work is measured with no spread | `a_single_chapter_work_is_measured_with_no_spread` |
//! | re-measuring replaces rather than duplicating | `a_recomputation_replaces_the_previous_row` |
//! | many works, one query, unmeasurable omitted | `a_batch_read_returns_only_measured_works` |
//! | the reason survives the round trip | `the_unmeasurable_reason_survives_the_round_trip` |
//! | coordinates read from real chapter text | `coordinates_can_be_measured_from_stored_chapters` |
//! | a stale row can be withdrawn | `a_cleared_work_is_absent_rather_than_stale` |
//! | the two engines agree | `both_engines_report_the_same_coordinates` |

use lorehaven_db::work_coordinates as wc;
use lorehaven_domain::coordinates::{
    ChapterText, Coordinates, Corpus, Unmeasurable, WorkCoordinates, MIN_MEASURABLE_WORDS,
};
use test_support::{id, scratch_dir, TestDb};

/// A measurable corpus: `chapters` chapters of `words` words each.
fn corpus_of(chapters: usize, words: usize, dialogue: usize) -> Corpus {
    Corpus {
        chapters: (0..chapters)
            .map(|_| ChapterText {
                word_count: words,
                plain_text: (0..(words / 20))
                    .map(|_| {
                        (0..20)
                            .map(|i| format!("w{i}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                            + "."
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            })
            .collect(),
        dialogue_words: dialogue,
    }
}

/// The coordinates a fixture should produce, computed by the domain layer.
///
/// Deliberately *not* hand-written numbers: the point is that the store is a
/// faithful round trip, and a hardcoded expectation would test the fixture instead
/// of the persistence.
fn expected_measured(chapters: usize, words: usize, dialogue: usize) -> WorkCoordinates {
    let outcome = lorehaven_domain::coordinates::coordinates(&corpus_of(chapters, words, dialogue));
    match outcome {
        Coordinates::Measured(c) => c,
        other => panic!("fixture must be measurable, got {other:?}"),
    }
}

// ------------------------------------------------------------------ round trips

#[tokio::test]
async fn a_measured_work_reads_back_identically() {
    let db = scratch("coord_round_trip").await;

    let work = id("coord-round-trip");
    fixture_work(&db, &work).await;
    let corpus = corpus_of(3, 800, 240);
    let expected = expected_measured(3, 800, 240);

    wc::measure_and_store(db.db(), &work, &corpus, 7, "2026-10-01T12:00:00Z")
        .await
        .expect("store coordinates");

    let read = wc::measured_coordinates(db.db(), &work)
        .await
        .expect("read coordinates")
        .expect("the work was just measured, so it must have coordinates");

    assert_eq!(
        read.sentence_length_variance, expected.sentence_length_variance,
        "sentence_length_variance must survive the round trip exactly"
    );
    assert_eq!(read.dialogue_ratio, expected.dialogue_ratio);
    assert_eq!(read.vocabulary_richness, expected.vocabulary_richness);
    assert_eq!(
        read.chapter_length_spread, expected.chapter_length_spread,
        "the chapter spread must survive as an Option, not collapse to a value"
    );
    // The counts too: they are the evidence that the row describes *this* text.
    assert_eq!(read.word_count, expected.word_count);
    assert_eq!(read.sentence_count, expected.sentence_count);

    db.cleanup().await;
}

#[tokio::test]
async fn an_unmeasurable_work_is_absent_not_zero() {
    let db = scratch("coord_unmeasurable").await;

    let work = id("coord-unmeasurable");
    fixture_work(&db, &work).await;

    // Below MIN_MEASURABLE_WORDS, so §49.3 says it has no coordinates.
    let short = corpus_of(1, 100, 0);
    assert!(
        short.word_count() < MIN_MEASURABLE_WORDS,
        "the fixture must actually be under the threshold: {} words",
        short.word_count()
    );

    wc::measure_and_store(db.db(), &work, &short, 1, "2026-10-01T12:00:00Z")
        .await
        .expect("store an unmeasurable outcome");

    // The read that hands out coordinates must return None, NOT a zeroed struct.
    assert!(
        wc::measured_coordinates(db.db(), &work)
            .await
            .expect("read")
            .is_none(),
        "an unmeasurable work must read as absent; a zeroed coordinate would mean \\
         'uniformly flat prose' and would rank against short-but-sharp works (§49.3)"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn a_single_chapter_work_is_measured_with_no_spread() {
    let db = scratch("coord_single_chapter").await;

    let work = id("coord-single-chapter");
    fixture_work(&db, &work).await;

    // One chapter, so there is no chapter-length *distribution* — but the work IS
    // measurable and its other three coordinates are real.
    let corpus = corpus_of(1, 1200, 300);
    let expected = expected_measured(1, 1200, 300);
    assert_eq!(
        expected.chapter_length_spread, None,
        "one chapter has no distribution; the fixture must actually be single-chapter"
    );

    wc::measure_and_store(db.db(), &work, &corpus, 2, "2026-10-01T12:00:00Z")
        .await
        .expect("store");

    let read = wc::measured_coordinates(db.db(), &work)
        .await
        .expect("read")
        .expect("a single-chapter work is still measured");

    assert_eq!(
        read.chapter_length_spread, None,
        "the absent spread must survive storage as None, not become 0.0 -- which \\
         would assert 'evenly sized chapters' about a work that has exactly one"
    );
    // And the three that do apply are present and correct.
    assert_eq!(read.dialogue_ratio, expected.dialogue_ratio);
    assert_eq!(read.vocabulary_richness, expected.vocabulary_richness);
    assert!(read.sentence_length_variance >= 0.0);

    db.cleanup().await;
}

#[tokio::test]
async fn a_recomputation_replaces_the_previous_row() {
    let db = scratch("coord_recompute").await;

    let work = id("coord-recompute");
    fixture_work(&db, &work).await;

    // Measure twice with different text. A second opinion must replace the first,
    // not accumulate beside it — that is what makes a backfill safe to re-run.
    let first = corpus_of(4, 800, 0);
    wc::measure_and_store(db.db(), &work, &first, 1, "2026-10-01T12:00:00Z")
        .await
        .expect("first measurement");

    let second = corpus_of(2, 800, 400);
    let expected_second = expected_measured(2, 800, 400);
    wc::measure_and_store(db.db(), &work, &second, 2, "2026-10-01T13:00:00Z")
        .await
        .expect("second measurement");

    assert_eq!(
        count_rows(&db, &work).await,
        1,
        "one work has one set of coordinates; a recompute must replace, not duplicate"
    );

    let read = wc::measured_coordinates(db.db(), &work)
        .await
        .expect("read")
        .expect("measured");
    assert_eq!(
        read.dialogue_ratio, expected_second.dialogue_ratio,
        "the read must return the SECOND measurement, not the first"
    );

    // And the version moved with it, so a stale row is detectable.
    let stored = wc::stored_coordinates(db.db(), &work)
        .await
        .expect("read stored")
        .expect("a row exists");
    assert_eq!(
        stored.text_version, 2,
        "text_version records which text was measured; it must advance"
    );
    assert_eq!(stored.computed_at, "2026-10-01T13:00:00Z");

    db.cleanup().await;
}

#[tokio::test]
async fn a_batch_read_returns_only_measured_works() {
    let db = scratch("coord_batch").await;

    let mut measured = Vec::new();
    let mut unmeasured = Vec::new();

    // Three measurable works.
    for i in 0..3 {
        let work = id(&format!("coord-batch-measured-{i}"));
        fixture_work(&db, &work).await;
        wc::measure_and_store(
            db.db(),
            &work,
            &corpus_of(3, 800, 100 * i),
            1,
            "2026-10-01T12:00:00Z",
        )
        .await
        .expect("store measured");
        measured.push(work);
    }
    // Two that are too short, and one never measured at all.
    for i in 0..2 {
        let work = id(&format!("coord-batch-short-{i}"));
        fixture_work(&db, &work).await;
        wc::measure_and_store(
            db.db(),
            &work,
            &corpus_of(1, 100, 0),
            1,
            "2026-10-01T12:00:00Z",
        )
        .await
        .expect("store unmeasurable");
        unmeasured.push(work);
    }
    let never = id("coord-batch-never");
    fixture_work(&db, &never).await;

    // Ask for all six, in one call, in the order a ranker would.
    let mut asked = measured.clone();
    asked.extend(unmeasured.clone());
    asked.push(never.clone());

    let batch = wc::measured_coordinates_for(db.db(), &asked)
        .await
        .expect("batch read");

    for work in &measured {
        assert!(
            batch.contains_key(work),
            "a measured work must be in the batch: {work}"
        );
    }
    for work in unmeasured.iter().chain(std::iter::once(&never)) {
        assert!(
            !batch.contains_key(work),
            "an unmeasurable work must be ABSENT from the batch, not present with \\
             zeroed coordinates -- that is the shape a ranker would misread as \\
             'flat prose': {work}"
        );
    }
    assert_eq!(
        batch.len(),
        measured.len(),
        "the batch must contain exactly the measured works"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn the_unmeasurable_reason_survives_the_round_trip() {
    let db = scratch("coord_reason").await;

    // Two different reasons, because "no row" and "measured as too short" and
    // "measured as having no text" are three facts and a caller asking *why* needs
    // the right one.
    for (label, corpus, expected_reason) in [
        ("too-short", corpus_of(1, 100, 0), Unmeasurable::TooShort),
        ("no-text", Corpus::default(), Unmeasurable::NoText),
    ] {
        let work = id(&format!("coord-reason-{label}"));
        fixture_work(&db, &work).await;

        wc::measure_and_store(db.db(), &work, &corpus, 1, "2026-10-01T12:00:00Z")
            .await
            .expect("store an unmeasurable outcome");

        let stored = wc::stored_coordinates(db.db(), &work)
            .await
            .expect("read stored")
            .expect("a row exists");
        assert_eq!(
            stored.coordinates,
            Coordinates::Unmeasurable(expected_reason),
            "the reason must survive storage; a caller asking why a work is not \
             rankable should not have to guess"
        );
        assert_eq!(
            stored.coordinates.clone_reason(),
            Some(expected_reason),
            "the reason must be recoverable from the stored row"
        );
        assert!(!expected_reason.explain().is_empty());
    }

    db.cleanup().await;
}

#[tokio::test]
async fn coordinates_can_be_measured_from_stored_chapters() {
    let db = scratch("coord_from_chapters").await;

    let work = id("coord-from-chapters");
    let author = fixture_work(&db, &work).await;
    fixture_chapters(&db, &work, &author, 3, 800).await;

    // Pull the prose out of the database the way a backfill would, and measure it.
    let (corpus, _) = wc::corpus_for_work(db.db(), &work, 240)
        .await
        .expect("read chapters")
        .expect("the fixture wrote three chapters");

    assert_eq!(
        corpus.chapters.len(),
        3,
        "every chapter must come back, in reading order"
    );
    let total: usize = corpus.chapters.iter().map(|c| c.word_count).sum();
    assert_eq!(
        total, 2400,
        "stored per-chapter word counts must be what the corpus reports, so the \
         coordinates agree with the number the platform already shows"
    );

    let measured = wc::measure_and_store(db.db(), &work, &corpus, 1, "2026-10-01T12:00:00Z")
        .await
        .expect("store");

    let outcome = measured.coordinates;
    assert!(
        outcome.is_measured(),
        "2400 words of real prose must be measurable, got {outcome:?}"
    );
    let read = wc::measured_coordinates(db.db(), &work)
        .await
        .expect("read")
        .expect("measured");
    assert_eq!(read.word_count, 2400);
    assert!(
        read.sentence_count > 0,
        "real prose must produce sentences, not zero"
    );
    assert_eq!(
        read.chapter_length_spread,
        Some(0.0),
        "three equal-length chapters have no spread"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn a_cleared_work_is_absent_rather_than_stale() {
    let db = scratch("coord_cleared").await;

    let work = id("coord-cleared");
    fixture_work(&db, &work).await;
    wc::measure_and_store(
        db.db(),
        &work,
        &corpus_of(3, 800, 100),
        1,
        "2026-10-01T12:00:00Z",
    )
    .await
    .expect("store");
    assert!(
        wc::measured_coordinates(db.db(), &work)
            .await
            .expect("read")
            .is_some(),
        "the fixture must actually be measured before the clear"
    );

    assert!(
        wc::clear_coordinates(db.db(), &work).await.expect("clear"),
        "clearing an existing row must report that it removed one"
    );
    assert!(
        wc::measured_coordinates(db.db(), &work)
            .await
            .expect("read")
            .is_none(),
        "a cleared work must read as absent; a stale coordinate is worse than none"
    );
    assert!(
        !wc::clear_coordinates(db.db(), &work)
            .await
            .expect("clear again"),
        "clearing a row that is not there must report honestly rather than claim a removal"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn both_engines_report_the_same_coordinates() {
    // This test's real work is the harness, not the assertion. SQLite is the
    // default; with LOREHAVEN_TEST_PG_URL set the same fixtures run on PostgreSQL,
    // and the *same* expected values are computed by the domain layer rather than
    // read from the previous run. So a dialect difference shows up as a failure
    // here rather than as a divergence nobody notices until production.
    let db = scratch("coord_both_engines").await;
    let which = if db.is_postgres() {
        "postgres"
    } else {
        "sqlite"
    };

    let work = id("coord-both-engines");
    fixture_work(&db, &work).await;

    // A corpus with real structure: varied prose, some dialogue, uneven chapters.
    let corpus = Corpus {
        chapters: vec![
            ChapterText {
                word_count: 900,
                plain_text: (0..45)
                    .map(|i| {
                        let words = if i % 3 == 0 { 10 } else { 25 };
                        (0..words)
                            .map(|j| format!("w{i}_{j}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                            + "."
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            },
            ChapterText {
                word_count: 400,
                plain_text: (0..20)
                    .map(|i| {
                        (0..20)
                            .map(|j| format!("v{i}_{j}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                            + "."
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            },
        ],
        dialogue_words: 260,
    };
    // Built from `corpus` itself, not from `corpus_of(2, 0, 260)` -- the fixture
    // below is hand-built with real word counts, so asking the domain layer about a
    // *different* corpus asked it to measure an empty one and reported NoText.
    // The expectation has to be derived from the same input the store is given.
    let expected = match lorehaven_domain::coordinates::coordinates(&corpus) {
        Coordinates::Measured(c) => c,
        other => panic!("[{which}] the fixture must be measurable, got {other:?}"),
    };

    wc::measure_and_store(db.db(), &work, &corpus, 3, "2026-10-01T12:00:00Z")
        .await
        .expect("store");

    let read = wc::measured_coordinates(db.db(), &work)
        .await
        .expect("read")
        .unwrap_or_else(|| panic!("[{which}] the work must be measured"));

    // Exact equality, not a tolerance. §49.8 asks for "byte for byte" and the
    // DOUBLE PRECISION column exists precisely so this holds across engines; a
    // tolerance here would hide the REAL-versus-double difference it was chosen to
    // prevent.
    assert_eq!(
        read.sentence_length_variance, expected.sentence_length_variance,
        "[{which}] sentence_length_variance must match the domain layer exactly"
    );
    assert_eq!(
        read.dialogue_ratio, expected.dialogue_ratio,
        "[{which}] dialogue_ratio"
    );
    assert_eq!(
        read.vocabulary_richness, expected.vocabulary_richness,
        "[{which}] vocabulary_richness"
    );
    assert_eq!(
        read.chapter_length_spread, expected.chapter_length_spread,
        "[{which}] chapter_length_spread"
    );
    assert_eq!(read.word_count, expected.word_count, "[{which}] word_count");
    assert_eq!(
        read.sentence_count, expected.sentence_count,
        "[{which}] sentence_count"
    );

    db.cleanup().await;
}

// ------------------------------------------------------------------- fixtures

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

/// `chapters` + `chapter_revisions` rows carrying real prose.
///
/// The prose is generated rather than pasted so the word count is arithmetic, and
/// `plain_text` is stored to match it — the coordinates depend on that agreement,
/// and a mismatch here would look like a coordinates bug rather than a fixture bug.
async fn fixture_chapters(
    db: &TestDb,
    work_id: &str,
    author_pseud: &str,
    count: usize,
    words_each: usize,
) {
    let now = "2026-10-01T12:00:00Z".to_string();
    for i in 0..count {
        let chapter_id = id(&format!("coord-chapter-{work_id}-{i}"));
        let revision_id = id(&format!("coord-revision-{work_id}-{i}"));
        let plain_text: String = (0..(words_each / 20))
            .map(|_| {
                (0..20)
                    .map(|j| format!("w{j}"))
                    .collect::<Vec<_>>()
                    .join(" ")
                    + "."
            })
            .collect::<Vec<_>>()
            .join(" ");

        exec_bound_mixed(
            db,
            "INSERT INTO chapters (id, work_id, title, order_key, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
            "INSERT INTO chapters (id, work_id, title, order_key, created_at, updated_at) \
             VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)",
            &[
                Bind::Text(chapter_id.clone()),
                Bind::Text(work_id.to_string()),
                Bind::Text(format!("Chapter {i}")),
                // order_key is the fourth parameter and an integer on both engines.
                Bind::Int(i as i64),
                Bind::Text(now.clone()),
                Bind::Text(now.clone()),
            ],
        )
        .await;

        exec_bound_mixed(
            db,
            // revision_number is NOT NULL: a revision without a number cannot be
            // ordered against its siblings, which is what "the newest revision"
            // means. Named explicitly rather than defaulted so the fixture says what
            // the schema requires.
            // Four columns are NOT NULL that a prose-only fixture would not think
            // of: document_json and sanitized_html (ADR 0002 -- the structured
            // document is the source of truth and the other two are *derived* from
            // it), and created_by_pseud_id. plain_text and word_count are also
            // NOT NULL, which is the whole reason this fixture can drive a
            // measurement at all.
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
                // A revision without a number cannot be ordered against its
                // siblings, which is what "the newest revision" means.
                Bind::Int(1),
                // A minimal but well-formed editor document. Its content is
                // irrelevant to coordinates -- only plain_text is read -- so this
                // stays small and says so, rather than inventing a document
                // structure nothing here depends on.
                Bind::Text("{\"type\":\"doc\",\"content\":[]}".to_string()),
                Bind::Text(format!("<p>{}</p>", plain_text)),
                Bind::Text(plain_text),
                // word_count is the third integer column in this fixture to be
                // BIGINT on PostgreSQL and INTEGER on SQLite -- order_key, then
                // revision_number, then this.
                Bind::Int(words_each as i64),
                Bind::Text(author_pseud.to_string()),
                Bind::Text(now.clone()),
            ],
        )
        .await;
    }
}

/// How many coordinate rows exist for a work. Always 0 or 1.
///
/// How many coordinate rows exist for a work. Always 0 or 1.
///
/// One difference between the engines: `work_coordinates.work_id` is UUID on
/// PostgreSQL and TEXT on SQLite, so the PostgreSQL bind needs `::uuid`. A cast
/// cannot be conditional inside one string, which is why this is an if/else on
/// `is_postgres()` rather than one template.
///
/// `count(*)` deliberately carries **no** cast. It is INT8 on PostgreSQL and
/// `TestDb::count_by` decodes an `i64`, which matches. An earlier draft cast it to
/// `::text` on the reasoning that the helper wanted TEXT; that is backwards, and
/// the failure said so precisely:
/// `Rust type i64 (as SQL type INT8) is not compatible with SQL type TEXT`.
async fn count_rows(db: &TestDb, work_id: &str) -> i64 {
    if db.is_postgres() {
        db.count_by(
            "SELECT count(*) FROM work_coordinates WHERE work_id = $1::uuid",
            work_id,
        )
        .await
    } else {
        db.count_by(
            "SELECT count(*) FROM work_coordinates WHERE work_id = ?",
            work_id,
        )
        .await
    }
}
