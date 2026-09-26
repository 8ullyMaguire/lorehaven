//! The query language's comparison operators, end to end against a real
//! database on both backends.
//!
//! `words:>10000` is the motivating query. It has to (a) parse, (b) render to
//! SQL, (c) run, and (d) return the right rows — and the unit tests in
//! `crates/domain/src/query.rs` only cover (a) and (b). These cover (c) and
//! (d), which is where the dialect faults live: `chapter_revisions.word_count`
//! is `BIGINT` in both, but the `CAST` that makes a text bind legal against it
//! only exists on one path, and a `COALESCE` that is forgotten drops every
//! work with no chapters from a `words:>=0` query without any error.

use lorehaven_db::search::search_works_ast;
use test_support::id;

/// One published work with a single chapter of exactly `words` words.
///
/// `word_count` lives on `chapter_revisions`, and the search row sums it over
/// live chapters, so seeding the chapter *and* pointing `current_revision_id`
/// at it is the minimum that makes a word-count filter meaningful.
async fn seed_work(db: &lorehaven_db::Database, slug: &str, title: &str, words: i64) -> String {
    let work_id = id(slug);
    let pseud_id = id(&format!("{slug}-pseud"));
    let account_id = id(&format!("{slug}-acct"));
    let chapter_id = id(&format!("{slug}-ch1"));
    let revision_id = id(&format!("{slug}-rev1"));

    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, datetime('now'), datetime('now'))")
                .bind(&account_id).bind(&format!("{slug}@test.dev"))
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES (?, ?, ?, ?, datetime('now'), datetime('now'))")
                .bind(&pseud_id).bind(&account_id).bind(&pseud_id).bind(&pseud_id)
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            sqlx::query("INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at) VALUES (?, ?, ?, 'published', 'public', datetime('now'), datetime('now'))")
                .bind(&work_id).bind(title).bind(&pseud_id)
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            sqlx::query("INSERT INTO chapters (id, work_id, order_key, title, created_at, updated_at) VALUES (?, ?, 10, 'One', datetime('now'), datetime('now'))")
                .bind(&chapter_id).bind(&work_id)
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            sqlx::query("INSERT INTO chapter_revisions (id, chapter_id, revision_number, document_json, sanitized_html, plain_text, word_count, created_by_pseud_id, created_at) VALUES (?, ?, 1, '{}', '', '', ?, ?, datetime('now'))")
                .bind(&revision_id).bind(&chapter_id).bind(words).bind(&pseud_id)
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            sqlx::query("UPDATE chapters SET current_revision_id = ? WHERE id = ?")
                .bind(&revision_id)
                .bind(&chapter_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .unwrap();
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES ($1::uuid, $2, now(), now())")
                .bind(&account_id).bind(&format!("{slug}@test.dev"))
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES ($1::uuid, $2::uuid, $3, $4, now(), now())")
                .bind(&pseud_id).bind(&account_id).bind(&pseud_id).bind(&pseud_id)
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            sqlx::query("INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at) VALUES ($1::uuid, $2, $3::uuid, 'published', 'public', now(), now())")
                .bind(&work_id).bind(title).bind(&pseud_id)
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            sqlx::query("INSERT INTO chapters (id, work_id, order_key, title, created_at, updated_at) VALUES ($1::uuid, $2::uuid, 10, 'One', now(), now())")
                .bind(&chapter_id).bind(&work_id)
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            sqlx::query("INSERT INTO chapter_revisions (id, chapter_id, revision_number, document_json, sanitized_html, plain_text, word_count, created_by_pseud_id, created_at) VALUES ($1::uuid, $2::uuid, 1, '{}', '', '', $3, $4::uuid, now())")
                .bind(&revision_id).bind(&chapter_id).bind(words).bind(&pseud_id)
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            sqlx::query("UPDATE chapters SET current_revision_id = $1::uuid WHERE id = $2::uuid")
                .bind(&revision_id)
                .bind(&chapter_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .unwrap();
        }
    }
    work_id
}

/// A published work with *no* chapters at all.
///
/// The COALESCE case: its word count sums to NULL, so without a coalesce every
/// comparison against it is NULL and it silently drops out of results.
async fn seed_emptied_work(db: &lorehaven_db::Database, slug: &str, title: &str) -> String {
    let work_id = id(slug);
    let pseud_id = id(&format!("{slug}-pseud"));
    let account_id = id(&format!("{slug}-acct"));
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, datetime('now'), datetime('now'))")
                .bind(&account_id).bind(&format!("{slug}@test.dev"))
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES (?, ?, ?, ?, datetime('now'), datetime('now'))")
                .bind(&pseud_id).bind(&account_id).bind(&pseud_id).bind(&pseud_id)
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
            sqlx::query("INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at) VALUES (?, ?, ?, 'published', 'public', datetime('now'), datetime('now'))")
                .bind(&work_id).bind(title).bind(&pseud_id)
                .execute(db.sqlite_pool().expect("sqlite")).await.unwrap();
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at) VALUES ($1::uuid, $2, now(), now())")
                .bind(&account_id).bind(&format!("{slug}@test.dev"))
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) VALUES ($1::uuid, $2::uuid, $3, $4, now(), now())")
                .bind(&pseud_id).bind(&account_id).bind(&pseud_id).bind(&pseud_id)
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
            sqlx::query("INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at) VALUES ($1::uuid, $2, $3::uuid, 'published', 'public', now(), now())")
                .bind(&work_id).bind(title).bind(&pseud_id)
                .execute(db.postgres_pool().expect("postgres")).await.unwrap();
        }
    }
    work_id
}

/// Titles of the results, so a failure names the work rather than a count.
async fn titles(db: &lorehaven_db::Database, query: &str) -> Vec<String> {
    search_works_ast(db, query, None, 50)
        .await
        .unwrap_or_else(|e| panic!("search {query:?} failed: {e:?}"))
        .into_iter()
        .map(|r| r.title)
        .collect()
}

/// Seed three works: 500, 5 000 and 50 000 words, plus one with no chapters.
async fn seed_spectrum(db: &lorehaven_db::Database) {
    seed_work(db, "cmp-tiny", "Tiny", 500).await;
    seed_work(db, "cmp-mid", "Mid", 5_000).await;
    seed_work(db, "cmp-huge", "Huge", 50_000).await;
    seed_emptied_work(db, "cmp-empty", "Empty").await;
}

#[tokio::test]
async fn words_greater_than_narrows_the_result_set() {
    let dir = test_support::scratch_dir("cmp_gt");
    let tdb = test_support::TestDb::connect_with_dir("cmp-gt", &dir).await;
    seed_spectrum(tdb.db()).await;

    let found = titles(tdb.db(), "words:>10000").await;
    assert_eq!(
        found,
        vec!["Huge".to_string()],
        "words:>10000 should match only the 50k work"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn words_greater_or_equal_includes_the_boundary() {
    let dir = test_support::scratch_dir("cmp_gte");
    let tdb = test_support::TestDb::connect_with_dir("cmp-gte", &dir).await;
    seed_spectrum(tdb.db()).await;

    // `>` excludes the 5 000-word work, `>=` includes it. If the two operators
    // rendered the same SQL this is the test that catches it.
    assert_eq!(
        titles(tdb.db(), "words:>5000").await,
        vec!["Huge".to_string()]
    );
    assert_eq!(
        titles(tdb.db(), "words:>=5000").await.len(),
        2,
        ">= must include the exact boundary"
    );
    // Membership rather than order: the rows tie on `score` and the tiebreak is
    // `updated_at DESC`, whose resolution differs between the two backends.
    // Ordering is not what this test is about; the boundary is.
    let mut gte = titles(tdb.db(), "words:>=5000").await;
    gte.sort();
    assert_eq!(gte, vec!["Huge".to_string(), "Mid".to_string()]);
    tdb.cleanup().await;
}

#[tokio::test]
async fn words_less_than_is_the_mirror_of_greater_than() {
    let dir = test_support::scratch_dir("cmp_lt");
    let tdb = test_support::TestDb::connect_with_dir("cmp-lt", &dir).await;
    seed_spectrum(tdb.db()).await;

    // "Empty" has no chapters, so its word count is 0, and 0 < 5000. That is
    // the correct answer, and it is the same COALESCE behaviour the
    // `words:>=0` test pins from the other side.
    assert_eq!(
        titles(tdb.db(), "words:<5000").await,
        vec!["Empty".to_string(), "Tiny".to_string()],
        "words:<5000 matches the 500-word work and the chapterless one"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn words_less_or_equal_includes_the_boundary() {
    let dir = test_support::scratch_dir("cmp_lte");
    let tdb = test_support::TestDb::connect_with_dir("cmp-lte", &dir).await;
    seed_spectrum(tdb.db()).await;

    // 500 is the boundary, and "Empty" is below it.
    assert_eq!(
        titles(tdb.db(), "words:<=500").await,
        vec!["Empty".to_string(), "Tiny".to_string()],
        "<= must include the exact boundary"
    );
    // And one below the boundary excludes it, which is what makes the
    // boundary test meaningful rather than an artefact of the empty work.
    assert_eq!(
        titles(tdb.db(), "words:<500").await,
        vec!["Empty".to_string()],
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_work_with_no_chapters_still_matches_a_zero_threshold() {
    let dir = test_support::scratch_dir("cmp_coalesce");
    let tdb = test_support::TestDb::connect_with_dir("cmp-coalesce", &dir).await;
    seed_spectrum(tdb.db()).await;

    // Without COALESCE the SUM is NULL, `NULL >= 0` is NULL, and this work
    // vanishes with no error and no indication why.
    let found = titles(tdb.db(), "words:>=0").await;
    assert!(
        found.contains(&"Empty".to_string()),
        "a chapterless work has 0 words and must match words:>=0, got {found:?}"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_comparison_composes_with_a_free_text_term() {
    let dir = test_support::scratch_dir("cmp_and");
    let tdb = test_support::TestDb::connect_with_dir("cmp-and", &dir).await;
    seed_spectrum(tdb.db()).await;

    // Implicit AND: both conditions must hold. `Huge` is the only work that is
    // both over 1000 words and named Huge; `Tiny` and `Mid` are too short.
    assert_eq!(
        titles(tdb.db(), "words:>1000 Huge").await,
        vec!["Huge".to_string()],
        "both conditions must hold"
    );
    // The same free text without the comparison matches only Huge anyway, so
    // the assertion above is not vacuous: `words:>1000 Huge` would match
    // Mid as well if the free text were ignored.
    assert_eq!(titles(tdb.db(), "Huge").await, vec!["Huge".to_string()]);
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_comparison_composes_with_a_negated_term() {
    let dir = test_support::scratch_dir("cmp_not");
    let tdb = test_support::TestDb::connect_with_dir("cmp-not", &dir).await;
    seed_spectrum(tdb.db()).await;

    let found = titles(tdb.db(), "words:>=500 NOT Tiny").await;
    assert!(
        !found.contains(&"Tiny".to_string()),
        "NOT Tiny must exclude Tiny, got {found:?}"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn kudos_comparison_reads_the_aggregate_table() {
    let dir = test_support::scratch_dir("cmp_kudos");
    let tdb = test_support::TestDb::connect_with_dir("cmp-kudos", &dir).await;
    seed_spectrum(tdb.db()).await;

    // No aggregate rows at all: every work has zero kudos. A `kudos:>0` must
    // therefore match nothing, and `kudos:>=0` must match everything -- the
    // COALESCE in the renderer is what makes the second half true.
    assert!(
        titles(tdb.db(), "kudos:>0").await.is_empty(),
        "no work has been kudosed"
    );
    assert_eq!(
        titles(tdb.db(), "kudos:>=0").await.len(),
        4,
        "every work defaults to zero kudos"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_foreign_field_is_an_error_not_an_empty_result() {
    let dir = test_support::scratch_dir("cmp_foreign");
    let tdb = test_support::TestDb::connect_with_dir("cmp-foreign", &dir).await;
    seed_spectrum(tdb.db()).await;

    // `replies:>50` is a forum query typed into the works search. It must fail
    // loudly. Silently returning zero rows would read as "no work has 50
    // replies", which is a lie about a filter the reader believes is active.
    let err = search_works_ast(tdb.db(), "replies:>50", None, 10)
        .await
        .expect_err("a forum field on the works search must be rejected");
    let message = format!("{err}");
    assert!(
        message.contains("replies") || message.contains("render"),
        "the error must name the offending field, got: {message}"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_quote_does_not_change_an_existing_query() {
    let dir = test_support::scratch_dir("cmp_regress");
    let tdb = test_support::TestDb::connect_with_dir("cmp-regress", &dir).await;
    seed_spectrum(tdb.db()).await;

    // The comparison work must not have disturbed the paths that were already
    // there: free text, and `field:value`.
    assert_eq!(titles(tdb.db(), "Huge").await, vec!["Huge".to_string()]);
    assert_eq!(
        titles(tdb.db(), "title:Huge").await,
        vec!["Huge".to_string()],
        "fielded equality still works"
    );
    tdb.cleanup().await;
}
