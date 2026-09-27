//! M38 — Topic-to-work backlinks (`crates/db/src/work_backlink.rs`).
//!
//! One `pub async fn` with no tests, called on every work-discussion page load
//! to render the backlink card. It walks three tables in three steps —
//! `topic_work_links` for the link, `works` for the title, and
//! `work_contributors` ⋈ `pseuds` for the credited authors — and every step can
//! return early, so the branches are the interesting part.
//!
//! Two things in the schema make this worth pinning down:
//!
//! - **`topic_work_links` mixes types**: `topic_id` is `TEXT` but `work_id` is
//!   `UUID` on PostgreSQL, and `TEXT` on SQLite. The function carries casts in
//!   both directions on the PostgreSQL arm, and the round-trip tests are what
//!   keep them honest.
//! - **`public_attribution` is `INTEGER` on both engines**, not `BOOLEAN`. This
//!   schema's PostgreSQL migrations deliberately mirror the SQLite types, so the
//!   predicate is `= 1`; `= true` would be
//!   `operator does not exist: bigint = boolean`. An uncredited contributor
//!   must not appear in the card, and that is the assertion that would catch a
//!   well-meaning "fix" to `= true`.

use std::path::PathBuf;

use lorehaven_db::content::create_work;
use lorehaven_db::identity::{create_account, create_pseud, AccountStatus};
use lorehaven_db::work_backlink::work_for_topic;
use lorehaven_domain::ids::{PseudId, WorkId};
use lorehaven_domain::policy::AgeState;
use test_support::TestDb;
use uuid::Uuid;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-backlink-{tag}-{}-{:?}",
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

    async fn count(&self, table: &str) -> i64 {
        let q = self.tdb.sql(&format!("SELECT COUNT(*) FROM {table}"));
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

    /// A pseud with a known handle.
    async fn pseud(&self, handle: &str) -> (PseudId, String) {
        let account = create_account(
            self.db(),
            &format!("backlink-{}@example.test", Uuid::new_v4()),
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

    /// A work owned by `owner`, titled `title`.
    async fn work(&self, owner: PseudId, title: &str) -> WorkId {
        create_work(self.db(), owner, title, None)
            .await
            .expect("create_work")
            .id
    }

    /// A forum topic to hang a backlink card on.
    async fn topic(&self) -> String {
        let id = format!("t-{}", Uuid::new_v4());
        let account = create_account(
            self.db(),
            &format!("topic-{}@example.test", Uuid::new_v4()),
            AgeState::DeclaredAdult,
            AccountStatus::Active,
        )
        .await
        .expect("create_account");
        let (pseud, _) = self
            .pseud(&format!("topic-poster-{}", Uuid::new_v4()))
            .await;
        let _ = account;
        self.exec(&format!(
            "INSERT INTO forum_topics (id, category_id, author_pseud, title, created_at) \
             VALUES ('{id}', 'cat-general', '{pseud}', 'On this topic', '2026-01-01T00:00:00Z')"
        ))
        .await;
        id
    }

    /// Link `topic` to `work`. The `topic_work_links` row is written directly,
    /// because the route that creates it is out of scope here.
    async fn link(&self, topic: &str, work: &WorkId) {
        self.exec(&format!(
            "INSERT INTO topic_work_links (id, topic_id, work_id, created_at) \
             VALUES ('{id}', '{topic}', '{work}', '2026-01-01T00:00:00Z')",
            id = Uuid::new_v4()
        ))
        .await;
    }

    /// Credit `pseud` on `work`, optionally anonymously.
    ///
    /// `work_contributors` is unique on `(work_id, pseud_id)` and `create_work`
    /// already inserts the owner, so an existing row is updated rather than
    /// duplicated.
    async fn credit(&self, work: &WorkId, pseud: &PseudId, public: bool) {
        let flag = if public { 1 } else { 0 };
        let inserted = self
            .try_exec(&format!(
                "INSERT INTO work_contributors (work_id, pseud_id, role, public_attribution, created_at) \
                 VALUES ('{work}', '{pseud}', 'author', {flag}, '2026-01-01T00:00:00Z')"
            ))
            .await;
        if !inserted {
            self.exec(&format!(
                "UPDATE work_contributors SET public_attribution = {flag} \
                 WHERE work_id = '{work}' AND pseud_id = '{pseud}'"
            ))
            .await;
        }
    }

    /// Run a statement, reporting whether it succeeded instead of panicking.
    async fn try_exec(&self, query: &str) -> bool {
        let q = self.tdb.sql(query);
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query(&q)
                .execute(self.db().sqlite_pool().expect("sqlite"))
                .await
                .is_ok(),
            lorehaven_db::Backend::Postgres => sqlx::query(&q)
                .execute(self.db().postgres_pool().expect("pg"))
                .await
                .is_ok(),
        }
    }
}

// ---------------------------------------------------------------------------
// The happy path
// ---------------------------------------------------------------------------

/// **A linked work comes back with its id, title, and credited author.** This is
/// the whole point of the module, and it covers all three queries plus the
/// `work_id` TEXT/UUID cast on the PostgreSQL arm.
#[tokio::test]
async fn a_linked_work_comes_back() {
    let h = Harness::new("backlink-happy").await;
    let (owner, handle) = h.pseud("author-one").await;
    let work = h.work(owner, "The Linked Work").await;
    h.credit(&work, &owner, true).await;
    let topic = h.topic().await;
    h.link(&topic, &work).await;

    let card = work_for_topic(h.db(), &topic)
        .await
        .expect("query runs")
        .expect("a link exists");
    assert_eq!(card.id, work.to_string());
    assert_eq!(card.title, "The Linked Work");
    assert_eq!(card.author_handles, vec![handle]);
}

/// **Authors come back in contribution order**, which is what makes a
/// co-authored card read correctly.
#[tokio::test]
async fn authors_are_in_contribution_order() {
    let h = Harness::new("backlink-order").await;
    let (first, first_handle) = h.pseud("first-author").await;
    let (second, second_handle) = h.pseud("second-author").await;
    let (third, third_handle) = h.pseud("third-author").await;
    let work = h.work(first, "Co-authored").await;
    // create_work already credited `first`; all three get explicit stamps so
    // ORDER BY created_at has something to order and does not depend on the
    // order the rows happened to be written in.
    h.exec(&format!(
        "UPDATE work_contributors SET created_at = '2026-01-01T00:00:00Z' \
         WHERE work_id = '{work}' AND pseud_id = '{first}'"
    ))
    .await;
    h.credit(&work, &second, true).await;
    h.exec(&format!(
        "UPDATE work_contributors SET created_at = '2026-01-03T00:00:00Z' \
         WHERE work_id = '{work}' AND pseud_id = '{second}'"
    ))
    .await;
    h.credit(&work, &third, true).await;
    h.exec(&format!(
        "UPDATE work_contributors SET created_at = '2026-01-02T00:00:00Z' \
         WHERE work_id = '{work}' AND pseud_id = '{third}'"
    ))
    .await;
    // The order is first, third, second by stamp -- deliberately not the order
    // the credits were added in.
    let topic = h.topic().await;
    h.link(&topic, &work).await;

    let card = work_for_topic(h.db(), &topic)
        .await
        .expect("run")
        .expect("present");
    assert_eq!(
        card.author_handles,
        vec![first_handle, third_handle, second_handle],
        "ORDER BY created_at, which is not the order the credits were added"
    );
}

/// A work with no contributors has a card with no authors — the work still
/// links, it just has nobody to credit.
#[tokio::test]
async fn a_work_with_no_contributors_has_no_authors() {
    let h = Harness::new("backlink-no-authors").await;
    let (owner, _) = h.pseud("lonely").await;
    let work = h.work(owner, "Anonymous").await;
    // Remove the owner row create_work added.
    h.exec(&format!(
        "DELETE FROM work_contributors WHERE work_id = '{work}'"
    ))
    .await;
    let topic = h.topic().await;
    h.link(&topic, &work).await;

    let card = work_for_topic(h.db(), &topic)
        .await
        .expect("run")
        .expect("present");
    assert_eq!(card.title, "Anonymous");
    assert!(card.author_handles.is_empty());
}

// ---------------------------------------------------------------------------
// Attribution
// ---------------------------------------------------------------------------

/// **An uncredited contributor is not named.** `public_attribution` is INTEGER
/// on both engines, so the flag is compared to 1; a falsey value must keep the
/// handle off the public card even though the pseud exists.
#[tokio::test]
async fn an_uncredited_contributor_is_not_named() {
    let h = Harness::new("backlink-uncredited").await;
    let (credited, credited_handle) = h.pseud("credited-author").await;
    let (ghost, ghost_handle) = h.pseud("ghost-author").await;
    let work = h.work(credited, "With a ghost").await;
    h.exec(&format!(
        "DELETE FROM work_contributors WHERE work_id = '{work}'"
    ))
    .await;
    h.credit(&work, &credited, true).await;
    h.credit(&work, &ghost, false).await;

    let topic = h.topic().await;
    h.link(&topic, &work).await;
    let card = work_for_topic(h.db(), &topic)
        .await
        .expect("run")
        .expect("present");

    assert_eq!(
        card.author_handles,
        vec![credited_handle],
        "the uncredited pseud still exists but is not named"
    );
    assert!(
        !card.author_handles.contains(&ghost_handle),
        "and specifically not {ghost_handle}"
    );
}

/// **Crediting someone who has since deleted their pseud drops them from the
/// card** — the query inner-joins `pseuds`, so a dangling contributor row simply
/// does not surface rather than erroring.
#[tokio::test]
async fn a_credit_without_a_pseud_is_not_named() {
    let h = Harness::new("backlink-orphan").await;
    let (owner, owner_handle) = h.pseud("survivor").await;
    let (gone, gone_handle) = h.pseud("departed").await;
    let work = h.work(owner, "After departure").await;
    h.credit(&work, &gone, true).await;
    h.exec(&format!("DELETE FROM pseuds WHERE id = '{gone}'"))
        .await;

    let topic = h.topic().await;
    h.link(&topic, &work).await;
    let card = work_for_topic(h.db(), &topic)
        .await
        .expect("run")
        .expect("present");

    assert_eq!(card.author_handles, vec![owner_handle]);
    assert!(!card.author_handles.contains(&gone_handle));
}

/// A reader who merely bookmarked a work is not a contributor, and never
/// appears in a backlink card.
#[tokio::test]
async fn a_bookmarker_is_not_an_author() {
    let h = Harness::new("backlink-bookmarker").await;
    let (owner, owner_handle) = h.pseud("real-author").await;
    let work = h.work(owner, "Not yours").await;
    let topic = h.topic().await;
    h.link(&topic, &work).await;

    // A second pseud that has no contributor row at all.
    h.pseud("just-a-reader").await;
    let card = work_for_topic(h.db(), &topic)
        .await
        .expect("run")
        .expect("present");
    assert_eq!(card.author_handles, vec![owner_handle]);
}

// ---------------------------------------------------------------------------
// The three early returns
// ---------------------------------------------------------------------------

/// **Step 1's early return: a topic with no link has no card.** No query beyond
/// the link lookup runs, so this is also the cheap path.
#[tokio::test]
async fn an_unlinked_topic_has_no_card() {
    let h = Harness::new("backlink-unlinked").await;
    let topic = h.topic().await;
    assert!(
        work_for_topic(h.db(), &topic)
            .await
            .expect("query runs")
            .is_none(),
        "no link row, so no card"
    );
}

/// A topic id that does not exist has no card.
#[tokio::test]
async fn an_unknown_topic_has_no_card() {
    let h = Harness::new("backlink-unknown-topic").await;
    assert!(work_for_topic(h.db(), &format!("t-{}", Uuid::new_v4()))
        .await
        .expect("query runs")
        .is_none());
}

/// A malformed topic id is refused by the TEXT column on both backends — the
/// topic id is a `TEXT` primary key, so this is a comparison, not a cast.
#[tokio::test]
async fn a_malformed_topic_id_finds_nothing() {
    let h = Harness::new("backlink-bad-topic").await;
    assert!(work_for_topic(h.db(), "not-a-topic-id")
        .await
        .expect("the lookup is a comparison, not a cast")
        .is_none());
}

/// **Step 2's early return: a link to a deleted work has no card.** The title
/// query filters `deleted_at IS NULL`, so a soft-deleted work drops off the topic
/// page without the link row being touched.
#[tokio::test]
async fn a_link_to_a_deleted_work_has_no_card() {
    let h = Harness::new("backlink-deleted").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Gone").await;
    h.exec(&format!(
        "UPDATE works SET deleted_at = '2026-01-02T00:00:00Z' WHERE id = '{work}'"
    ))
    .await;
    let topic = h.topic().await;
    h.link(&topic, &work).await;

    assert!(
        work_for_topic(h.db(), &topic)
            .await
            .expect("query runs")
            .is_none(),
        "the link row survives but the work does not"
    );
    assert_eq!(
        h.count("topic_work_links").await,
        1,
        "and the link itself was not cascaded away"
    );
}

/// **A link cannot be made to a work that does not exist.** `topic_work_links`
/// has a foreign key on `work_id` with `ON DELETE CASCADE`, so the orphan case
/// is unreachable by construction on both engines — PostgreSQL enforces it
/// eagerly, SQLite on every connection. The read path still filters
/// `deleted_at IS NULL`, so the guard is belt-and-braces, not load-bearing.
///
/// This is asserted as "the insert is refused" rather than "the read returns
/// nothing", because the second would only be reachable by defeating the
/// foreign key first.
#[tokio::test]
async fn a_link_to_a_missing_work_is_refused() {
    let h = Harness::new("backlink-missing-work").await;
    let topic = h.topic().await;
    let ghost = WorkId::new();

    assert!(
        !h.try_exec(&format!(
            "INSERT INTO topic_work_links (id, topic_id, work_id, created_at) \
             VALUES ('{id}', '{topic}', '{ghost}', '2026-01-01T00:00:00Z')",
            id = Uuid::new_v4()
        ))
        .await,
        "the foreign key on work_id refuses the orphan link"
    );
    assert_eq!(h.count("topic_work_links").await, 0);
    assert!(work_for_topic(h.db(), &topic).await.expect("run").is_none());
}

/// **Deleting a work cascades its links away.** The reverse direction of the
/// same foreign key: a topic whose work is hard-deleted stops showing a card,
/// and leaves no dangling link row behind.
#[tokio::test]
async fn deleting_a_work_cascades_its_links() {
    let h = Harness::new("backlink-cascade").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Removed entirely").await;
    let topic = h.topic().await;
    h.link(&topic, &work).await;
    assert_eq!(h.count("topic_work_links").await, 1);

    h.exec(&format!("DELETE FROM works WHERE id = '{work}'"))
        .await;

    assert_eq!(
        h.count("topic_work_links").await,
        0,
        "ON DELETE CASCADE took the link with it"
    );
    assert!(work_for_topic(h.db(), &topic).await.expect("run").is_none());
}

/// **A withdrawn work still has a card.** Only `deleted_at` hides a work here;
/// lifecycle and withdrawal are not consulted, because a topic about a
/// withdrawn work should still be findable.
#[tokio::test]
async fn a_withdrawn_work_still_has_a_card() {
    let h = Harness::new("backlink-withdrawn").await;
    let (owner, handle) = h.pseud("author").await;
    let work = h.work(owner, "Withdrawn but linked").await;
    h.credit(&work, &owner, true).await;
    h.exec(&format!(
        "UPDATE works SET lifecycle = 'withdrawn' WHERE id = '{work}'"
    ))
    .await;
    let topic = h.topic().await;
    h.link(&topic, &work).await;

    let card = work_for_topic(h.db(), &topic)
        .await
        .expect("run")
        .expect("present");
    assert_eq!(card.title, "Withdrawn but linked");
    assert_eq!(card.author_handles, vec![handle]);
}

/// A draft work still has a card — again, only deletion hides it.
#[tokio::test]
async fn a_draft_work_still_has_a_card() {
    let h = Harness::new("backlink-draft").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Still a draft").await;
    let topic = h.topic().await;
    h.link(&topic, &work).await;
    assert!(
        work_for_topic(h.db(), &topic).await.expect("run").is_some(),
        "an unpublished work still resolves, because only deleted_at filters"
    );
}

// ---------------------------------------------------------------------------
// Link semantics
// ---------------------------------------------------------------------------

/// **`topic_id` is `UNIQUE`, so a topic has at most one linked work.** A second
/// link for the same topic is refused by the schema rather than shadowing the
/// first.
#[tokio::test]
async fn a_topic_links_to_at_most_one_work() {
    let h = Harness::new("backlink-unique").await;
    let (owner, _) = h.pseud("author").await;
    let first = h.work(owner, "First").await;
    let second = h.work(owner, "Second").await;
    let topic = h.topic().await;
    h.link(&topic, &first).await;

    // The second link violates UNIQUE(topic_id) and is expected to be refused.
    let insert = h.tdb.sql(&format!(
        "INSERT INTO topic_work_links (id, topic_id, work_id, created_at) \
             VALUES ('{id}', '{topic}', '{second}', '2026-01-02T00:00:00Z')",
        id = Uuid::new_v4()
    ));
    let refused = match h.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query(&insert)
            .execute(h.db().sqlite_pool().expect("sqlite"))
            .await
            .is_err(),
        lorehaven_db::Backend::Postgres => sqlx::query(&insert)
            .execute(h.db().postgres_pool().expect("pg"))
            .await
            .is_err(),
    };
    assert!(refused, "UNIQUE(topic_id) refuses the second link");

    // And the first link is the one that resolves.
    let card = work_for_topic(h.db(), &topic)
        .await
        .expect("run")
        .expect("present");
    assert_eq!(card.title, "First");
}

/// **Two topics can link to the same work**, and both cards resolve — the
/// uniqueness is on `topic_id`, not `work_id`.
#[tokio::test]
async fn a_work_can_be_linked_from_several_topics() {
    let h = Harness::new("backlink-many-topics").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Discussed at length").await;
    let topic_a = h.topic().await;
    let topic_b = h.topic().await;
    h.link(&topic_a, &work).await;
    h.link(&topic_b, &work).await;

    for topic in [&topic_a, &topic_b] {
        let card = work_for_topic(h.db(), topic)
            .await
            .expect("run")
            .expect("present");
        assert_eq!(card.id, work.to_string());
        assert_eq!(card.title, "Discussed at length");
    }
    assert_eq!(h.count("topic_work_links").await, 2);
}

/// A card is the same whichever topic asks for the work.
#[tokio::test]
async fn both_topics_see_the_same_card() {
    let h = Harness::new("backlink-same-card").await;
    let (owner, handle) = h.pseud("shared-author").await;
    let work = h.work(owner, "One work, two topics").await;
    h.credit(&work, &owner, true).await;
    let topic_a = h.topic().await;
    let topic_b = h.topic().await;
    h.link(&topic_a, &work).await;
    h.link(&topic_b, &work).await;

    let a = work_for_topic(h.db(), &topic_a)
        .await
        .expect("run")
        .expect("present");
    let b = work_for_topic(h.db(), &topic_b)
        .await
        .expect("run")
        .expect("present");
    assert_eq!(a.id, b.id);
    assert_eq!(a.title, b.title);
    assert_eq!(a.author_handles, b.author_handles);
    assert_eq!(a.author_handles, vec![handle]);
}

/// A title with unusual characters round-trips through the card unchanged.
#[tokio::test]
async fn the_title_round_trips_verbatim() {
    let h = Harness::new("backlink-title").await;
    let (owner, _) = h.pseud("author").await;
    let title = "Quotes \" ' — em-dash, 日本語, emoji 🎭, <script>alert(1)</script>";
    let work = h.work(owner, title).await;
    let topic = h.topic().await;
    h.link(&topic, &work).await;

    let card = work_for_topic(h.db(), &topic)
        .await
        .expect("run")
        .expect("present");
    assert_eq!(
        card.title, title,
        "no escaping, no truncation, no re-encoding"
    );
}

/// The card's `id` is the work's string id, which is what the route links to.
#[tokio::test]
async fn the_card_id_is_the_work_id() {
    let h = Harness::new("backlink-id").await;
    let (owner, _) = h.pseud("author").await;
    let work = h.work(owner, "Identify me").await;
    let topic = h.topic().await;
    h.link(&topic, &work).await;

    let card = work_for_topic(h.db(), &topic)
        .await
        .expect("run")
        .expect("present");
    assert_eq!(card.id, work.to_string());
    assert!(
        uuid::Uuid::parse_str(&card.id).is_ok(),
        "and it parses as a UUID"
    );
}

/// A handle with unusual characters is credited verbatim.
#[tokio::test]
async fn handles_round_trip_verbatim() {
    let h = Harness::new("backlink-handles").await;
    let handle = "Author_42-åß-🎭";
    let (owner, owner_handle) = h.pseud(handle).await;
    let work = h.work(owner, "Exotic handle").await;
    h.credit(&work, &owner, true).await;
    let topic = h.topic().await;
    h.link(&topic, &work).await;

    let card = work_for_topic(h.db(), &topic)
        .await
        .expect("run")
        .expect("present");
    assert_eq!(card.author_handles, vec![owner_handle]);
}
