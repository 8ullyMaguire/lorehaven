//! M31 — Work discussion: modes, linked topics, reactions and the
//! comment-to-topic migration tool (`crates/db/src/work_discussion.rs`).
//!
//! Eight public functions with no test touching them. Three properties are
//! load-bearing and are asserted here rather than assumed:
//!
//! **`link_topic` is idempotent per work.** The doc says a chapter-publish hook
//! that fires twice creates exactly one topic. That claim is what stops a
//! retried publish from littering the forum, so it is tested by calling it twice
//! and comparing topic ids — not by counting rows.
//!
//! **`set_reaction` distinguishes cast / change / retract.** Clicking the same
//! reaction twice retracts it rather than being a no-op or an error, which is
//! the whole reason `ReactionOutcome` is an enum the caller reports. All four
//! transitions are covered.
//!
//! **The migration is idempotent and order-preserving.** A second run moves
//! zero rows, and the posts that were made keep each comment's author and
//! timestamp. A migration that is neither leaves comments stranded or
//! duplicated.

use std::path::PathBuf;

use lorehaven_db::work_discussion as wd;
use lorehaven_domain::work_discussion::{ReactionOutcome, WorkDiscussionMode};
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m31-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

struct Db {
    tdb: TestDb,
    #[allow(dead_code)]
    dir: PathBuf,
}

impl Db {
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

    async fn account(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO accounts (id, email, created_at, updated_at) \
             VALUES ('{id}', 'a{id}@example.test', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        id
    }

    /// `pseuds.handle` is UNIQUE (normalised), so a repeated handle in one test
    /// is a constraint violation. The suffix keeps callers able to name a
    /// pseud without coordinating across the test.
    async fn pseud(&self, handle: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let handle = format!("{handle}-{}", &id[..8]);
        self.exec(&format!(
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
             VALUES ('{id}', '{}', '{handle}', '{handle} d', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
            self.account().await
        ))
        .await;
        id
    }

    async fn work(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at) \
             VALUES ('{id}', '{}', 'A Work', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
            self.pseud("work-owner").await
        ))
        .await;
        id
    }

    /// A forum category to hang topics off. `create_topic` foreign-keys it.
    async fn category(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO forum_categories (id, name, position, min_trust) \
             VALUES ('{id}', 'Cat', 1, 0)"
        ))
        .await;
        id
    }

    /// A comment on a work, as the migration tool sees it. `body_version` is
    /// NOT NULL in the schema.
    async fn comment(&self, work: &str, author: &str, body: &str, at: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO comments (id, subject_type, subject_id, author_pseud, body, body_version, created_at) \
             VALUES ('{id}', 'work', '{work}', '{author}', '{body}', '1', '{at}')"
        ))
        .await;
        id
    }

    async fn comment_body(&self, id: &str) -> String {
        let q = self
            .tdb
            .sql(&format!("SELECT body FROM comments WHERE id = '{id}'"));
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&q)
                .fetch_one(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("body"),
            lorehaven_db::Backend::Postgres => sqlx::query_scalar(&q)
                .fetch_one(self.db().postgres_pool().expect("pg"))
                .await
                .expect("body"),
        }
    }

    async fn comment_deleted_at(&self, id: &str) -> Option<String> {
        let q = self.tdb.sql(&format!(
            "SELECT deleted_at FROM comments WHERE id = '{id}'"
        ));
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&q)
                .fetch_one(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("deleted_at"),
            lorehaven_db::Backend::Postgres => sqlx::query_scalar(&q)
                .fetch_one(self.db().postgres_pool().expect("pg"))
                .await
                .expect("deleted_at"),
        }
    }

    /// Post bodies on a topic, oldest first, as the migration produced them.
    async fn post_bodies(&self, topic: &str) -> Vec<String> {
        let q = self.tdb.sql(&format!(
            "SELECT body FROM forum_posts WHERE topic_id = '{topic}' ORDER BY created_at"
        ));
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&q)
                .fetch_all(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("bodies"),
            lorehaven_db::Backend::Postgres => sqlx::query_scalar(&q)
                .fetch_all(self.db().postgres_pool().expect("pg"))
                .await
                .expect("bodies"),
        }
    }

    async fn post_count(&self, topic: &str) -> i64 {
        let q = self.tdb.sql(&format!(
            "SELECT COUNT(*) FROM forum_posts WHERE topic_id = '{topic}'"
        ));
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&q)
                .fetch_one(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("count"),
            lorehaven_db::Backend::Postgres => sqlx::query_scalar(&q)
                .fetch_one(self.db().postgres_pool().expect("pg"))
                .await
                .expect("count"),
        }
    }

    async fn topic_count(&self) -> i64 {
        self.count("forum_topics").await
    }

    async fn count(&self, table: &str) -> i64 {
        let q = self.tdb.sql(&format!("SELECT COUNT(*) FROM {table}"));
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&q)
                .fetch_one(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("count"),
            lorehaven_db::Backend::Postgres => sqlx::query_scalar(&q)
                .fetch_one(self.db().postgres_pool().expect("pg"))
                .await
                .expect("count"),
        }
    }
}

// ---------------------------------------------------------------------------
// work_discussion_mode / set_work_discussion_mode
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_new_work_defaults_to_comments_only() {
    // 0038 makes the column `NOT NULL DEFAULT 'comments_only'`, so "no mode set"
    // is not a reachable state for a work row -- the column itself carries the
    // default. Asserted so the assumption is visible if that default changes.
    let h = Db::new("mode-default").await;
    let work = h.work().await;
    assert_eq!(
        wd::work_discussion_mode(h.db(), &work).await.unwrap(),
        Some(WorkDiscussionMode::CommentsOnly)
    );
}

#[tokio::test]
async fn a_discussion_mode_can_be_set_and_read_back() {
    let h = Db::new("mode-roundtrip").await;
    let work = h.work().await;

    for mode in [
        WorkDiscussionMode::ThreadOnly,
        WorkDiscussionMode::CommentsOnly,
        WorkDiscussionMode::Both,
    ] {
        assert!(wd::set_work_discussion_mode(h.db(), &work, mode)
            .await
            .unwrap());
        assert_eq!(
            wd::work_discussion_mode(h.db(), &work).await.unwrap(),
            Some(mode),
            "each mode round-trips through its storage form"
        );
    }
}

#[tokio::test]
async fn setting_the_mode_twice_keeps_the_later_value() {
    let h = Db::new("mode-twice").await;
    let work = h.work().await;
    wd::set_work_discussion_mode(h.db(), &work, WorkDiscussionMode::CommentsOnly)
        .await
        .unwrap();
    assert!(
        wd::set_work_discussion_mode(h.db(), &work, WorkDiscussionMode::ThreadOnly)
            .await
            .unwrap()
    );
    assert_eq!(
        wd::work_discussion_mode(h.db(), &work).await.unwrap(),
        Some(WorkDiscussionMode::ThreadOnly)
    );
}

#[tokio::test]
async fn setting_the_mode_on_a_work_that_is_not_there_reports_no_change() {
    let h = Db::new("mode-missing").await;
    assert!(!wd::set_work_discussion_mode(
        h.db(),
        &uuid::Uuid::new_v4().to_string(),
        WorkDiscussionMode::ThreadOnly
    )
    .await
    .unwrap());
}

#[tokio::test]
async fn a_soft_deleted_work_is_not_readable_and_not_writable() {
    // Both queries carry `deleted_at IS NULL`, so a deleted work behaves as if
    // it were not there at all rather than exposing a mode nobody can change.
    let h = Db::new("mode-deleted").await;
    let work = h.work().await;
    wd::set_work_discussion_mode(h.db(), &work, WorkDiscussionMode::Both)
        .await
        .unwrap();
    h.exec(&format!(
        "UPDATE works SET deleted_at = '2026-02-01T00:00:00+00:00' WHERE id = '{work}'"
    ))
    .await;

    assert_eq!(
        wd::work_discussion_mode(h.db(), &work).await.unwrap(),
        None,
        "a deleted work reads as having no mode, whatever it was set to"
    );
    assert!(
        !wd::set_work_discussion_mode(h.db(), &work, WorkDiscussionMode::ThreadOnly)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn an_unreadable_stored_mode_reads_as_absent() {
    // KNOWN BEHAVIOUR, pinned deliberately. `parse` returning None is flattened
    // into the function's own `Option`, so a corrupt stored mode and a
    // soft-deleted work are indistinguishable -- both read `None`. The caller
    // falls back to the default (`CommentsOnly`). That is the safe direction to
    // fail: an unreadable mode does not silently become `thread_only`.
    // See docs/known-gaps.md M31-D02.
    let h = Db::new("mode-corrupt").await;
    let work = h.work().await;
    h.exec(&format!(
        "UPDATE works SET discussion_mode = 'telepathy' WHERE id = '{work}'"
    ))
    .await;

    assert_eq!(
        wd::work_discussion_mode(h.db(), &work).await.unwrap(),
        None,
        "an unknown stored mode is dropped rather than guessed at"
    );
}

// ---------------------------------------------------------------------------
// linked_topic / link_topic
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_work_with_no_link_has_no_linked_topic() {
    let h = Db::new("link-absent").await;
    let work = h.work().await;
    assert!(wd::linked_topic(h.db(), &work).await.unwrap().is_none());
}

#[tokio::test]
async fn linking_creates_a_topic_and_a_link_row() {
    let h = Db::new("link").await;
    let work = h.work().await;
    let category = h.category().await;
    let author = h.pseud("linker").await;

    let link = wd::link_topic(h.db(), &work, None, &category, &author, "Work thread")
        .await
        .unwrap();

    assert_eq!(link.work_id, work);
    assert!(link.chapter_id.is_none());
    let found = wd::linked_topic(h.db(), &work)
        .await
        .unwrap()
        .expect("link");
    assert_eq!(found.topic_id, link.topic_id);
    assert_eq!(found.id, link.id);
}

#[tokio::test]
async fn linking_the_same_work_twice_returns_the_same_topic() {
    // The doc's reason for this being idempotent: a chapter-publish hook that
    // fires twice must create exactly one topic, not two.
    let h = Db::new("link-idem").await;
    let work = h.work().await;
    let category = h.category().await;
    let author = h.pseud("linker").await;

    let first = wd::link_topic(h.db(), &work, None, &category, &author, "Work thread")
        .await
        .unwrap();
    let second = wd::link_topic(h.db(), &work, None, &category, &author, "Work thread")
        .await
        .unwrap();

    assert_eq!(
        first.topic_id, second.topic_id,
        "the second call returns the existing link unchanged"
    );
    assert_eq!(h.topic_count().await, 1, "and creates no second topic");
}

#[tokio::test]
async fn a_second_link_keeps_the_original_chapter_and_title() {
    // Idempotent means "returned unchanged", not "updated": a later call with a
    // different chapter must not repoint the existing thread.
    let h = Db::new("link-idem-args").await;
    let work = h.work().await;
    let category = h.category().await;
    let author = h.pseud("linker").await;
    let chapter = uuid::Uuid::new_v4().to_string();
    h.exec(&format!(
        "INSERT INTO chapters (id, work_id, title, order_key, created_at, updated_at) \
         VALUES ('{chapter}', '{work}', 'One', 1, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
    ))
    .await;

    let first = wd::link_topic(h.db(), &work, Some(&chapter), &category, &author, "First")
        .await
        .unwrap();
    let second = wd::link_topic(h.db(), &work, None, &category, &author, "Second")
        .await
        .unwrap();

    assert_eq!(second.topic_id, first.topic_id);
    assert_eq!(
        second.chapter_id, first.chapter_id,
        "the chapter recorded the first time is kept"
    );
}

#[tokio::test]
async fn a_chapter_scoped_link_records_the_chapter() {
    let h = Db::new("link-chapter").await;
    let work = h.work().await;
    let category = h.category().await;
    let author = h.pseud("linker").await;
    let chapter = uuid::Uuid::new_v4().to_string();
    h.exec(&format!(
        "INSERT INTO chapters (id, work_id, title, order_key, created_at, updated_at) \
         VALUES ('{chapter}', '{work}', 'One', 1, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
    ))
    .await;

    let link = wd::link_topic(
        h.db(),
        &work,
        Some(&chapter),
        &category,
        &author,
        "Ch thread",
    )
    .await
    .unwrap();
    assert_eq!(link.chapter_id.as_deref(), Some(chapter.as_str()));
    assert_eq!(
        wd::linked_topic(h.db(), &work)
            .await
            .unwrap()
            .unwrap()
            .chapter_id,
        Some(chapter)
    );
}

#[tokio::test]
async fn links_for_two_works_do_not_leak_into_each_other() {
    let h = Db::new("link-two").await;
    let w1 = h.work().await;
    let w2 = h.work().await;
    let category = h.category().await;
    let author = h.pseud("linker").await;

    let a = wd::link_topic(h.db(), &w1, None, &category, &author, "One")
        .await
        .unwrap();
    let b = wd::link_topic(h.db(), &w2, None, &category, &author, "Two")
        .await
        .unwrap();

    assert_ne!(a.topic_id, b.topic_id);
    assert_eq!(h.topic_count().await, 2);
    assert_eq!(
        wd::linked_topic(h.db(), &w1)
            .await
            .unwrap()
            .unwrap()
            .work_id,
        w1
    );
    assert_eq!(
        wd::linked_topic(h.db(), &w2)
            .await
            .unwrap()
            .unwrap()
            .work_id,
        w2
    );
}

// ---------------------------------------------------------------------------
// reaction_counts / reaction_by / set_reaction
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_work_with_no_reactions_has_no_counts() {
    let h = Db::new("react-none").await;
    let work = h.work().await;
    assert!(wd::reaction_counts(h.db(), &work).await.unwrap().is_empty());
}

#[tokio::test]
async fn reaction_counts_group_by_vote_type() {
    let h = Db::new("react-counts").await;
    let work = h.work().await;
    for (handle, vote) in [("a", "love"), ("b", "love"), ("c", "laugh")] {
        wd::set_reaction(h.db(), &work, &h.pseud(handle).await, Some(vote))
            .await
            .unwrap();
    }

    let counts = wd::reaction_counts(h.db(), &work).await.unwrap();
    let pairs: Vec<(&str, i64)> = counts
        .iter()
        .map(|r| (r.vote_type.as_str(), r.count))
        .collect();
    assert_eq!(
        pairs,
        vec![("love", 2), ("laugh", 1)],
        "most reactions first, then vote_type to break ties"
    );
}

#[tokio::test]
async fn reaction_counts_for_one_work_exclude_another_works_reactions() {
    let h = Db::new("react-isolation").await;
    let w1 = h.work().await;
    let w2 = h.work().await;
    wd::set_reaction(h.db(), &w1, &h.pseud("a").await, Some("love"))
        .await
        .unwrap();
    wd::set_reaction(h.db(), &w2, &h.pseud("b").await, Some("love"))
        .await
        .unwrap();

    assert_eq!(wd::reaction_counts(h.db(), &w1).await.unwrap()[0].count, 1);
    assert_eq!(wd::reaction_counts(h.db(), &w2).await.unwrap()[0].count, 1);
}

#[tokio::test]
async fn equal_counts_are_broken_by_vote_type_so_the_order_is_stable() {
    let h = Db::new("react-tie").await;
    let work = h.work().await;
    for (handle, vote) in [("a", "zzz"), ("b", "aaa")] {
        wd::set_reaction(h.db(), &work, &h.pseud(handle).await, Some(vote))
            .await
            .unwrap();
    }
    let order: Vec<String> = wd::reaction_counts(h.db(), &work)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.vote_type)
        .collect();
    assert_eq!(order, vec!["aaa".to_string(), "zzz".to_string()]);
}

#[tokio::test]
async fn a_pseud_with_no_reaction_has_none() {
    let h = Db::new("react-by-none").await;
    let work = h.work().await;
    assert_eq!(
        wd::reaction_by(h.db(), &work, &h.pseud("quiet").await)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_casted_reaction_is_readable_by_its_pseud_only() {
    let h = Db::new("react-by").await;
    let work = h.work().await;
    let me = h.pseud("me").await;
    let other = h.pseud("other").await;
    wd::set_reaction(h.db(), &work, &me, Some("love"))
        .await
        .unwrap();

    assert_eq!(
        wd::reaction_by(h.db(), &work, &me)
            .await
            .unwrap()
            .as_deref(),
        Some("love")
    );
    assert_eq!(wd::reaction_by(h.db(), &work, &other).await.unwrap(), None);
}

#[tokio::test]
async fn the_first_reaction_is_a_cast() {
    let h = Db::new("react-cast").await;
    let work = h.work().await;
    let me = h.pseud("me").await;
    assert_eq!(
        wd::set_reaction(h.db(), &work, &me, Some("love"))
            .await
            .unwrap(),
        ReactionOutcome::Cast
    );
    assert_eq!(
        wd::reaction_counts(h.db(), &work).await.unwrap()[0].count,
        1
    );
}

#[tokio::test]
async fn changing_a_reaction_is_a_change_and_does_not_double_count() {
    let h = Db::new("react-change").await;
    let work = h.work().await;
    let me = h.pseud("me").await;
    wd::set_reaction(h.db(), &work, &me, Some("love"))
        .await
        .unwrap();

    assert_eq!(
        wd::set_reaction(h.db(), &work, &me, Some("laugh"))
            .await
            .unwrap(),
        ReactionOutcome::Changed
    );
    assert_eq!(
        wd::reaction_by(h.db(), &work, &me)
            .await
            .unwrap()
            .as_deref(),
        Some("laugh")
    );
    let counts = wd::reaction_counts(h.db(), &work).await.unwrap();
    assert_eq!(counts.len(), 1, "one row per (work, pseud)");
    assert_eq!(counts[0].vote_type, "laugh");
    assert_eq!(counts[0].count, 1);
}

#[tokio::test]
async fn clicking_the_same_reaction_again_retracts_it() {
    let h = Db::new("react-toggle").await;
    let work = h.work().await;
    let me = h.pseud("me").await;
    wd::set_reaction(h.db(), &work, &me, Some("love"))
        .await
        .unwrap();

    assert_eq!(
        wd::set_reaction(h.db(), &work, &me, Some("love"))
            .await
            .unwrap(),
        ReactionOutcome::Retracted,
        "the same vote is a toggle off, not a no-op"
    );
    assert_eq!(wd::reaction_by(h.db(), &work, &me).await.unwrap(), None);
    assert!(wd::reaction_counts(h.db(), &work).await.unwrap().is_empty());
}

#[tokio::test]
async fn retracting_a_reaction_that_was_never_cast_is_a_retraction_not_an_error() {
    let h = Db::new("react-retract-none").await;
    let work = h.work().await;
    let me = h.pseud("me").await;
    assert_eq!(
        wd::set_reaction(h.db(), &work, &me, None).await.unwrap(),
        ReactionOutcome::Retracted
    );
    assert!(wd::reaction_by(h.db(), &work, &me).await.unwrap().is_none());
}

#[tokio::test]
async fn a_pseud_can_react_after_retracting() {
    let h = Db::new("react-recast").await;
    let work = h.work().await;
    let me = h.pseud("me").await;
    wd::set_reaction(h.db(), &work, &me, Some("love"))
        .await
        .unwrap();
    wd::set_reaction(h.db(), &work, &me, None).await.unwrap();

    assert_eq!(
        wd::set_reaction(h.db(), &work, &me, Some("laugh"))
            .await
            .unwrap(),
        ReactionOutcome::Cast,
        "retracting deletes the row, so the next one is a fresh cast"
    );
    assert_eq!(
        wd::reaction_by(h.db(), &work, &me)
            .await
            .unwrap()
            .as_deref(),
        Some("laugh")
    );
}

#[tokio::test]
async fn two_pseuds_reacting_do_not_interfere() {
    let h = Db::new("react-two").await;
    let work = h.work().await;
    let a = h.pseud("a").await;
    let b = h.pseud("b").await;
    wd::set_reaction(h.db(), &work, &a, Some("love"))
        .await
        .unwrap();
    wd::set_reaction(h.db(), &work, &b, Some("love"))
        .await
        .unwrap();

    assert_eq!(
        wd::set_reaction(h.db(), &work, &a, None).await.unwrap(),
        ReactionOutcome::Retracted
    );
    assert_eq!(
        wd::reaction_by(h.db(), &work, &b).await.unwrap().as_deref(),
        Some("love")
    );
    assert_eq!(
        wd::reaction_counts(h.db(), &work).await.unwrap()[0].count,
        1
    );
}

#[tokio::test]
async fn retracting_a_reaction_leaves_the_others_counts_intact() {
    let h = Db::new("react-count-after").await;
    let work = h.work().await;
    let a = h.pseud("a").await;
    let b = h.pseud("b").await;
    let c = h.pseud("c").await;
    for p in [&a, &b, &c] {
        wd::set_reaction(h.db(), &work, p, Some("love"))
            .await
            .unwrap();
    }

    assert_eq!(
        wd::set_reaction(h.db(), &work, &a, None).await.unwrap(),
        ReactionOutcome::Retracted
    );

    let counts = wd::reaction_counts(h.db(), &work).await.unwrap();
    assert_eq!(counts.len(), 1);
    assert_eq!(counts[0].count, 2, "one retraction drops the count by one");
    assert_eq!(wd::reaction_by(h.db(), &work, &a).await.unwrap(), None);
    assert_eq!(
        wd::reaction_by(h.db(), &work, &b).await.unwrap().as_deref(),
        Some("love")
    );
}

#[tokio::test]
async fn a_pseud_who_reacted_nothing_appears_in_no_count() {
    // The aggregation is over rows, not over pseuds: a pseud who has not
    // reacted contributes nothing, and retracting the last reaction leaves no
    // empty `vote_type` group behind.
    let h = Db::new("react-empty-group").await;
    let work = h.work().await;
    let me = h.pseud("me").await;
    wd::set_reaction(h.db(), &work, &me, Some("love"))
        .await
        .unwrap();
    wd::set_reaction(h.db(), &work, &me, None).await.unwrap();

    assert!(
        wd::reaction_counts(h.db(), &work).await.unwrap().is_empty(),
        "no zero-count group is emitted"
    );
}

// ---------------------------------------------------------------------------
// migrate_comments_to_topic
// ---------------------------------------------------------------------------

#[tokio::test]
async fn migrating_a_work_with_no_comments_still_creates_its_topic() {
    let h = Db::new("mig-empty").await;
    let work = h.work().await;
    let category = h.category().await;
    let author = h.pseud("mover").await;

    let report = wd::migrate_comments_to_topic(h.db(), &work, &category, &author, "Thread")
        .await
        .unwrap();
    assert_eq!(report.moved, 0);
    assert_eq!(
        wd::linked_topic(h.db(), &work)
            .await
            .unwrap()
            .unwrap()
            .topic_id,
        report.topic_id,
        "the link is made whether or not there was anything to move"
    );
}

#[tokio::test]
async fn each_comment_becomes_a_post_with_its_text() {
    let h = Db::new("mig-basic").await;
    let work = h.work().await;
    let category = h.category().await;
    let author = h.pseud("mover").await;
    h.comment(
        &work,
        &h.pseud("c1").await,
        "first",
        "2026-01-01T00:00:00+00:00",
    )
    .await;
    h.comment(
        &work,
        &h.pseud("c2").await,
        "second",
        "2026-01-02T00:00:00+00:00",
    )
    .await;

    let report = wd::migrate_comments_to_topic(h.db(), &work, &category, &author, "Thread")
        .await
        .unwrap();

    assert_eq!(report.moved, 2);
    assert_eq!(
        h.post_bodies(&report.topic_id).await,
        vec!["first".to_string(), "second".to_string()],
        "original order and original text"
    );
}

#[tokio::test]
async fn a_moved_comment_is_soft_deleted_with_a_tombstone_pointing_at_the_topic() {
    let h = Db::new("mig-tombstone").await;
    let work = h.work().await;
    let category = h.category().await;
    let author = h.pseud("mover").await;
    let c = h
        .comment(
            &work,
            &h.pseud("c1").await,
            "goodbye",
            "2026-01-01T00:00:00+00:00",
        )
        .await;

    wd::migrate_comments_to_topic(h.db(), &work, &category, &author, "Thread")
        .await
        .unwrap();

    assert!(
        h.comment_deleted_at(&c).await.is_some(),
        "the original is soft-deleted, not destroyed"
    );
    assert_eq!(
        h.comment_body(&c).await,
        "[moved to the work's discussion thread]"
    );
}

#[tokio::test]
async fn a_post_keeps_the_comment_s_author_and_timestamp() {
    let h = Db::new("mig-attrib").await;
    let work = h.work().await;
    let category = h.category().await;
    let author = h.pseud("mover").await;
    let speaker = h.pseud("speaker").await;
    h.comment(&work, &speaker, "mine", "2026-03-04T05:06:07+00:00")
        .await;

    let report = wd::migrate_comments_to_topic(h.db(), &work, &category, &author, "Thread")
        .await
        .unwrap();

    let q = h.tdb.sql(&format!(
        "SELECT author_pseud, created_at FROM forum_posts WHERE topic_id = '{}'",
        report.topic_id
    ));
    let (who, when): (String, String) = match h.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_as(&q)
            .fetch_one(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("post"),
        lorehaven_db::Backend::Postgres => sqlx::query_as(&q)
            .fetch_one(h.db().postgres_pool().expect("pg"))
            .await
            .expect("post"),
    };
    assert_eq!(who, speaker, "the post is still the commenter's");
    assert_eq!(
        when, "2026-03-04T05:06:07+00:00",
        "and still at the same time"
    );
}

#[tokio::test]
async fn migrating_twice_moves_nothing_the_second_time() {
    let h = Db::new("mig-idem").await;
    let work = h.work().await;
    let category = h.category().await;
    let author = h.pseud("mover").await;
    h.comment(
        &work,
        &h.pseud("c1").await,
        "one",
        "2026-01-01T00:00:00+00:00",
    )
    .await;

    let first = wd::migrate_comments_to_topic(h.db(), &work, &category, &author, "Thread")
        .await
        .unwrap();
    let second = wd::migrate_comments_to_topic(h.db(), &work, &category, &author, "Thread")
        .await
        .unwrap();

    assert_eq!(first.moved, 1);
    assert_eq!(
        second.moved, 0,
        "a soft-deleted comment is not migrated again"
    );
    assert_eq!(
        second.topic_id, first.topic_id,
        "and it reuses the same topic"
    );
    assert_eq!(h.post_count(&first.topic_id).await, 1, "no duplicate post");
}

#[tokio::test]
async fn a_deleted_comment_is_not_migrated() {
    let h = Db::new("mig-skip-deleted").await;
    let work = h.work().await;
    let category = h.category().await;
    let author = h.pseud("mover").await;
    let gone = h
        .comment(
            &work,
            &h.pseud("c1").await,
            "gone",
            "2026-01-01T00:00:00+00:00",
        )
        .await;
    h.comment(
        &work,
        &h.pseud("c2").await,
        "stays",
        "2026-01-02T00:00:00+00:00",
    )
    .await;
    h.exec(&format!(
        "UPDATE comments SET deleted_at = '2026-01-05T00:00:00+00:00' WHERE id = '{gone}'"
    ))
    .await;

    let report = wd::migrate_comments_to_topic(h.db(), &work, &category, &author, "Thread")
        .await
        .unwrap();
    assert_eq!(report.moved, 1, "an already-deleted comment is left alone");
    assert_eq!(
        h.post_bodies(&report.topic_id).await,
        vec!["stays".to_string()]
    );
}

#[tokio::test]
async fn comments_on_something_else_are_not_migrated() {
    let h = Db::new("mig-subject").await;
    let work = h.work().await;
    let category = h.category().await;
    let author = h.pseud("mover").await;
    h.comment(
        &work,
        &h.pseud("c1").await,
        "mine",
        "2026-01-01T00:00:00+00:00",
    )
    .await;

    // A comment on a different work, and one on a series rather than a work.
    let other = h.work().await;
    h.comment(
        &other,
        &h.pseud("c2").await,
        "theirs",
        "2026-01-02T00:00:00+00:00",
    )
    .await;
    let series_id = uuid::Uuid::new_v4().to_string();
    h.exec(&format!(
        "INSERT INTO comments (id, subject_type, subject_id, author_pseud, body, body_version, created_at) \
         VALUES ('{}', 'series', '{series_id}', '{}', 'on a series', '1', '2026-01-03T00:00:00+00:00')",
        uuid::Uuid::new_v4(),
        h.pseud("c3").await
    ))
    .await;

    let report = wd::migrate_comments_to_topic(h.db(), &work, &category, &author, "Thread")
        .await
        .unwrap();
    assert_eq!(report.moved, 1, "only this work's work-comments move");
    assert_eq!(
        h.post_bodies(&report.topic_id).await,
        vec!["mine".to_string()]
    );
}

#[tokio::test]
async fn a_work_with_comments_can_still_have_its_topic_created_once() {
    // The migration calls `link_topic` first, so a work that already has a
    // linked topic must reuse it rather than making a second one.
    let h = Db::new("mig-existing-link").await;
    let work = h.work().await;
    let category = h.category().await;
    let author = h.pseud("mover").await;
    let existing = wd::link_topic(h.db(), &work, None, &category, &author, "Already here")
        .await
        .unwrap();
    h.comment(
        &work,
        &h.pseud("c1").await,
        "hello",
        "2026-01-01T00:00:00+00:00",
    )
    .await;

    let report = wd::migrate_comments_to_topic(h.db(), &work, &category, &author, "Thread")
        .await
        .unwrap();

    assert_eq!(report.topic_id, existing.topic_id);
    assert_eq!(h.topic_count().await, 1);
    assert_eq!(
        h.post_bodies(&report.topic_id).await,
        vec!["hello".to_string()]
    );
}
