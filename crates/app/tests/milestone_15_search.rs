//! M15.9–15.10 — The search index (`crates/db/src/search.rs`).
//!
//! Three `pub async fn` with no tests, and two of them are on a live path:
//! `worker.rs` runs `rebuild_work_index` on a schedule, and
//! `routes/external.rs` serves `search_works` to anonymous readers at
//! `GET /api/v1/public/search`.
//!
//! The index is a two-table design that is easy to get subtly wrong:
//!
//! - `works_index` holds one row per work (the raw body text).
//! - `works_index_terms` holds one row per whitespace-separated token, with the
//!   token's ordinal as `pos`. Tokenisation is done in Rust, not SQL, and the
//!   normaliser strips non-alphanumerics from both ends and lowercases.
//!
//! The three behaviours worth pinning:
//!
//! - **`rebuild` is idempotent and total.** It deletes both halves before
//!   inserting, so a reindex replaces rather than accumulates. A test that only
//!   calls it once has not tested the swap.
//! - **The term predicate is a prefix match, not a substring.** `t.term LIKE
//!   'word%'`, so searching `wor` finds `word` and searching `ord` does not.
//!   Both engines agree, but nothing in the SQL says so.
//! - **The search predicate is the anonymous one.** Only works that are
//!   `published`, `public` and not soft-deleted surface, because the term index
//!   is not a visibility boundary: the worker fills it from chapter text
//!   regardless of lifecycle.
//!
//! `search_in_work` has no caller outside this module. It is tested anyway — a
//! dead function is a feature that is silently off, and the alternative is
//! leaving it unwritten.

use std::path::PathBuf;

use lorehaven_db::content::create_work;
use lorehaven_db::identity::{create_account, create_pseud, AccountStatus};
use lorehaven_db::search::{rebuild_work_index, search_in_work, search_works};
use lorehaven_domain::ids::{PseudId, WorkId};
use lorehaven_domain::policy::AgeState;
use test_support::TestDb;
use uuid::Uuid;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-search-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

struct Harness {
    tdb: TestDb,
    #[allow(dead_code)]
    dir: PathBuf,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        Self { tdb, dir }
    }

    fn db(&self) -> &lorehaven_db::Database {
        self.tdb.db()
    }

    async fn exec(&self, query: &str) {
        let q = self.tdb.sql(query);
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => {
                sqlx::query(&q)
                    .execute(self.db().sqlite_pool().expect("sqlite"))
                    .await
                    .expect("exec");
            }
            lorehaven_db::Backend::Postgres => {
                sqlx::query(&q)
                    .execute(self.db().postgres_pool().expect("pg"))
                    .await
                    .expect("exec");
            }
        }
    }

    async fn count_where(&self, table: &str, where_clause: &str) -> i64 {
        let q = self.tdb.sql(&format!(
            "SELECT COUNT(*) FROM {table} WHERE {where_clause}"
        ));
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar::<_, i64>(&q)
                .fetch_one(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("count"),
            lorehaven_db::Backend::Postgres => sqlx::query_scalar::<_, i64>(&q)
                .fetch_one(self.db().postgres_pool().expect("pg"))
                .await
                .expect("count"),
        }
    }

    async fn terms_of(&self, work: &WorkId) -> Vec<(String, i64)> {
        let q = self.tdb.sql(&format!(
            "SELECT term, pos FROM works_index_terms WHERE work_id = '{work}' ORDER BY pos"
        ));
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_as::<_, (String, i64)>(&q)
                .fetch_all(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("terms"),
            lorehaven_db::Backend::Postgres => sqlx::query_as::<_, (String, i64)>(&q)
                .fetch_all(self.db().postgres_pool().expect("pg"))
                .await
                .expect("terms"),
        }
    }

    async fn pseud(&self, handle: &str) -> (PseudId, String) {
        let account = create_account(
            self.db(),
            &format!("search-{}@example.test", Uuid::new_v4()),
            AgeState::DeclaredAdult,
            AccountStatus::Active,
        )
        .await
        .expect("create_account");
        let pseud = create_pseud(self.db(), account, handle, handle)
            .await
            .expect("create_pseud");
        (pseud, handle.to_string())
    }

    /// A work, published and public unless told otherwise — the state the
    /// anonymous search door can see.
    async fn work(&self, owner: PseudId, title: &str) -> WorkId {
        let work = create_work(self.db(), owner, title, None)
            .await
            .expect("create_work")
            .id;
        self.set_public(&work, true).await;
        work
    }

    async fn set_public(&self, work: &WorkId, published: bool) {
        if published {
            self.exec(&format!(
                "UPDATE works SET lifecycle = 'published', visibility = 'public' \
                 WHERE id = '{work}'"
            ))
            .await;
        }
    }

    /// A chapter with `words` words in its live revision, so the `word_count`
    /// subquery has something to sum.
    async fn chapter(&self, work: &WorkId, author: PseudId, order: i64, words: i64) {
        let chapter = Uuid::new_v4();
        let revision = Uuid::new_v4();
        // `chapters.current_revision_id` and `chapter_revisions.chapter_id` form
        // a reference cycle, so the chapter is inserted with a NULL current
        // revision, the revision is added, and the pointer is set last. Doing it
        // the other way round trips the FK on the second insert.
        self.exec(&format!(
            "INSERT INTO chapters \
             (id, work_id, order_key, title, current_revision_id, created_at, updated_at) \
             VALUES ('{chapter}', '{work}', {order}, 'Chapter {order}', NULL, \
             '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO chapter_revisions \
             (id, chapter_id, revision_number, document_json, sanitized_html, plain_text, \
              word_count, created_by_pseud_id, created_at) \
             VALUES ('{revision}', '{chapter}', 1, '{{}}', '', '', {words}, '{author}', \
             '2026-01-01T00:00:00Z')"
        ))
        .await;
        self.exec(&format!(
            "UPDATE chapters SET current_revision_id = '{revision}' WHERE id = '{chapter}'"
        ))
        .await;
    }
}

// ---------------------------------------------------------------------------
// rebuild_work_index
// ---------------------------------------------------------------------------

/// **A rebuild writes the body text and one term row per token.**
#[tokio::test]
async fn a_rebuild_indexes_every_token() {
    let h = Harness::new("search-rebuild").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Indexed").await;

    rebuild_work_index(h.db(), &work, "the quick brown fox")
        .await
        .expect("rebuild");

    assert_eq!(
        h.terms_of(&work).await,
        vec![
            ("the".to_string(), 0),
            ("quick".to_string(), 1),
            ("brown".to_string(), 2),
            ("fox".to_string(), 3),
        ]
    );
}

/// **`pos` is the token's ordinal, not a character offset** — the field is what
/// `search_in_work` orders by to produce reading positions.
#[tokio::test]
async fn positions_are_ordinals() {
    let h = Harness::new("search-pos").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Positional").await;
    rebuild_work_index(h.db(), &work, "a bb ccc dddd")
        .await
        .expect("rebuild");

    assert_eq!(
        h.terms_of(&work)
            .await
            .iter()
            .map(|(_, p)| *p)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3],
        "four tokens, positions 0..3"
    );
}

/// **The normaliser lowercases and strips non-alphanumerics from both ends.**
/// This is the one piece of tokenisation logic in the module, so each part of it
/// gets its own assertion.
#[tokio::test]
async fn terms_are_normalised() {
    let h = Harness::new("search-normalise").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Normalised").await;

    rebuild_work_index(
        h.db(),
        &work,
        "Hello, WORLD! \"quoted\" -- dashes -- (parens)",
    )
    .await
    .expect("rebuild");

    let terms: Vec<String> = h
        .terms_of(&work)
        .await
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    assert!(
        terms.contains(&"hello".to_string()),
        "lower-cased: {terms:?}"
    );
    assert!(
        terms.contains(&"world".to_string()),
        "trailing punctuation stripped: {terms:?}"
    );
    assert!(
        !terms.iter().any(|t| t.contains(',')),
        "no term keeps a comma: {terms:?}"
    );
    // A token that is *only* punctuation normalises to empty and is dropped.
    assert_eq!(
        terms.iter().filter(|t| t.is_empty()).count(),
        0,
        "punctuation-only tokens are filtered out: {terms:?}"
    );
}

/// A repeated word is indexed once per occurrence, not deduplicated — `pos`
/// would be ambiguous otherwise.
#[tokio::test]
async fn a_repeated_word_is_indexed_per_occurrence() {
    let h = Harness::new("search-repeat").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Repetition").await;
    rebuild_work_index(h.db(), &work, "echo echo echo")
        .await
        .expect("rebuild");

    assert_eq!(
        h.count_where(
            "works_index_terms",
            &format!("work_id = '{work}' AND term = 'echo'")
        )
        .await,
        3,
        "three occurrences, three rows"
    );
}

/// **A rebuild replaces the previous index rather than adding to it.** The
/// function's doc comment calls this out ("idempotent: stages the new term set,
/// then swaps"), and it is the property a scheduled job depends on — without it
/// every reindex would double the term table and inflate every `score`.
#[tokio::test]
async fn a_second_rebuild_replaces_the_index() {
    let h = Harness::new("search-replace").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Reindexed").await;

    rebuild_work_index(h.db(), &work, "alpha beta gamma")
        .await
        .expect("first");
    rebuild_work_index(h.db(), &work, "delta")
        .await
        .expect("second");

    let terms = h.terms_of(&work).await;
    assert_eq!(
        terms,
        vec![("delta".to_string(), 0)],
        "the old terms are gone, not merged"
    );
    assert_eq!(
        h.count_where("works_index_terms", &format!("work_id = '{work}'"))
            .await,
        1
    );
}

/// Rebuilding the same text twice is idempotent.
#[tokio::test]
async fn rebuilding_the_same_text_twice_is_idempotent() {
    let h = Harness::new("search-idempotent").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Stable").await;

    rebuild_work_index(h.db(), &work, "one two three")
        .await
        .expect("first");
    let after_first = h.terms_of(&work).await;
    rebuild_work_index(h.db(), &work, "one two three")
        .await
        .expect("second");

    assert_eq!(after_first, h.terms_of(&work).await);
    assert_eq!(
        h.count_where("works_index_terms", &format!("work_id = '{work}'"))
            .await,
        3
    );
}

/// **`works_index` holds exactly one row per work**, the body text.
#[tokio::test]
async fn the_index_row_holds_the_body_text() {
    let h = Harness::new("search-body").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Bodied").await;
    let body = "the body text as the worker extracted it";

    rebuild_work_index(h.db(), &work, body)
        .await
        .expect("rebuild");

    let q = h.tdb.sql(&format!(
        "SELECT body_text FROM works_index WHERE work_id = '{work}'"
    ));
    let stored: Option<String> = match h.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&q)
            .fetch_optional(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("scalar"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&q)
            .fetch_optional(h.db().postgres_pool().expect("pg"))
            .await
            .expect("scalar"),
    };
    assert_eq!(stored.as_deref(), Some(body), "verbatim, not truncated");
}

/// Empty body text indexes nothing but still leaves the work indexed, so a
/// work with no extractable text is distinguishable from an unindexed one.
#[tokio::test]
async fn an_empty_body_indexes_no_terms() {
    let h = Harness::new("search-empty").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Nothing").await;

    rebuild_work_index(h.db(), &work, "")
        .await
        .expect("rebuild");

    assert!(h.terms_of(&work).await.is_empty());
    assert_eq!(
        h.count_where("works_index", &format!("work_id = '{work}'"))
            .await,
        1,
        "but the work is still indexed"
    );
}

/// Whitespace-only body text behaves like empty — `split_whitespace` yields
/// nothing.
#[tokio::test]
async fn whitespace_only_indexes_no_terms() {
    let h = Harness::new("search-ws").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Blank").await;
    rebuild_work_index(h.db(), &work, "   \t\n  ")
        .await
        .expect("rebuild");
    assert!(h.terms_of(&work).await.is_empty());
}

/// Two works index independently — the delete is scoped by `work_id`.
#[tokio::test]
async fn rebuilding_one_work_leaves_others_alone() {
    let h = Harness::new("search-scoped").await;
    let (owner, _) = h.pseud("author").await;
    let first = h.work(owner, "First").await;
    let second = h.work(owner, "Second").await;
    rebuild_work_index(h.db(), &first, "alpha")
        .await
        .expect("first");
    rebuild_work_index(h.db(), &second, "beta")
        .await
        .expect("second");

    rebuild_work_index(h.db(), &first, "gamma")
        .await
        .expect("rebuild first");

    assert_eq!(h.terms_of(&first).await, vec![("gamma".to_string(), 0)]);
    assert_eq!(h.terms_of(&second).await, vec![("beta".to_string(), 0)]);
}

/// Unicode body text indexes without error, and non-ASCII letters survive
/// normalisation — `is_alphanumeric` is Unicode-aware.
#[tokio::test]
async fn unicode_text_indexes() {
    let h = Harness::new("search-unicode").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Unicode").await;
    rebuild_work_index(h.db(), &work, "日本語 café naïve")
        .await
        .expect("rebuild");

    let terms: Vec<String> = h
        .terms_of(&work)
        .await
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    assert_eq!(terms, vec!["日本語", "café", "naïve"]);
}

/// A very long body indexes every token, and the last one keeps its position.
#[tokio::test]
async fn a_long_body_indexes_completely() {
    let h = Harness::new("search-long").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Long").await;
    let body = (0..500)
        .map(|i| format!("w{i}"))
        .collect::<Vec<_>>()
        .join(" ");

    rebuild_work_index(h.db(), &work, &body)
        .await
        .expect("rebuild");

    let terms = h.terms_of(&work).await;
    assert_eq!(terms.len(), 500);
    assert_eq!(terms.last().unwrap().1, 499, "the last position is 499");
}

// ---------------------------------------------------------------------------
// search_works
// ---------------------------------------------------------------------------

/// **An anonymous search finds a published public work by a term prefix.**
#[tokio::test]
async fn a_published_work_is_findable() {
    let h = Harness::new("search-find").await;
    let (owner, handle) = h.pseud("searchable-author").await;
    let work = h.work(owner, "The Findable Work").await;
    rebuild_work_index(h.db(), &work, "a story about dragons and knights")
        .await
        .expect("rebuild");

    let results = search_works(h.db(), "dragons", 50).await.expect("search");
    assert_eq!(results.len(), 1, "one result");
    assert_eq!(results[0].work_id, work.to_string());
    assert_eq!(results[0].title, "The Findable Work");
    assert_eq!(results[0].author_handle, handle);
    assert_eq!(results[0].score, 1, "one matching term");
}

/// **A term is a prefix, not a substring.** This is the one matching rule and
/// nothing in the SQL states it, so it gets asserted from both sides.
#[tokio::test]
async fn matching_is_by_prefix_not_substring() {
    let h = Harness::new("search-prefix").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Prefixed").await;
    rebuild_work_index(h.db(), &work, "dragonfly dragons")
        .await
        .expect("rebuild");

    assert_eq!(
        search_works(h.db(), "dragon", 50)
            .await
            .expect("search")
            .len(),
        1,
        "'dragon' is a prefix of both terms, so the work matches"
    );
    assert!(
        search_works(h.db(), "ragons", 50)
            .await
            .expect("search")
            .is_empty(),
        "'ragons' is a substring but not a prefix, so nothing matches"
    );
}

/// **A multi-word needle ORs its words together**, so any one of them is enough.
#[tokio::test]
async fn a_multi_word_needle_ors_its_words() {
    let h = Harness::new("search-multi").await;
    let (owner, _) = h.pseud("author").await;
    let with_first = h.work(owner, "Has the first").await;
    let with_second = h.work(owner, "Has the second").await;
    rebuild_work_index(h.db(), &with_first, "alpha only")
        .await
        .expect("first");
    rebuild_work_index(h.db(), &with_second, "beta only")
        .await
        .expect("second");

    let results = search_works(h.db(), "alpha beta", 50)
        .await
        .expect("search");
    let ids: Vec<String> = results.iter().map(|r| r.work_id.clone()).collect();
    assert!(ids.contains(&with_first.to_string()), "matched on 'alpha'");
    assert!(ids.contains(&with_second.to_string()), "matched on 'beta'");
}

/// The needle is lower-cased, so a capitalised query still matches the
/// normalised index.
#[tokio::test]
async fn the_needle_is_case_insensitive() {
    let h = Harness::new("search-case").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Cased").await;
    rebuild_work_index(h.db(), &work, "lowercase token")
        .await
        .expect("rebuild");

    for needle in ["lowercase", "LOWERCASE", "LowerCase"] {
        assert_eq!(
            search_works(h.db(), needle, 50)
                .await
                .expect("search")
                .len(),
            1,
            "{needle:?} matches a lower-cased index"
        );
    }
}

/// Surrounding whitespace on the needle is trimmed rather than treated as a
/// term.
#[tokio::test]
async fn a_padded_needle_is_trimmed() {
    let h = Harness::new("search-pad").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Padded").await;
    rebuild_work_index(h.db(), &work, "padded")
        .await
        .expect("rebuild");
    assert_eq!(
        search_works(h.db(), "   padded   ", 50)
            .await
            .expect("search")
            .len(),
        1
    );
}

/// **An empty or whitespace-only needle returns nothing** without a query — the
/// early return that stops `LIKE '%'` from matching the entire index.
#[tokio::test]
async fn an_empty_needle_matches_nothing() {
    let h = Harness::new("search-no-needle").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Indexed").await;
    rebuild_work_index(h.db(), &work, "content")
        .await
        .expect("rebuild");

    for needle in ["", "   ", "\t\n"] {
        assert!(
            search_works(h.db(), needle, 50)
                .await
                .expect("search")
                .is_empty(),
            "{needle:?} is not a search"
        );
    }
}

/// A needle that matches nothing returns an empty list, not an error.
#[tokio::test]
async fn a_needle_with_no_match_is_empty() {
    let h = Harness::new("search-nomatch").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Indexed").await;
    rebuild_work_index(h.db(), &work, "content")
        .await
        .expect("rebuild");
    assert!(search_works(h.db(), "zzzznotpresent", 50)
        .await
        .expect("search")
        .is_empty());
}

/// **`score` counts the matching terms, and results are ordered by it** — a
/// work matching both words of a two-word needle outranks one matching either.
#[tokio::test]
async fn score_is_the_matching_term_count_and_ranks_results() {
    let h = Harness::new("search-score").await;
    let (owner, _) = h.pseud("author").await;
    let both = h.work(owner, "Matches both").await;
    let one = h.work(owner, "Matches one").await;
    rebuild_work_index(h.db(), &both, "alpha beta")
        .await
        .expect("both");
    rebuild_work_index(h.db(), &one, "alpha gamma")
        .await
        .expect("one");

    let results = search_works(h.db(), "alpha beta", 50)
        .await
        .expect("search");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].work_id, both.to_string(), "higher score first");
    assert_eq!(results[0].score, 2, "both words matched");
    assert_eq!(results[1].work_id, one.to_string());
    assert_eq!(results[1].score, 1, "one word matched");
}

/// A repeated matching word inflates the score, which is the arithmetic the
/// ordering rests on.
#[tokio::test]
async fn a_repeated_match_raises_the_score() {
    let h = Harness::new("search-repeat-score").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Repetitive").await;
    rebuild_work_index(h.db(), &work, "word word word other")
        .await
        .expect("rebuild");

    let results = search_works(h.db(), "wor", 50).await.expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].score, 3,
        "three occurrences of a matching prefix"
    );
}

/// **`word_count` is summed from live chapter revisions, not read from the
/// denormalised `works.word_count` column** — which the function's doc comment
/// says is unmaintained, so it reports zero for every result.
#[tokio::test]
async fn word_count_comes_from_live_revisions() {
    let h = Harness::new("search-wordcount").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Counted").await;
    h.chapter(&work, owner, 1, 1200).await;
    h.chapter(&work, owner, 2, 800).await;
    rebuild_work_index(h.db(), &work, "countable text")
        .await
        .expect("rebuild");

    let results = search_works(h.db(), "countable", 50).await.expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].word_count, 2000, "1200 + 800 across chapters");
}

/// A work with no chapters has a word count of zero, not NULL — the
/// `COALESCE` in the subquery.
#[tokio::test]
async fn a_work_with_no_chapters_counts_zero() {
    let h = Harness::new("search-nocount").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Uncounted").await;
    rebuild_work_index(h.db(), &work, "countless")
        .await
        .expect("rebuild");

    let results = search_works(h.db(), "countless", 50).await.expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].word_count, 0, "COALESCE, not NULL");
}

/// **`limit` bounds the result set**, and it is honoured on both backends.
#[tokio::test]
async fn the_limit_bounds_the_results() {
    let h = Harness::new("search-limit").await;
    let (owner, _) = h.pseud("author").await;
    let mut works = Vec::new();
    for i in 0..5 {
        let w = h.work(owner, &format!("Work {i}")).await;
        rebuild_work_index(h.db(), &w, "shared term")
            .await
            .expect("rebuild");
        works.push(w);
    }

    assert_eq!(
        search_works(h.db(), "shared", 50).await.expect("all").len(),
        5,
        "five works indexed the same term"
    );
    for limit in [0, 1, 2, 3] {
        let got = search_works(h.db(), "shared", limit)
            .await
            .expect("limited");
        assert_eq!(got.len(), limit as usize, "limit {limit} is honoured");
    }
    let _ = works;
}

// ---------------------------------------------------------------------------
// The visibility boundary
// ---------------------------------------------------------------------------

/// **A draft work is not searchable by an anonymous reader.** The term index is
/// not a visibility boundary — the worker fills it regardless of lifecycle — so
/// this predicate is the only thing keeping unpublished work out.
#[tokio::test]
async fn a_draft_work_is_not_searchable() {
    let h = Harness::new("search-draft").await;
    let (owner, _) = h.pseud("author").await;
    let work = create_work(h.db(), owner, "Unpublished", None)
        .await
        .expect("create_work")
        .id;
    // create_work leaves lifecycle = 'draft'; index it anyway, as the worker does.
    rebuild_work_index(h.db(), &work, "secretmaterial")
        .await
        .expect("rebuild");

    assert_eq!(h.terms_of(&work).await.len(), 1, "it is indexed");
    assert!(
        search_works(h.db(), "secret", 50)
            .await
            .expect("search")
            .is_empty(),
        "but not searchable"
    );
}

/// A published work with `visibility = 'private'` is likewise not searchable.
#[tokio::test]
async fn a_private_work_is_not_searchable() {
    let h = Harness::new("search-private").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Private").await;
    h.exec(&format!(
        "UPDATE works SET lifecycle = 'published', visibility = 'private' WHERE id = '{work}'"
    ))
    .await;
    rebuild_work_index(h.db(), &work, "hiddenmaterial")
        .await
        .expect("rebuild");

    assert_eq!(h.terms_of(&work).await.len(), 1, "indexed");
    assert!(search_works(h.db(), "hidden", 50)
        .await
        .expect("search")
        .is_empty());
}

/// A published public work *is* searchable — the positive half of the same
/// predicate, so the two private cases above are not passing for free.
#[tokio::test]
async fn a_public_published_work_is_searchable() {
    let h = Harness::new("search-public").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Public").await;
    rebuild_work_index(h.db(), &work, "publicmaterial")
        .await
        .expect("rebuild");
    assert_eq!(
        search_works(h.db(), "public", 50)
            .await
            .expect("search")
            .len(),
        1
    );
}

/// **A withdrawn work leaves the search results**, which is the §3.3 case the
/// doc comment describes: a deindex event can lag, so the route is the guard.
#[tokio::test]
async fn a_withdrawn_work_leaves_the_results() {
    let h = Harness::new("search-withdrawn").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Withdrawn").await;
    rebuild_work_index(h.db(), &work, "withdrawnmaterial")
        .await
        .expect("rebuild");
    assert_eq!(
        search_works(h.db(), "withdrawn", 50)
            .await
            .expect("search")
            .len(),
        1
    );

    h.exec(&format!(
        "UPDATE works SET lifecycle = 'withdrawn' WHERE id = '{work}'"
    ))
    .await;

    assert!(
        search_works(h.db(), "withdrawn", 50)
            .await
            .expect("search")
            .is_empty(),
        "still indexed, but no longer offered"
    );
}

/// A soft-deleted work is not searchable either.
#[tokio::test]
async fn a_deleted_work_is_not_searchable() {
    let h = Harness::new("search-deleted").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Deleted").await;
    rebuild_work_index(h.db(), &work, "deletedmaterial")
        .await
        .expect("rebuild");
    h.exec(&format!(
        "UPDATE works SET deleted_at = '2026-01-02T00:00:00Z' WHERE id = '{work}'"
    ))
    .await;
    assert!(search_works(h.db(), "deleted", 50)
        .await
        .expect("search")
        .is_empty());
}

/// **The author's handle comes from the work's owner**, not from whoever
/// contributed — and a work whose owner pseud is gone drops out of results
/// entirely, because the join is an inner one.
#[tokio::test]
async fn a_work_whose_owner_is_gone_is_not_returned() {
    let h = Harness::new("search-owner").await;
    let (owner, _) = h.pseud("departing-author").await;
    let work = h.work(owner, "Orphaned").await;
    rebuild_work_index(h.db(), &work, "orphanedmaterial")
        .await
        .expect("rebuild");
    assert_eq!(
        search_works(h.db(), "orphaned", 50)
            .await
            .expect("search")
            .len(),
        1
    );

    h.exec(&format!("DELETE FROM pseuds WHERE id = '{owner}'"))
        .await;

    assert!(
        search_works(h.db(), "orphaned", 50)
            .await
            .expect("search")
            .is_empty(),
        "an inner join on pseuds drops the work rather than returning a blank handle"
    );
}

/// **Deleting a work cascades its index away**, so a deleted work cannot be
/// resurrected in search results by a stale term row. The foreign keys are
/// `ON DELETE CASCADE` on both index tables.
#[tokio::test]
async fn deleting_a_work_cascades_its_index() {
    let h = Harness::new("search-cascade").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Cascaded").await;
    rebuild_work_index(h.db(), &work, "cascadematerial")
        .await
        .expect("rebuild");

    h.exec(&format!("DELETE FROM works WHERE id = '{work}'"))
        .await;

    assert_eq!(
        h.count_where("works", &format!("id = '{work}'")).await,
        0,
        "the work is gone"
    );
    assert_eq!(
        h.count_where("works_index", &format!("work_id = '{work}'"))
            .await,
        0,
        "and the index row with it"
    );
    assert_eq!(
        h.count_where("works_index_terms", &format!("work_id = '{work}'"))
            .await,
        0,
        "and every term row"
    );
}

/// **Soft-deleting a work does not remove its index** — that is deliberate, and
/// it is why `search_works` needs its own `deleted_at` guard rather than
/// trusting the index to be clean. The deindex event is asynchronous and can
/// lag; the route is the thing that must not serve a removed work.
#[tokio::test]
async fn a_soft_deleted_work_keeps_its_index_rows() {
    let h = Harness::new("search-softkeep").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Soft deleted").await;
    rebuild_work_index(h.db(), &work, "softmaterial")
        .await
        .expect("rebuild");
    h.exec(&format!(
        "UPDATE works SET deleted_at = '2026-01-02T00:00:00Z' WHERE id = '{work}'"
    ))
    .await;

    assert_eq!(
        h.count_where("works_index_terms", &format!("work_id = '{work}'"))
            .await,
        1,
        "the terms are still there"
    );
    assert!(
        search_works(h.db(), "soft", 50)
            .await
            .expect("search")
            .is_empty(),
        "but the predicate keeps it out of results"
    );
}

// ---------------------------------------------------------------------------
// search_in_work
// ---------------------------------------------------------------------------

/// **An in-work search returns the matching terms with their positions**,
/// which is what makes a hit navigable.
#[tokio::test]
async fn an_in_work_search_returns_positions() {
    let h = Harness::new("inwork-hits").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Navigable").await;
    rebuild_work_index(h.db(), &work, "alpha beta gamma delta")
        .await
        .expect("rebuild");

    let hits = search_in_work(h.db(), &work, "beta").await.expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].pos, 1, "the token's ordinal");
    assert_eq!(hits[0].snippet, "beta", "and the term itself");
}

/// **In-work matching is also a prefix match**, matching the works search.
#[tokio::test]
async fn an_in_work_search_is_a_prefix_match() {
    let h = Harness::new("inwork-prefix").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Prefix").await;
    rebuild_work_index(h.db(), &work, "alphabet beta")
        .await
        .expect("rebuild");

    assert_eq!(
        search_in_work(h.db(), &work, "alpha")
            .await
            .expect("search")
            .len(),
        1,
        "'alpha' prefixes 'alphabet'"
    );
    assert!(
        search_in_work(h.db(), &work, "lpha")
            .await
            .expect("search")
            .is_empty(),
        "'lpha' is a substring, not a prefix"
    );
}

/// **In-work hits come back in reading order**, not index order.
#[tokio::test]
async fn in_work_hits_are_ordered_by_position() {
    let h = Harness::new("inwork-order").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Ordered").await;
    let body = "keep one two three four five six seven eight keep nine keep";
    rebuild_work_index(h.db(), &work, body)
        .await
        .expect("rebuild");

    let hits = search_in_work(h.db(), &work, "keep").await.expect("search");
    let positions: Vec<i64> = hits.iter().map(|m| m.pos).collect();
    assert_eq!(positions, vec![0, 9, 11], "ascending, every occurrence");
}

/// The in-work needle is trimmed and lower-cased.
#[tokio::test]
async fn an_in_work_needle_is_trimmed_and_lowercased() {
    let h = Harness::new("inwork-normalise").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Normalised").await;
    rebuild_work_index(h.db(), &work, "Mixed Case")
        .await
        .expect("rebuild");
    assert_eq!(
        search_in_work(h.db(), &work, "  MIXED  ")
            .await
            .expect("search")
            .len(),
        1
    );
}

/// An empty in-work needle returns nothing without a query.
#[tokio::test]
async fn an_empty_in_work_needle_matches_nothing() {
    let h = Harness::new("inwork-empty").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Indexed").await;
    rebuild_work_index(h.db(), &work, "content here")
        .await
        .expect("rebuild");
    for needle in ["", "   "] {
        assert!(search_in_work(h.db(), &work, needle)
            .await
            .expect("search")
            .is_empty());
    }
}

/// **An in-work search does not cross work boundaries** — another work's terms
/// are not returned.
#[tokio::test]
async fn an_in_work_search_is_scoped_to_one_work() {
    let h = Harness::new("inwork-scoped").await;
    let (owner, _) = h.pseud("author").await;
    let target = h.work(owner, "Target").await;
    let other = h.work(owner, "Other").await;
    rebuild_work_index(h.db(), &target, "sharedterm here")
        .await
        .expect("target");
    rebuild_work_index(h.db(), &other, "sharedterm there")
        .await
        .expect("other");

    let hits = search_in_work(h.db(), &target, "shared")
        .await
        .expect("search");
    assert_eq!(hits.len(), 1, "one hit, from this work only");
    assert_eq!(hits[0].pos, 0);
}

/// An unindexed work has no in-work hits.
#[tokio::test]
async fn an_unindexed_work_has_no_in_work_hits() {
    let h = Harness::new("inwork-unindexed").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Never indexed").await;
    assert!(search_in_work(h.db(), &work, "anything")
        .await
        .expect("search")
        .is_empty());
}

/// An unknown work has no in-work hits, and does not error.
#[tokio::test]
async fn an_unknown_work_has_no_in_work_hits() {
    let h = Harness::new("inwork-unknown").await;
    assert!(search_in_work(h.db(), &WorkId::new(), "anything")
        .await
        .expect("search")
        .is_empty());
}

/// **An in-work search finds a draft's terms** — this function has no lifecycle
/// predicate, unlike `search_works`. It is scoped to one work by id, so the
/// caller has already established the reader's access; pinning the difference
/// stops a future edit from quietly changing one of the two.
#[tokio::test]
async fn an_in_work_search_ignores_lifecycle() {
    let h = Harness::new("inwork-lifecycle").await;
    let (owner, _) = h.pseud("author").await;
    let work = create_work(h.db(), owner, "Draft", None)
        .await
        .expect("create_work")
        .id;
    rebuild_work_index(h.db(), &work, "draftterm text")
        .await
        .expect("rebuild");

    assert_eq!(
        search_in_work(h.db(), &work, "draft")
            .await
            .expect("search")
            .len(),
        1,
        "no lifecycle filter here, unlike search_works"
    );
    assert!(
        search_works(h.db(), "draft", 50)
            .await
            .expect("search")
            .is_empty(),
        "whereas search_works filters it out"
    );
}

/// In-work hits are capped at 100, so a needle matching a very common token
/// does not return the whole work.
#[tokio::test]
async fn in_work_hits_are_capped_at_a_hundred() {
    let h = Harness::new("inwork-cap").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Repetitive").await;
    let body = std::iter::repeat_n("keep", 250)
        .collect::<Vec<_>>()
        .join(" ");
    rebuild_work_index(h.db(), &work, &body)
        .await
        .expect("rebuild");

    let hits = search_in_work(h.db(), &work, "keep").await.expect("search");
    assert_eq!(hits.len(), 100, "capped, and in reading order");
    assert_eq!(hits[0].pos, 0);
    assert_eq!(hits[99].pos, 99);
}
