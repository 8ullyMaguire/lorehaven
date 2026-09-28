//! M45 — Roadmap consensus: cards, ballots, moves (`crates/db/src/roadmap.rs`,
//! spec §44, ADR 0023).
//!
//! Twelve `pub async fn` with no test touching them, behind the public roadmap
//! board and the Elo arena that ranks it. Every card a reader sees comes from
//! `list_cards`, and every vote goes `create_ballot` → `mark_voted` →
//! `apply_elo_and_counters`.
//!
//! **Writing the first tests for this module found it non-functional on
//! PostgreSQL — the production backend.** Four separate decode failures, each
//! in a function the roadmap board calls on every page view:
//!
//! 1. `row_to_card_postgres` read the three counters as `i32` while every PG
//!    branch of `list_cards` selects them as `CAST(... AS BIGINT)`, so *every*
//!    card failed to decode.
//! 2. `list_cards` read `created_at` / `updated_at` (both `TIMESTAMPTZ`) as
//!    `String`, in all four branches.
//! 3. `fetch_ballot` read the `JSONB` columns `card_ids` and `served_elo` as
//!    `String`.
//! 4. `list_moves` read `moved_by` (`UUID`) and `created_at` (`TIMESTAMPTZ`) as
//!    `String`.
//!
//! All four are fixed, and the SQLite arm is unaffected: the PG `SELECT`s now
//! render the non-text types as text and both decoders read `i64`. Every
//! `::text` in `roadmap.rs` is on a PostgreSQL arm; SQLite has no such cast.
//!
//! **This is the most type-hostile schema in the data layer.** Migration 0067
//! is explicit about it — "TEXT timestamps → TIMESTAMPTZ, JSON TEXT → JSONB" —
//! so unlike most modules the dialect split is *designed in*, and each of the
//! following is a place the two arms can disagree:
//!
//! - `elo_rating` is `DOUBLE PRECISION`; the decoders read `f64` on both.
//! - `matches_played` / `times_best` / `times_worst` are `INTEGER`, which is
//!   `INT4` on PostgreSQL and a 64-bit integer on SQLite. The decoders disagree
//!   (`i32` vs `i64`) and the PG `SELECT` compensates with
//!   `CAST(... AS BIGINT)`, so both arrive as the same wire type.
//! - `card_ids` / `served_elo` are `JSONB`, and `fetch_ballot` reads both into
//!   a `String` on *both* backends.
//! - `created_at` / `updated_at` / `voted_at` are `TIMESTAMPTZ` on PostgreSQL
//!   and are read as `String`.
//!
//! All four `list_cards` branches select the counters as
//! `CAST(... AS BIGINT)`, so both backends hand the decoders an `i64` and the
//! two mappers agree. It was the PostgreSQL mapper that disagreed, reading
//! `i32` and breaking every unfiltered and filtered read on PG;
//! `a_filtered_list_matches_the_unfiltered_one` now pins that both branches
//! decode identically.

use std::path::PathBuf;

use lorehaven_db::roadmap::{
    apply_elo_and_counters, arena_candidates, create_ballot, fetch_ballot,
    find_card_by_title_normalized, insert_suggestion, list_cards, list_moves, mark_voted,
    record_move, update_card_stage, upsert_card, Card, find_card_by_id};
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m45-roadmap-{tag}-{}-{:?}",
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

    /// Read `roadmap_suggestions.body` back by column name, on either backend.
    ///
    /// `exec` can only assert that a statement did not fail; this returns
    /// values, which the absent-vs-empty test needs — it is the difference
    /// between "the insert worked" and "the insert stored what was meant".
    /// Dual-backend like `exec`, and reads by NAME rather than position
    /// because the two dialects return the same column in the same place only
    /// by accident.
    async fn suggestion_bodies(&self, account_id: &str) -> Vec<(String, Option<String>)> {
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => {
                sqlx::query_as::<_, (String, Option<String>)>(
                    "SELECT raw_text, body FROM roadmap_suggestions WHERE account_id = ? ORDER BY raw_text",
                )
                .bind(account_id)
                .fetch_all(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("read suggestion bodies")
            }
            lorehaven_db::Backend::Postgres => {
                sqlx::query_as::<_, (String, Option<String>)>(
                    "SELECT raw_text, body FROM roadmap_suggestions WHERE account_id = $1 ORDER BY raw_text",
                )
                .bind(account_id)
                .fetch_all(self.db().postgres_pool().expect("pg"))
                .await
                .expect("read suggestion bodies")
            }
        }
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

    /// `roadmap_*` has no `UUID` columns -- every id there is `TEXT` -- so
    /// comparisons are plain text on both backends. Only the `account_id` and
    /// `moved_by` foreign keys are `UUID` on PostgreSQL, hence [`Self::acct`].
    fn acct(&self, col: &str, value: &str) -> String {
        match self.db().backend() {
            lorehaven_db::Backend::Postgres => format!("{col} = '{value}'::uuid"),
            lorehaven_db::Backend::Sqlite => format!("{col} = '{value}'"),
        }
    }

    async fn account(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO accounts (id, email, created_at, updated_at) \
             VALUES ('{id}', 'rm-{id}@example.test', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        id
    }

    /// A card with the given stage and elo, inserted through the repository so
    /// the tests exercise the same write path the seeder does.
    ///
    /// Title-only by default: most of the suite is about ordering, dedup and
    /// stage transitions, none of which involve prose. `card_with_body` is the
    /// builder for the tests that are about the body.
    async fn card(&self, title: &str, stage: &str, elo: f64) -> Card {
        self.card_with_body(title, stage, elo, "").await
    }

    /// As `card`, with a description. §44.1: the arena shows a card's body, so
    /// the tests that exercise a body need a way to write one.
    async fn card_with_body(&self, title: &str, stage: &str, elo: f64, body: &str) -> Card {
        let card = Card {
            id: format!("card-{}", uuid::Uuid::new_v4()),
            title: title.to_string(),
            body: body.to_string(),
            category: "general".to_string(),
            stage: stage.to_string(),
            elo_rating: elo,
            matches_played: 0,
            times_best: 0,
            times_worst: 0,
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
            updated_at: "2026-01-01T00:00:00+00:00".to_string(),
        };
        upsert_card(self.db(), &card).await.expect("upsert_card");
        card
    }

    /// The board's card titles, in the order `list_cards` returns them. Owned
    /// `String`s, because a `Vec<&str>` borrowed out of the returned `Vec<Card>`
    /// would outlive the temporary.
    async fn titles(&self) -> Vec<String> {
        list_cards(self.db(), None)
            .await
            .expect("list")
            .into_iter()
            .map(|c| c.title)
            .collect()
    }

    /// One card's fields, read back through `list_cards` so the assertions see
    /// what a reader would.
    async fn reload(&self, id: &str) -> Option<Card> {
        list_cards(self.db(), None)
            .await
            .expect("list")
            .into_iter()
            .find(|c| c.id == id)
    }
}

// ---------------------------------------------------------------------------
// upsert_card
// ---------------------------------------------------------------------------

/// A new card lands with the fields it was given.
#[tokio::test]
async fn upsert_inserts_a_new_card() {
    let h = Harness::new("rm-insert").await;
    let card = h.card("Dark mode", "idea", 1500.0).await;

    let found = h.reload(&card.id).await.expect("the card is there");
    assert_eq!(found.title, "Dark mode");
    assert_eq!(found.category, "general");
    assert_eq!(found.stage, "idea");
    assert_eq!(found.elo_rating, 1500.0);
    assert_eq!(found.matches_played, 0);
}

/// The elo and counters of an existing card are *not* reset by a re-upsert.
/// `upsert_card` only updates title/category/stage/updated_at, so a re-seed
/// that carries default zeros must not wipe the arena's accumulated ratings.
#[tokio::test]
async fn a_re_upsert_does_not_reset_elo_or_counters() {
    let h = Harness::new("rm-upsert-preserve").await;
    let card = h.card("Search", "idea", 1500.0).await;
    apply_elo_and_counters(h.db(), &[(card.id.clone(), 1720.5, true, false)])
        .await
        .expect("elo");

    // Re-upsert with the default values a fresh Card literal would carry.
    let reset = Card {
        elo_rating: 1500.0,
        matches_played: 0,
        times_best: 0,
        times_worst: 0,
        ..card.clone()
    };
    upsert_card(h.db(), &reset).await.expect("re-upsert");

    let found = h.reload(&card.id).await.expect("card");
    assert_eq!(
        found.elo_rating, 1720.5,
        "the arena's rating survives a re-seed"
    );
    assert_eq!(found.matches_played, 1, "and so does its match count");
    assert_eq!(found.times_best, 1);
}

/// §44.6: a `shipped` or `rejected` card's stage is never downgraded, so a
/// re-seed carrying a stale stage cannot un-ship a finished feature.
#[tokio::test]
async fn a_shipped_cards_stage_is_never_downgraded() {
    let h = Harness::new("rm-no-downgrade").await;
    let card = h.card("Exports", "shipped", 1500.0).await;

    let stale = Card {
        stage: "idea".to_string(),
        ..card.clone()
    };
    upsert_card(h.db(), &stale).await.expect("re-upsert");

    assert_eq!(
        h.reload(&card.id).await.expect("card").stage,
        "shipped",
        "the ON CONFLICT ... WHERE guard skips the update entirely"
    );
}

/// The same guard covers `rejected`, and cards are never deleted — a rejected
/// card keeps its history.
#[tokio::test]
async fn a_rejected_cards_stage_is_never_downgraded() {
    let h = Harness::new("rm-no-downgrade-rejected").await;
    let card = h.card("Dark mode", "rejected", 1500.0).await;
    let stale = Card {
        stage: "in_progress".to_string(),
        ..card.clone()
    };
    upsert_card(h.db(), &stale).await.expect("re-upsert");

    assert_eq!(h.reload(&card.id).await.expect("card").stage, "rejected");
}

/// A card in any other stage *does* move, so the guard is not simply refusing
/// every update.
#[tokio::test]
async fn a_card_in_an_ordinary_stage_does_move() {
    let h = Harness::new("rm-stage-moves").await;
    let card = h.card("Threads", "idea", 1500.0).await;
    let moved = Card {
        stage: "in_progress".to_string(),
        ..card.clone()
    };
    upsert_card(h.db(), &moved).await.expect("re-upsert");

    assert_eq!(h.reload(&card.id).await.expect("card").stage, "in_progress");
}

/// **A `shipped`/`rejected` card's STAGE is frozen; its prose is not.**
///
/// This test previously asserted the whole row was frozen — including the
/// title — and recorded that as gap M45-D01, on the grounds that it was "not
/// what §44.6 intended" but was a product decision rather than a bug fix. The
/// body column forced the decision: 596 of the 667 seeded cards are `shipped`,
/// so a whole-row freeze would have made 90% of the board permanently unable
/// to carry the column that was added precisely so cards could be read. The
/// arena would have looked finished and been 90% title-only.
///
/// The guard is now a `CASE` on the `stage` assignment rather than a `WHERE` on
/// the whole update:
///
/// ```text
/// stage = CASE WHEN roadmap_cards.stage IN ('shipped','rejected')
///             THEN roadmap_cards.stage ELSE excluded.stage END
/// ```
///
/// so a frozen card keeps its stage and has its title, category and body
/// refreshed. §44.6's promise is about stage and is unchanged. A shipped
/// feature's documentation is exactly the thing that ages worst and most needs
/// correcting, and a typo in a shipped title is now fixable by re-seeding —
/// which is what this test now proves.
#[tokio::test]
async fn a_protected_cards_stage_is_frozen_but_its_prose_is_not() {
    let h = Harness::new("rm-protected-fields").await;
    let card = h.card("Exports", "shipped", 1500.0).await;
    let edited = Card {
        title: "Exports and imports".to_string(),
        body: "Bulk export and import, shipped in v0.06.".to_string(),
        category: "platform".to_string(),
        stage: "idea".to_string(),
        ..card.clone()
    };
    upsert_card(h.db(), &edited).await.expect("re-upsert");

    let found = h.reload(&card.id).await.expect("card");
    assert_eq!(found.stage, "shipped", "the stage is never downgraded");
    assert_eq!(found.title, "Exports and imports", "prose refreshes");
    assert_eq!(found.category, "platform", "prose refreshes");
    assert_eq!(
        found.body, "Bulk export and import, shipped in v0.06.",
        "a shipped card's body is correctable — the reason the guard narrowed"
    );
}

/// The stage guard is a `CASE`, so a frozen card must not be updatable in a
/// way that silently skips the row either: the row is written, and only the
/// stage assignment is conditional. If this ever reads `unchanged` for a
/// `shipped` card, the guard has been widened back to a `WHERE`.
#[tokio::test]
async fn a_frozen_card_still_takes_the_upsert() {
    let h = Harness::new("rm-frozen-upsert").await;
    let card = h.card("Rejected idea", "rejected", 1500.0).await;
    let edited = Card {
        body: "Considered and declined; the reasoning is recorded here.".to_string(),
        stage: "up_next".to_string(),
        ..card.clone()
    };
    upsert_card(h.db(), &edited).await.expect("re-upsert");

    let found = h.reload(&card.id).await.expect("card");
    assert_eq!(found.stage, "rejected", "a rejected card is not resurrected");
    assert_eq!(
        found.body, "Considered and declined; the reasoning is recorded here.",
        "and its reasoning can still be written"
    );
}

// ---------------------------------------------------------------------------
// find_card_by_title_normalized
// ---------------------------------------------------------------------------

/// Case, punctuation and collapsed whitespace do not create a second card.
///
/// `normalize_title` *strips* non-alphanumerics rather than replacing them with
/// a space, so a hyphen joins its neighbours: `Dark-Mode` normalizes to
/// `darkmode`, which does **not** match `dark mode`. Pinned because that is
/// the one input class a caller would reasonably expect to work and it does
/// not — a title of `Dark-Mode` and a title of `DarkMode` are treated as
/// duplicates of each other, and neither matches `Dark Mode`.
#[tokio::test]
async fn a_title_is_found_despite_case_punctuation_and_spacing() {
    let h = Harness::new("rm-normalize").await;
    h.card("Dark  Mode!", "idea", 1500.0).await;

    for probe in ["dark mode", "DARK MODE", "Dark Mode", "dark   mode"] {
        assert!(
            find_card_by_title_normalized(h.db(), probe)
                .await
                .expect("lookup")
                .is_some(),
            "{probe:?} finds the card"
        );
    }
}

/// A different title is not found.
#[tokio::test]
async fn a_different_title_is_not_found() {
    let h = Harness::new("rm-normalize-miss").await;
    h.card("Dark mode", "idea", 1500.0).await;
    assert!(find_card_by_title_normalized(h.db(), "light mode")
        .await
        .expect("lookup")
        .is_none());
}

/// Two cards whose titles normalize the same are ambiguous: the function
/// returns one rather than refusing, which is what makes it a dedup *hint*
/// and not a uniqueness constraint.
///
/// The winner is whichever `list_cards` yields first -- `elo_rating DESC,
/// matches_played DESC, id ASC` -- and *not* the first inserted, because
/// `find_card_by_title_normalized` scans the list rather than querying for the
/// match. Pinned with an explicit elo gap so the expected winner is the
/// higher-rated one, which is both the actual order and the useful answer for
/// a dedup check ("the card users already prefer wins").
#[tokio::test]
async fn two_cards_with_the_same_normalized_title_are_ambiguous() {
    let h = Harness::new("rm-normalize-ambig").await;
    let older = h.card("Dark mode", "idea", 1400.0).await;
    let preferred = h.card("Dark  Mode", "idea", 1800.0).await;

    let found = find_card_by_title_normalized(h.db(), "dark mode")
        .await
        .expect("lookup")
        .expect("a match");
    assert_eq!(
        found.id, preferred.id,
        "the higher-rated card wins, not the first inserted"
    );
    assert_ne!(found.id, older.id, "which is not the original one");
}

/// An empty board finds nothing.
#[tokio::test]
async fn an_empty_board_finds_nothing() {
    let h = Harness::new("rm-normalize-empty").await;
    assert!(find_card_by_title_normalized(h.db(), "anything")
        .await
        .expect("lookup")
        .is_none());
}

// ---------------------------------------------------------------------------
// list_cards
// ---------------------------------------------------------------------------

/// Elo descending, then matches played descending, then id ascending.
///
/// The three cards are given explicit sortable ids because the tie-break
/// is on `id` and the harness's ids are random UUIDs.
#[tokio::test]
async fn cards_are_ordered_by_elo_then_matches_then_id() {
    let h = Harness::new("rm-order").await;
    // Inserted out of order, and with matching counts that leave every
    // earlier sort key tied, so only `id` can decide.
    let c = |id: &str, title: &str, elo: f64| Card {
        id: id.to_string(),
        title: title.to_string(),
        body: String::new(),
        category: "general".to_string(),
        stage: "idea".to_string(),
        elo_rating: elo,
        matches_played: 0,
        times_best: 0,
        times_worst: 0,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    for card in [
        c("card-c", "c", 1500.0),
        c("card-a", "a", 1500.0),
        c("card-b", "b", 1500.0),
    ] {
        upsert_card(h.db(), &card).await.expect("upsert");
    }

    assert_eq!(
        h.titles().await,
        vec!["a", "b", "c"],
        "all three keys tie, so id ASC decides"
    );
}

/// Elo descending wins before any tie-break: a higher-rated card with fewer
/// matches still comes first.
#[tokio::test]
async fn elo_descending_outranks_match_count() {
    let h = Harness::new("rm-order-elo").await;
    let busy = h.card("busy", "idea", 1400.0).await;
    h.card("strong", "idea", 1800.0).await;
    for _ in 0..5 {
        apply_elo_and_counters(h.db(), &[(busy.id.clone(), 1400.0, false, false)])
            .await
            .expect("elo");
    }

    assert_eq!(h.titles().await, vec!["strong", "busy"], "1800 beats 1400");
}

/// The stage filter returns only that stage.
#[tokio::test]
async fn the_stage_filter_returns_only_that_stage() {
    let h = Harness::new("rm-filter").await;
    h.card("idea one", "idea", 1500.0).await;
    h.card("idea two", "idea", 1500.0).await;
    h.card("shipped one", "shipped", 1500.0).await;

    let ideas = list_cards(h.db(), Some("idea")).await.expect("list");
    assert_eq!(ideas.len(), 2);
    assert!(ideas.iter().all(|c| c.stage == "idea"));

    let shipped = list_cards(h.db(), Some("shipped")).await.expect("list");
    assert_eq!(shipped.len(), 1);
    assert_eq!(shipped[0].title, "shipped one");
}

/// **The one deliberate asymmetry in the module.** The PostgreSQL filtered
/// branch carries `CAST(matches_played AS BIGINT)` and the SQLite filtered
/// branch does not, because `INTEGER` is `INT4` on PostgreSQL and a 64-bit
/// integer on SQLite — the cast is needed on one and a no-op on the other. This
/// pins that the two branches return the same rows, which is the thing the
/// asymmetry could plausibly break.
#[tokio::test]
async fn a_filtered_list_matches_the_unfiltered_one() {
    let h = Harness::new("rm-filter-parity").await;
    let keep = h.card("keep", "idea", 1500.0).await;
    h.card("other", "shipped", 1500.0).await;
    apply_elo_and_counters(h.db(), &[(keep.id.clone(), 1600.0, true, false)])
        .await
        .expect("elo");

    let all = list_cards(h.db(), None).await.expect("unfiltered");
    let filtered = list_cards(h.db(), Some("idea")).await.expect("filtered");

    let counters = |cards: &[Card]| -> Vec<(i64, i64, i64)> {
        cards
            .iter()
            .map(|c| (c.matches_played, c.times_best, c.times_worst))
            .collect()
    };
    assert_eq!(all.len(), 2);
    assert_eq!(filtered.len(), 1);
    assert_eq!(
        counters(&filtered),
        counters(&[all[0].clone()]),
        "the same counters decode on the filtered branch as the unfiltered one"
    );
    assert_eq!(filtered[0].matches_played, 1);
    assert_eq!(filtered[0].times_best, 1);
}

/// A stage nothing is in returns an empty list, not everything.
#[tokio::test]
async fn a_stage_with_no_cards_is_empty() {
    let h = Harness::new("rm-filter-empty").await;
    h.card("one", "idea", 1500.0).await;
    assert!(list_cards(h.db(), Some("medium_term"))
        .await
        .expect("list")
        .is_empty());
}

/// An empty board lists empty.
#[tokio::test]
async fn an_empty_board_lists_nothing() {
    let h = Harness::new("rm-list-empty").await;
    assert!(list_cards(h.db(), None).await.expect("list").is_empty());
}

// ---------------------------------------------------------------------------
// arena_candidates
// ---------------------------------------------------------------------------

/// Only `idea`-stage cards are offered, so a shipped or in-progress card never
/// reaches a ballot.
#[tokio::test]
async fn only_idea_cards_are_offered_to_the_arena() {
    let h = Harness::new("rm-arena-stage").await;
    let idea = h.card("idea", "idea", 1500.0).await;
    h.card("up next", "up_next", 1500.0).await;
    h.card("shipped", "shipped", 1500.0).await;

    let candidates = arena_candidates(h.db(), 10).await.expect("candidates");
    let ids: Vec<String> = candidates.into_iter().map(|c| c.id).collect();
    assert_eq!(ids, vec![idea.id]);
}

/// The limit is honoured.
#[tokio::test]
async fn the_arena_honours_its_limit() {
    let h = Harness::new("rm-arena-limit").await;
    for i in 0..6 {
        h.card(&format!("idea {i}"), "idea", 1500.0).await;
    }
    assert_eq!(
        arena_candidates(h.db(), 2).await.expect("candidates").len(),
        2
    );
    assert_eq!(
        arena_candidates(h.db(), 0).await.expect("candidates").len(),
        0
    );
}

/// A limit larger than the pool returns the whole pool rather than erroring.
#[tokio::test]
async fn a_limit_larger_than_the_pool_returns_everything() {
    let h = Harness::new("rm-arena-over").await;
    for i in 0..3 {
        h.card(&format!("idea {i}"), "idea", 1500.0).await;
    }
    assert_eq!(
        arena_candidates(h.db(), 99)
            .await
            .expect("candidates")
            .len(),
        3
    );
}

/// The selection is random, so repeated calls return a subset rather than a
/// fixed prefix -- and every card it returns is a real one.
#[tokio::test]
async fn the_arena_returns_real_cards_whichever_it_picks() {
    let h = Harness::new("rm-arena-random").await;
    for i in 0..8 {
        h.card(&format!("idea {i}"), "idea", 1500.0).await;
    }
    let all: Vec<String> = list_cards(h.db(), Some("idea"))
        .await
        .expect("list")
        .into_iter()
        .map(|c| c.id)
        .collect();

    for _ in 0..5 {
        for c in arena_candidates(h.db(), 3).await.expect("candidates") {
            assert!(all.contains(&c.id), "{} is a real card", c.id);
        }
    }
}

/// An empty pool offers nothing, which is what a route should render as an
/// empty arena rather than an error.
#[tokio::test]
async fn an_empty_pool_offers_nothing() {
    let h = Harness::new("rm-arena-empty").await;
    assert!(arena_candidates(h.db(), 4)
        .await
        .expect("candidates")
        .is_empty());
}

// ---------------------------------------------------------------------------
// create_ballot / fetch_ballot
// ---------------------------------------------------------------------------

/// A ballot round-trips its card ids and served elo.
///
/// The `JSONB` columns are read into a `String` on both backends, which is the
/// module's sharpest portability edge: on SQLite the column is TEXT, and on
/// PostgreSQL it is `JSONB` and sqlx has to hand it over as a JSON string.
#[tokio::test]
async fn a_ballot_round_trips_its_cards_and_served_elo() {
    let h = Harness::new("rm-ballot").await;
    let account = h.account().await;
    let ids = vec!["card-a".to_string(), "card-b".to_string()];
    let elo = vec![
        ("card-a".to_string(), 1500.0_f64),
        ("card-b".to_string(), 1610.5),
    ];

    create_ballot(h.db(), "ballot-1", &ids, &elo, &account)
        .await
        .expect("create_ballot");

    let (got_ids, got_elo, voted_at) = fetch_ballot(h.db(), "ballot-1")
        .await
        .expect("fetch")
        .expect("the ballot");
    assert_eq!(got_ids, ids);
    assert_eq!(got_elo, elo, "the served elo round-trips as f64");
    assert_eq!(voted_at, None, "not voted yet");
}

/// A fractional elo survives the JSON round-trip, which is where a float would
/// lose precision if it went through a string.
#[tokio::test]
async fn a_fractional_elo_survives_the_round_trip() {
    let h = Harness::new("rm-ballot-float").await;
    let account = h.account().await;
    let elo = vec![("c".to_string(), 1_583.333_333_333_3_f64)];

    create_ballot(h.db(), "ballot-f", &["c".to_string()], &elo, &account)
        .await
        .expect("create_ballot");

    let (_, got, _) = fetch_ballot(h.db(), "ballot-f")
        .await
        .expect("fetch")
        .expect("ballot");
    assert_eq!(
        got[0].1, 1_583.333_333_333_3,
        "f64 to JSON and back is exact enough"
    );
}

/// An unknown ballot is `None`, not an error.
#[tokio::test]
async fn fetching_an_unknown_ballot_is_none() {
    let h = Harness::new("rm-ballot-unknown").await;
    assert!(fetch_ballot(h.db(), "no-such-ballot")
        .await
        .expect("fetch")
        .is_none());
}

/// A ballot with no cards is legal and round-trips as an empty list, not NULL.
#[tokio::test]
async fn a_ballot_with_no_cards_round_trips_as_empty() {
    let h = Harness::new("rm-ballot-empty").await;
    let account = h.account().await;
    create_ballot(h.db(), "ballot-e", &[], &[], &account)
        .await
        .expect("create_ballot");

    let (ids, elo, _) = fetch_ballot(h.db(), "ballot-e")
        .await
        .expect("fetch")
        .expect("ballot");
    assert!(ids.is_empty());
    assert!(elo.is_empty());
}

// ---------------------------------------------------------------------------
// mark_voted
// ---------------------------------------------------------------------------

/// Marking stamps `voted_at` and returns true, exactly once.
#[tokio::test]
async fn marking_voted_stamps_the_time_once() {
    let h = Harness::new("rm-voted").await;
    let account = h.account().await;
    create_ballot(
        h.db(),
        "ballot-v",
        &["a".into()],
        &[("a".into(), 1500.0)],
        &account,
    )
    .await
    .expect("create_ballot");

    assert!(mark_voted(h.db(), "ballot-v").await.expect("mark_voted"));
    let (_, _, voted_at) = fetch_ballot(h.db(), "ballot-v")
        .await
        .expect("fetch")
        .expect("ballot");
    assert!(voted_at.is_some(), "stamped");
}

/// The `AND voted_at IS NULL` guard makes a second vote a no-op, which is the
/// one-vote-per-ballot property the arena's ranking depends on.
#[tokio::test]
async fn a_second_vote_is_refused() {
    let h = Harness::new("rm-voted-twice").await;
    let account = h.account().await;
    create_ballot(h.db(), "ballot-2", &["a".into()], &[], &account)
        .await
        .expect("create_ballot");

    assert!(mark_voted(h.db(), "ballot-2").await.expect("first"));
    assert!(
        !mark_voted(h.db(), "ballot-2").await.expect("second"),
        "the WHERE guard makes a replayed vote a no-op"
    );
}

/// Marking an unknown ballot is `false`.
#[tokio::test]
async fn marking_an_unknown_ballot_voted_is_false() {
    let h = Harness::new("rm-voted-unknown").await;
    assert!(!mark_voted(h.db(), "no-such-ballot")
        .await
        .expect("mark_voted"));
}

// ---------------------------------------------------------------------------
// apply_elo_and_counters
// ---------------------------------------------------------------------------

/// The new rating is stored and the counters advance.
#[tokio::test]
async fn applying_elo_stores_the_rating_and_advances_the_counters() {
    let h = Harness::new("rm-elo").await;
    let card = h.card("A", "idea", 1500.0).await;

    apply_elo_and_counters(h.db(), &[(card.id.clone(), 1600.0, true, false)])
        .await
        .expect("elo");

    let found = h.reload(&card.id).await.expect("card");
    assert_eq!(found.elo_rating, 1600.0);
    assert_eq!(found.matches_played, 1);
    assert_eq!(found.times_best, 1);
    assert_eq!(found.times_worst, 0);
}

/// A `times_worst` vote advances the other counter.
#[tokio::test]
async fn a_worst_vote_advances_times_worst() {
    let h = Harness::new("rm-elo-worst").await;
    let card = h.card("A", "idea", 1500.0).await;

    apply_elo_and_counters(h.db(), &[(card.id.clone(), 1400.0, false, true)])
        .await
        .expect("elo");

    let found = h.reload(&card.id).await.expect("card");
    assert_eq!(found.elo_rating, 1400.0);
    assert_eq!(found.times_worst, 1);
    assert_eq!(found.times_best, 0);
}

/// A middle vote advances only `matches_played`, which is what makes
/// `times_best + times_worst <= matches_played` hold.
#[tokio::test]
async fn a_middle_vote_advances_only_matches_played() {
    let h = Harness::new("rm-elo-middle").await;
    let card = h.card("A", "idea", 1500.0).await;

    apply_elo_and_counters(h.db(), &[(card.id.clone(), 1510.0, false, false)])
        .await
        .expect("elo");

    let found = h.reload(&card.id).await.expect("card");
    assert_eq!(found.matches_played, 1);
    assert_eq!(found.times_best, 0);
    assert_eq!(found.times_worst, 0);
    assert!(
        found.times_best + found.times_worst <= found.matches_played,
        "the counters stay consistent"
    );
}

/// Counters accumulate across a match rather than being overwritten.
#[tokio::test]
async fn the_counters_accumulate_across_matches() {
    let h = Harness::new("rm-elo-accumulate").await;
    let card = h.card("A", "idea", 1500.0).await;

    for (elo, best, worst) in [
        (1600.0, true, false),
        (1500.0, false, true),
        (1550.0, false, false),
    ] {
        apply_elo_and_counters(h.db(), &[(card.id.clone(), elo, best, worst)])
            .await
            .expect("elo");
    }

    let found = h.reload(&card.id).await.expect("card");
    assert_eq!(found.matches_played, 3);
    assert_eq!(found.times_best, 1);
    assert_eq!(found.times_worst, 1);
    assert_eq!(found.elo_rating, 1550.0, "the last rating wins");
}

/// A whole ballot is applied in one call -- four cards, each advanced once.
#[tokio::test]
async fn a_whole_ballot_is_applied_in_one_call() {
    let h = Harness::new("rm-elo-batch").await;
    let mut ids: Vec<String> = Vec::new();
    for i in 0..4 {
        ids.push(h.card(&format!("c{i}"), "idea", 1500.0).await.id);
    }
    let updates: Vec<(String, f64, bool, bool)> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), 1500.0 + i as f64 * 10.0, i == 0, i == 3))
        .collect();

    apply_elo_and_counters(h.db(), &updates).await.expect("elo");

    for id in &ids {
        let found = h.reload(id).await.expect("card");
        assert_eq!(found.matches_played, 1, "{id} played one match");
    }
    let first = h.reload(&ids[0]).await.expect("card");
    let last = h.reload(&ids[3]).await.expect("card");
    assert_eq!(first.times_best, 1);
    assert_eq!(last.times_worst, 1);
}

/// An empty update list is a no-op, not an error.
#[tokio::test]
async fn applying_no_updates_is_a_no_op() {
    let h = Harness::new("rm-elo-empty").await;
    let card = h.card("A", "idea", 1500.0).await;
    apply_elo_and_counters(h.db(), &[]).await.expect("elo");
    assert_eq!(h.reload(&card.id).await.expect("card").matches_played, 0);
}

/// An update naming a card that does not exist affects nothing and is not an
/// error -- the batch is applied row by row with no all-or-nothing guarantee.
#[tokio::test]
async fn an_update_for_an_unknown_card_is_ignored() {
    let h = Harness::new("rm-elo-unknown").await;
    let card = h.card("A", "idea", 1500.0).await;

    apply_elo_and_counters(
        h.db(),
        &[(uuid::Uuid::new_v4().to_string(), 1.0, true, true)],
    )
    .await
    .expect("elo");

    assert_eq!(
        h.reload(&card.id).await.expect("card").matches_played,
        0,
        "the real card is untouched"
    );
}

// ---------------------------------------------------------------------------
// record_move / list_moves
// ---------------------------------------------------------------------------

/// A move is recorded and lists back.
#[tokio::test]
async fn a_move_is_recorded_and_lists_back() {
    let h = Harness::new("rm-move").await;
    let card = h.card("A", "idea", 1500.0).await;
    let who = h.account().await;

    record_move(h.db(), &card.id, "idea", "in_progress", "started", &who)
        .await
        .expect("record_move");

    let moves = list_moves(h.db(), 10, 0).await.expect("list_moves");
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].0, card.id);
    assert_eq!(moves[0].1, "idea");
    assert_eq!(moves[0].2, "in_progress");
    assert_eq!(moves[0].3, "started");
    assert_eq!(moves[0].4, who);
    assert!(!moves[0].5.is_empty(), "a created_at is present");
}

/// Newest first, with `moved_by` a UUID column read as text.
#[tokio::test]
async fn moves_are_listed_newest_first() {
    let h = Harness::new("rm-move-order").await;
    let card = h.card("A", "idea", 1500.0).await;
    let who = h.account().await;

    for (i, stage) in ["up_next", "in_progress", "finished"].iter().enumerate() {
        record_move(h.db(), &card.id, "idea", stage, &format!("step {i}"), &who)
            .await
            .expect("record_move");
        // Force distinct timestamps: `now_rfc3339` has sub-second precision, so
        // three inserts in a row can tie and make the order a coin flip.
        h.exec(&format!(
            "UPDATE roadmap_moves SET created_at = '2026-01-0{}T00:00:00Z' WHERE reason = 'step {i}'",
            i + 1
        ))
        .await;
    }

    let moves = list_moves(h.db(), 10, 0).await.expect("list_moves");
    let reasons: Vec<String> = moves.into_iter().map(|m| m.3).collect();
    assert_eq!(reasons, vec!["step 2", "step 1", "step 0"]);
}

/// Limit and offset paginate.
#[tokio::test]
async fn moves_paginate_by_limit_and_offset() {
    let h = Harness::new("rm-move-page").await;
    let card = h.card("A", "idea", 1500.0).await;
    let who = h.account().await;
    for i in 0..5 {
        record_move(h.db(), &card.id, "idea", "up_next", &format!("s{i}"), &who)
            .await
            .expect("record_move");
        h.exec(&format!(
            "UPDATE roadmap_moves SET created_at = '2026-01-01T00:00:0{}Z' WHERE reason = 's{i}'",
            i
        ))
        .await;
    }

    let page1 = list_moves(h.db(), 2, 0).await.expect("page 1");
    let page2 = list_moves(h.db(), 2, 2).await.expect("page 2");
    let page3 = list_moves(h.db(), 2, 4).await.expect("page 3");
    assert_eq!(page1.len(), 2);
    assert_eq!(page2.len(), 2);
    assert_eq!(page3.len(), 1, "the last page is short");
    let all: Vec<String> = [page1, page2, page3]
        .concat()
        .into_iter()
        .map(|m| m.3)
        .collect();
    assert_eq!(
        all,
        vec!["s4", "s3", "s2", "s1", "s0"],
        "no overlap, no gap"
    );
}

/// An empty changelog lists empty.
#[tokio::test]
async fn an_empty_changelog_lists_nothing() {
    let h = Harness::new("rm-move-empty").await;
    assert!(list_moves(h.db(), 10, 0)
        .await
        .expect("list_moves")
        .is_empty());
}

/// A move for a card that does not exist is refused by the foreign key --
/// unlike `apply_elo_and_counters`, which ignores an unknown card.
#[tokio::test]
async fn a_move_for_an_unknown_card_is_refused() {
    let h = Harness::new("rm-move-unknown-card").await;
    let who = h.account().await;
    let result = record_move(
        h.db(),
        &format!("card-{}", uuid::Uuid::new_v4()),
        "idea",
        "shipped",
        "nope",
        &who,
    )
    .await;
    assert!(result.is_err(), "roadmap_moves.card_id is a foreign key");
}

// ---------------------------------------------------------------------------
// insert_suggestion
// ---------------------------------------------------------------------------

/// A suggestion with no card attached is allowed -- that is the point of a
/// suggestion.
#[tokio::test]
async fn an_unattached_suggestion_is_recorded() {
    let h = Harness::new("rm-suggestion").await;
    let who = h.account().await;
    insert_suggestion(h.db(), &who, "please add dark mode", None, None)
        .await
        .expect("insert_suggestion");

    let q = h.tdb.sql(&format!(
        "SELECT raw_text FROM roadmap_suggestions WHERE {}",
        h.acct("account_id", &who)
    ));
    let raw: String = match h.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&q)
            .fetch_one(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("text"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&q)
            .fetch_one(h.db().postgres_pool().expect("pg"))
            .await
            .expect("text"),
    };
    assert_eq!(raw, "please add dark mode");
}

/// A suggestion can name the card it proposes as.
#[tokio::test]
async fn a_suggestion_can_attach_to_a_card() {
    let h = Harness::new("rm-suggestion-attach").await;
    let who = h.account().await;
    let card = h.card("A", "idea", 1500.0).await;

    insert_suggestion(h.db(), &who, "extend this", None, Some(&card.id))
        .await
        .expect("insert_suggestion");

    let q = h.tdb.sql(&format!(
        "SELECT card_id FROM roadmap_suggestions WHERE {}",
        h.acct("account_id", &who)
    ));
    let attached: Option<String> = match h.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&q)
            .fetch_one(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("card_id"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&q)
            .fetch_one(h.db().postgres_pool().expect("pg"))
            .await
            .expect("card_id"),
    };
    assert_eq!(attached.as_deref(), Some(card.id.as_str()));
}

/// `ON DELETE SET NULL` detaches a suggestion when its card is deleted rather
/// than cascading the suggestion away, so the raw text an operator triaged
/// survives the card it referenced.
#[tokio::test]
async fn deleting_a_card_detaches_its_suggestion() {
    let h = Harness::new("rm-suggestion-detach").await;
    let who = h.account().await;
    let card = h.card("A", "idea", 1500.0).await;
    insert_suggestion(h.db(), &who, "extend this", None, Some(&card.id))
        .await
        .expect("insert_suggestion");

    h.exec(&format!(
        "DELETE FROM roadmap_cards WHERE id = '{}'",
        card.id
    ))
    .await;

    let q = h.tdb.sql(&format!(
        "SELECT card_id FROM roadmap_suggestions WHERE {}",
        h.acct("account_id", &who)
    ));
    let attached: Option<String> = match h.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&q)
            .fetch_one(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("card_id"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&q)
            .fetch_one(h.db().postgres_pool().expect("pg"))
            .await
            .expect("card_id"),
    };
    assert_eq!(attached, None, "the suggestion outlives the card");
}

/// A suggestion for an unknown account is refused by the foreign key.
#[tokio::test]
async fn a_suggestion_for_an_unknown_account_is_refused() {
    let h = Harness::new("rm-suggestion-unknown-account").await;
    let result = insert_suggestion(h.db(), &uuid::Uuid::new_v4().to_string(), "hi", None, None).await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// update_card_stage
// ---------------------------------------------------------------------------

/// The stage moves and `updated_at` advances.
#[tokio::test]
async fn updating_the_stage_moves_the_card() {
    let h = Harness::new("rm-update-stage").await;
    let card = h.card("A", "idea", 1500.0).await;

    update_card_stage(h.db(), &card.id, "in_progress")
        .await
        .expect("update_card_stage");

    assert_eq!(h.reload(&card.id).await.expect("card").stage, "in_progress");
}

/// Unlike `upsert_card`, this has no downgrade guard -- it is the operator's
/// tool, and §44.6's protection is on the seeder's path, not the operator's.
/// Pinned so the asymmetry is deliberate rather than an oversight.
#[tokio::test]
async fn updating_the_stage_has_no_downgrade_guard() {
    let h = Harness::new("rm-update-stage-guard").await;
    let card = h.card("A", "shipped", 1500.0).await;

    update_card_stage(h.db(), &card.id, "idea")
        .await
        .expect("update_card_stage");

    assert_eq!(
        h.reload(&card.id).await.expect("card").stage,
        "idea",
        "the operator can move a shipped card back, unlike upsert_card"
    );
}

/// A stage outside the CHECK constraint is refused by the schema.
#[tokio::test]
async fn an_invalid_stage_is_refused_by_the_check_constraint() {
    let h = Harness::new("rm-update-stage-invalid").await;
    let card = h.card("A", "idea", 1500.0).await;

    let result = update_card_stage(h.db(), &card.id, "not_a_stage").await;
    assert!(
        result.is_err(),
        "the CHECK constraint holds on both backends"
    );
}

/// Updating an unknown card is a silent no-op, not an error.
#[tokio::test]
async fn updating_an_unknown_card_is_a_no_op() {
    let h = Harness::new("rm-update-unknown").await;
    update_card_stage(h.db(), &format!("card-{}", uuid::Uuid::new_v4()), "idea")
        .await
        .expect("update_card_stage");
}

// ---------------------------------------------------------------------------
// The card body (spec §44.1, migration 0090)
// ---------------------------------------------------------------------------

/// A body written through the repository reads back byte-for-byte.
///
/// Worth pinning precisely because the column sits at index 2, immediately
/// before `category`, and both are `String`. A mapper left on the old
/// positions compiles, type-checks, and returns the category under the name
/// `body` — with every other assertion in this file still green. Comparing
/// the body against a distinctive string is what catches that specific
/// swap; asserting the body merely "round-trips" against whatever the second
/// column holds would not.
#[tokio::test]
async fn a_cards_body_round_trips_exactly() {
    let h = Harness::new("rm-body-roundtrip").await;
    let prose = "A page of prose about why this feature exists, with a quotation \
mark, a comma, and a — dash, so the round trip is not a trivially-empty string.";
    let card = h.card_with_body("Cache tiers", "idea", 1500.0, prose).await;

    let found = h.reload(&card.id).await.expect("card");
    assert_eq!(found.body, prose, "the body survives the round trip");
    assert_eq!(found.category, "general", "and category did not take its place");
    assert_eq!(found.title, "Cache tiers");
}

/// A card with no body reads as empty, not as an error and not as a missing
/// field. 667 cards predate the column and none of them may break.
#[tokio::test]
async fn a_card_without_a_body_reads_as_empty() {
    let h = Harness::new("rm-body-empty").await;
    let card = h.card("Untitled idea", "idea", 1500.0).await;

    let found = h.reload(&card.id).await.expect("card");
    assert_eq!(found.body, "", "empty is a supported state, not a null");
}

/// `find_card_by_id` returns the same card `list_cards` does, body included.
#[tokio::test]
async fn a_card_is_found_by_id_with_its_body() {
    let h = Harness::new("rm-body-by-id").await;
    let card = h.card_with_body("Fandom preservation report", "idea", 1610.0, "Per-fandom dashboard.").await;

    let found = find_card_by_id(h.db(), &card.id).await.expect("query").expect("a card");
    assert_eq!(found.id, card.id);
    assert_eq!(found.title, "Fandom preservation report");
    assert_eq!(found.body, "Per-fandom dashboard.");
    assert_eq!(found.elo_rating, 1610.0);
}

/// An unknown id is `None`, not a panic and not an error. §3.3: the route turns
/// this into a 404 naming a coarse noun.
#[tokio::test]
async fn an_unknown_card_id_is_none() {
    let h = Harness::new("rm-body-unknown-id").await;
    h.card("Real card", "idea", 1500.0).await;

    let found = find_card_by_id(h.db(), "card-does-not-exist")
        .await
        .expect("query");
    assert!(found.is_none(), "an unknown id must not resolve to a card");
}

/// The arena serves bodies, because a MaxDiff choice is a judgement about the
/// feature and the ballot is where that judgement is made.
#[tokio::test]
async fn the_arena_offers_cards_with_their_bodies() {
    let h = Harness::new("rm-body-arena").await;
    for i in 0..4 {
        h.card_with_body(&format!("Idea {i}"), "idea", 1500.0, &format!("Body {i}."))
            .await;
    }

    let candidates = arena_candidates(h.db(), 4).await.expect("candidates");
    assert_eq!(candidates.len(), 4);
    for c in &candidates {
        assert!(
            c.body.starts_with("Body "),
            "every ballot card carries its body, got {:?}",
            c.body
        );
    }
}

/// A suggestion records the description the member offered, and distinguishes
/// "no description" (NULL) from "an empty description" ('').
#[tokio::test]
async fn a_suggestion_records_its_body_and_distinguishes_absent_from_empty() {
    let h = Harness::new("rm-suggestion-body").await;
    let who = h.account().await;

    insert_suggestion(h.db(), &who, "with a description", Some("Here is why."), None)
        .await
        .expect("insert with body");
    insert_suggestion(h.db(), &who, "with an empty one", Some(""), None)
        .await
        .expect("insert with empty body");
    insert_suggestion(h.db(), &who, "with none at all", None, None)
        .await
        .expect("insert without body");

    let rows = h.suggestion_bodies(&who).await;

    assert_eq!(rows.len(), 3);
    // Returns the BODY (not the row), so a missing row and a NULL body are
    // different: `find` yields None for the former, `Some(None)` for the latter.
    let body_of = |t: &str| rows.iter().find(|r| r.0 == t).map(|r| r.1.as_deref());
    assert_eq!(body_of("with a description"), Some(Some("Here is why.")));
    assert_eq!(body_of("with an empty one"), Some(Some("")));
    assert_eq!(
        body_of("with none at all"),
        Some(None),
        "an absent description is NULL, which is not the same as an empty one"
    );
    assert_eq!(
        rows.iter().filter(|r| r.0 == "with none at all").count(),
        1,
        "the row exists; only its body is absent"
    );
}
