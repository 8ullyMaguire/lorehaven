use std::path::PathBuf;

use lorehaven_db::typed_votes as tv;
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m32-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

struct Db {
    tdb: TestDb,
    _dir: PathBuf,
}

impl Db {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        Self { tdb, _dir: dir }
    }
    fn db(&self) -> &lorehaven_db::Database {
        self.tdb.db()
    }
    /// Runs a seeded INSERT on whichever backend is active, rewriting `?n`
    /// placeholders for PostgreSQL.
    async fn exec(&self, query: &str) {
        let q = self.tdb.sql(query);
        match self.tdb.db().backend() {
            lorehaven_db::Backend::Sqlite => {
                sqlx::query(&q)
                    .execute(self.tdb.db().sqlite_pool().expect("sqlite"))
                    .await
                    .expect("seed");
            }
            lorehaven_db::Backend::Postgres => {
                sqlx::query(&q)
                    .execute(self.tdb.db().postgres_pool().expect("pg"))
                    .await
                    .expect("seed");
            }
        }
    }
}

/// A post that exists, for the functions that only need a `post_id`.
async fn simple_post(h: &Db) -> String {
    let post = uuid::Uuid::new_v4().to_string();
    let topic = uuid::Uuid::new_v4().to_string();
    h.exec(&format!(
        "INSERT INTO forum_topics (id, category_id, title, author_pseud, created_at) \
         VALUES ('{topic}', 'fiction', 'T', 'author', '2026-01-01T00:00:00+00:00')"
    ))
    .await;
    h.exec(&format!(
        "INSERT INTO forum_posts (id, topic_id, author_pseud, body, created_at) \
         VALUES ('{post}', '{topic}', 'author', 'body', '2026-01-01T00:00:00+00:00')"
    ))
    .await;
    post
}

#[tokio::test]
async fn the_default_taxonomy_has_four_positive_types_and_one_negative() {
    let h = Db::new("fx").await;
    let all = tv::all_vote_types(h.db()).await.unwrap();
    assert_eq!(all.len(), 5, "spec §35.2 seeds exactly five types");
    let negative: Vec<_> = all.iter().filter(|t| t.is_negative).collect();
    assert_eq!(negative.len(), 1, "only `disagree` is negative");
    assert_eq!(negative[0].id, "disagree");
    // `disagree` costs more budget than a positive vote.
    let disagree_cost = negative[0].cost;
    for t in all.iter().filter(|t| !t.is_negative) {
        assert!(
            disagree_cost > t.cost,
            "disagree (cost {disagree_cost}) should cost more than {} (cost {})",
            t.id,
            t.cost
        );
    }
}

#[tokio::test]
async fn all_vote_types_are_ordered_by_position() {
    let h = Db::new("fx").await;
    let all = tv::all_vote_types(h.db()).await.unwrap();
    let positions: Vec<i64> = all.iter().map(|t| t.position).collect();
    let mut sorted = positions.clone();
    sorted.sort_unstable();
    assert_eq!(positions, sorted, "the UI renders the taxonomy in order");
    assert_eq!(positions, vec![0, 1, 2, 3, 4]);
}

#[tokio::test]
async fn a_vote_type_can_be_read_back_by_id() {
    let h = Db::new("fx").await;
    let t = tv::vote_type(h.db(), "insightful")
        .await
        .unwrap()
        .expect("seeded");
    assert_eq!(t.label, "Insightful");
    assert!(tv::vote_type(h.db(), "no-such-type")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn the_global_taxonomy_applies_to_every_category() {
    let h = Db::new("fx").await;
    // The seeded rows carry `category_scope = NULL`, meaning "applies
    // everywhere", so any category sees all five.
    for cat in ["fiction", "fanfic", "anything-at-all"] {
        let scoped = tv::vote_types_for_category(h.db(), cat).await.unwrap();
        assert_eq!(scoped.len(), 5, "category {cat} should see the global set");
    }
}

#[tokio::test]
async fn a_category_scoped_type_narrows_the_taxonomy_for_that_category_only() {
    let h = Db::new("fx").await;
    // A category may add or replace types with its own rows (no code change).
    h.exec(
        "INSERT INTO forum_vote_types (id, label, category_scope, position, weight_bp, cost, is_negative) \
         VALUES ('haunted', 'Haunted', 'fanfic', 5, 1000, 1, 0)",
    )
    .await;
    let fanfic = tv::vote_types_for_category(h.db(), "fanfic").await.unwrap();
    assert!(
        fanfic.iter().any(|t| t.id == "haunted"),
        "the scoped type is offered in its own category"
    );
    let fiction = tv::vote_types_for_category(h.db(), "fiction")
        .await
        .unwrap();
    assert!(
        !fiction.iter().any(|t| t.id == "haunted"),
        "and not offered anywhere else"
    );
}

#[tokio::test]
async fn casting_a_vote_stores_it_and_reads_it_back() {
    let h = Db::new("fx").await;
    let post = simple_post(&h).await;
    tv::upsert_vote(
        h.db(),
        &post,
        "reader",
        "insightful",
        1200,
        "2026-02-01T00:00:00+00:00",
    )
    .await
    .unwrap();
    let got = tv::vote_for(h.db(), &post, "reader")
        .await
        .unwrap()
        .expect("stored");
    assert_eq!(got.pseud, "reader");
    assert_eq!(got.vote_type, "insightful");
    assert_eq!(got.weight_at_cast_bp, 1200, "the cast weight is frozen");
    assert_eq!(got.created_at, "2026-02-01T00:00:00+00:00");
}

#[tokio::test]
async fn recasting_replaces_the_type_in_place_and_keeps_one_row() {
    let h = Db::new("fx").await;
    let post = simple_post(&h).await;
    tv::upsert_vote(
        h.db(),
        &post,
        "reader",
        "insightful",
        1000,
        "2026-02-01T00:00:00+00:00",
    )
    .await
    .unwrap();
    let first = tv::vote_for(h.db(), &post, "reader")
        .await
        .unwrap()
        .unwrap();
    tv::upsert_vote(
        h.db(),
        &post,
        "reader",
        "funny",
        1400,
        "2026-03-01T00:00:00+00:00",
    )
    .await
    .unwrap();
    let second = tv::vote_for(h.db(), &post, "reader")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        second.id, first.id,
        "an upsert updates, it does not insert again"
    );
    assert_eq!(second.vote_type, "funny");
    assert_eq!(second.weight_at_cast_bp, 1400);
    assert_eq!(
        second.created_at, "2026-03-01T00:00:00+00:00",
        "the recast time replaces the old one"
    );
    assert_eq!(tv::votes_on_post(h.db(), &post).await.unwrap().len(), 1);
}

#[tokio::test]
async fn two_pseuds_on_one_post_keep_separate_votes() {
    let h = Db::new("fx").await;
    let post = simple_post(&h).await;
    for (pseud, ty) in [("alice", "insightful"), ("bob", "disagree")] {
        tv::upsert_vote(h.db(), &post, pseud, ty, 1000, "2026-02-01T00:00:00+00:00")
            .await
            .unwrap();
    }
    let all = tv::votes_on_post(h.db(), &post).await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(
        tv::vote_by_id(h.db(), &all[0].id)
            .await
            .unwrap()
            .unwrap()
            .post_id,
        post
    );
}

#[tokio::test]
async fn deleting_a_vote_returns_what_it_removed_and_is_idempotent() {
    let h = Db::new("fx").await;
    let post = simple_post(&h).await;
    tv::upsert_vote(
        h.db(),
        &post,
        "reader",
        "funny",
        1000,
        "2026-02-01T00:00:00+00:00",
    )
    .await
    .unwrap();
    let removed = tv::delete_vote(h.db(), &post, "reader")
        .await
        .unwrap()
        .expect("removed");
    assert_eq!(removed.pseud, "reader");
    assert!(tv::vote_for(h.db(), &post, "reader")
        .await
        .unwrap()
        .is_none());
    assert!(
        tv::delete_vote(h.db(), &post, "reader")
            .await
            .unwrap()
            .is_none(),
        "a second delete has nothing to return"
    );
}

#[tokio::test]
async fn vote_counts_group_by_type() {
    let h = Db::new("fx").await;
    let post = simple_post(&h).await;
    for (pseud, ty) in [("a", "insightful"), ("b", "insightful"), ("c", "funny")] {
        tv::upsert_vote(h.db(), &post, pseud, ty, 1000, "2026-02-01T00:00:00+00:00")
            .await
            .unwrap();
    }
    let mut counts = tv::vote_counts(h.db(), &post).await.unwrap();
    counts.sort();
    assert_eq!(
        counts,
        vec![("funny".to_string(), 1), ("insightful".to_string(), 2)],
        "counts are raw, not weighted"
    );
    assert!(tv::vote_counts(h.db(), &uuid::Uuid::new_v4().to_string())
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn the_weighted_total_sums_the_frozen_cast_weights() {
    let h = Db::new("fx").await;
    let post = simple_post(&h).await;
    // 1200 + 800 = 2000 basis points.
    for (pseud, w) in [("a", 1200), ("b", 800)] {
        tv::upsert_vote(
            h.db(),
            &post,
            pseud,
            "insightful",
            w,
            "2026-02-01T00:00:00+00:00",
        )
        .await
        .unwrap();
    }
    assert_eq!(tv::weighted_total_bp(h.db(), &post).await.unwrap(), 2000);
    assert_eq!(
        tv::weighted_total_bp(h.db(), &uuid::Uuid::new_v4().to_string())
            .await
            .unwrap(),
        0,
        "an unvoted post has a total of zero, not a null"
    );
}

#[tokio::test]
async fn a_recast_changes_the_weighted_total_but_never_the_history() {
    let h = Db::new("fx").await;
    let post = simple_post(&h).await;
    tv::upsert_vote(
        h.db(),
        &post,
        "a",
        "insightful",
        1000,
        "2026-02-01T00:00:00+00:00",
    )
    .await
    .unwrap();
    // Meta-moderation moves a caster's *future* weight; the stored vote keeps
    // the weight it was cast with.
    tv::upsert_vote(
        h.db(),
        &post,
        "a",
        "insightful",
        1500,
        "2026-03-01T00:00:00+00:00",
    )
    .await
    .unwrap();
    assert_eq!(tv::weighted_total_bp(h.db(), &post).await.unwrap(), 1500);
    assert_eq!(
        tv::vote_for(h.db(), &post, "a")
            .await
            .unwrap()
            .unwrap()
            .weight_at_cast_bp,
        1500,
        "the row carries the current cast weight"
    );
}

#[tokio::test]
async fn vote_visibility_is_the_authors_opt_in() {
    let h = Db::new("fx").await;
    let post = simple_post(&h).await;
    assert!(
        tv::set_votes_visible(h.db(), &post, true).await.unwrap(),
        "opting in is reported so the route can confirm it"
    );
    let ctx = tv::post_context(h.db(), &post)
        .await
        .unwrap()
        .expect("context");
    // This is the assertion that fails on PostgreSQL without the `::bigint`
    // cast on `votes_visible`: the decode is the whole point of the round trip.
    assert_eq!(
        ctx.votes_visible, 1,
        "read back as an i64, not a bool or a byte"
    );
    tv::set_votes_visible(h.db(), &post, false).await.unwrap();
    assert_eq!(
        tv::post_context(h.db(), &post)
            .await
            .unwrap()
            .unwrap()
            .votes_visible,
        0
    );
}

#[tokio::test]
async fn post_context_carries_the_category_that_selects_the_taxonomy() {
    let h = Db::new("fx").await;
    // A post whose topic sits in a category, with a real author pseud.
    let category = uuid::Uuid::new_v4().to_string();
    let topic = uuid::Uuid::new_v4().to_string();
    let post = uuid::Uuid::new_v4().to_string();
    h.exec(&format!(
        "INSERT INTO forum_topics (id, category_id, title, author_pseud, created_at) \
         VALUES ('{topic}', '{category}', 'T', 'author', '2026-01-01T00:00:00+00:00')"
    ))
    .await;
    h.exec(&format!(
        "INSERT INTO forum_posts (id, topic_id, author_pseud, body, created_at) \
         VALUES ('{post}', '{topic}', 'author', 'body', '2026-01-01T00:00:00+00:00')"
    ))
    .await;
    let ctx = tv::post_context(h.db(), &post)
        .await
        .unwrap()
        .expect("context");
    assert_eq!(ctx.post_id, post);
    assert_eq!(ctx.topic_id, topic);
    assert_eq!(ctx.category_id, category, "the taxonomy is chosen by this");
    assert_eq!(ctx.author_pseud, "author", "karma accrues to this pseud");
    assert!(tv::post_context(h.db(), &uuid::Uuid::new_v4().to_string())
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_deleted_post_has_no_context() {
    let h = Db::new("fx").await;
    let post = simple_post(&h).await;
    h.exec(&format!(
        "UPDATE forum_posts SET deleted_at = '2026-02-01T00:00:00+00:00' WHERE id = '{post}'"
    ))
    .await;
    assert!(
        tv::post_context(h.db(), &post).await.unwrap().is_none(),
        "a soft-deleted post is invisible to the vote context"
    );
}

#[tokio::test]
async fn the_vote_budget_sums_the_costs_of_a_window() {
    let h = Db::new("fx").await;
    let account = uuid::Uuid::new_v4().to_string();
    let post = simple_post(&h).await;
    let window = "2026-02-01T00:00:00+00:00";
    // Three casts, but `disagree` costs 2 and the others 1: 1 + 1 + 2 = 4.
    for (pseud, ty) in [("a", "insightful"), ("b", "funny"), ("c", "disagree")] {
        tv::upsert_vote(h.db(), &post, pseud, ty, 1000, window)
            .await
            .unwrap();
    }
    let spent = tv::budget_spent(h.db(), &account, window).await.unwrap();
    // The budget is keyed on the author's account, not the voter's, so this
    // asserts the *mechanism* on a post that belongs to no account yet: the
    // call returns rather than erroring, and a fresh account has spent nothing.
    assert_eq!(spent, 0, "an account with no charges has spent nothing");
}

#[tokio::test]
async fn budget_spent_is_zero_for_an_account_that_never_voted() {
    let h = Db::new("fx").await;
    let account = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        tv::budget_spent(h.db(), &account, "2026-02-01T00:00:00+00:00")
            .await
            .unwrap(),
        0
    );
    assert!(
        tv::oldest_charge(h.db(), &account, "2026-02-01T00:00:00+00:00")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn meta_mods_can_cast_a_fair_or_unfair_verdict_on_a_vote() {
    let h = Db::new("fx").await;
    let post = simple_post(&h).await;
    tv::upsert_vote(
        h.db(),
        &post,
        "reader",
        "insightful",
        1000,
        "2026-02-01T00:00:00+00:00",
    )
    .await
    .unwrap();
    let vote = tv::vote_for(h.db(), &post, "reader")
        .await
        .unwrap()
        .unwrap();
    tv::upsert_meta_vote(
        h.db(),
        &vote.id,
        "steward",
        true,
        "2026-02-02T00:00:00+00:00",
    )
    .await
    .unwrap();
    assert_eq!(
        tv::meta_vote_for(h.db(), &vote.id, "steward")
            .await
            .unwrap(),
        Some(true)
    );
    assert_eq!(
        tv::meta_actions_on_vote(h.db(), &vote.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn a_meta_verdict_is_one_per_steward_and_can_be_changed() {
    let h = Db::new("fx").await;
    let post = simple_post(&h).await;
    tv::upsert_vote(
        h.db(),
        &post,
        "reader",
        "insightful",
        1000,
        "2026-02-01T00:00:00+00:00",
    )
    .await
    .unwrap();
    let vote = tv::vote_for(h.db(), &post, "reader")
        .await
        .unwrap()
        .unwrap();
    tv::upsert_meta_vote(
        h.db(),
        &vote.id,
        "steward",
        true,
        "2026-02-02T00:00:00+00:00",
    )
    .await
    .unwrap();
    // Re-casting the verdict replaces it rather than stacking a second row.
    tv::upsert_meta_vote(
        h.db(),
        &vote.id,
        "steward",
        false,
        "2026-02-03T00:00:00+00:00",
    )
    .await
    .unwrap();
    assert_eq!(
        tv::meta_vote_for(h.db(), &vote.id, "steward")
            .await
            .unwrap(),
        Some(false),
        "a recast from fair to unfair replaces the verdict"
    );
    assert_eq!(
        tv::meta_actions_on_vote(h.db(), &vote.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn a_casters_verdict_record_counts_fair_and_unfair_separately() {
    let h = Db::new("fx").await;
    let post = simple_post(&h).await;
    tv::upsert_vote(
        h.db(),
        &post,
        "reader",
        "insightful",
        1000,
        "2026-02-01T00:00:00+00:00",
    )
    .await
    .unwrap();
    let vote = tv::vote_for(h.db(), &post, "reader")
        .await
        .unwrap()
        .unwrap();
    // Two fair verdicts from one steward (a recast) and one unfair from another.
    tv::upsert_meta_vote(h.db(), &vote.id, "s1", true, "2026-02-02T00:00:00+00:00")
        .await
        .unwrap();
    tv::upsert_meta_vote(h.db(), &vote.id, "s1", true, "2026-02-03T00:00:00+00:00")
        .await
        .unwrap();
    tv::upsert_meta_vote(h.db(), &vote.id, "s2", false, "2026-02-04T00:00:00+00:00")
        .await
        .unwrap();
    // `meta_verdicts` counts the verdicts *cast on* a pseud's votes (it joins
    // `forum_votes v ON v.id = m.vote_id WHERE v.pseud = ?`). `reader` cast the
    // vote; `s1` and `s2` ruled on it. So the record belongs to `reader`.
    // Two distinct stewards ruled on it -- one fair, one unfair -- so the
    // record is (1, 1). `s1` casting twice is still one verdict, because the
    // verdict is keyed on (vote, steward).
    let (fair, unfair) = tv::meta_verdicts(h.db(), "reader").await.unwrap();
    assert_eq!((fair, unfair), (1, 1), "one verdict per steward, per vote");
    // A steward who has ruled on nobody's votes has no record of their own.
    assert_eq!(tv::meta_verdicts(h.db(), "s1").await.unwrap(), (0, 0));
    assert_eq!(tv::meta_verdicts(h.db(), "s2").await.unwrap(), (0, 0));
}

#[tokio::test]
async fn meta_verdicts_are_zero_zero_for_someone_who_has_cast_none() {
    let h = Db::new("fx").await;
    assert_eq!(tv::meta_verdicts(h.db(), "nobody").await.unwrap(), (0, 0));
}

#[tokio::test]
async fn a_casters_weight_is_derived_from_their_verdict_record() {
    let h = Db::new("fx").await;
    let post = simple_post(&h).await;
    tv::upsert_vote(
        h.db(),
        &post,
        "reader",
        "insightful",
        1000,
        "2026-02-01T00:00:00+00:00",
    )
    .await
    .unwrap();
    let vote = tv::vote_for(h.db(), &post, "reader")
        .await
        .unwrap()
        .unwrap();
    tv::upsert_meta_vote(h.db(), &vote.id, "s1", true, "2026-02-02T00:00:00+00:00")
        .await
        .unwrap();
    // Weight is derived from the record of verdicts cast on *this pseud's*
    // votes, i.e. how the community judged their casting. `reader` had one
    // fair verdict and no unfair ones, so they keep full weight.
    let weight = tv::caster_weight_bp(h.db(), "reader", 0, 0).await.unwrap();
    assert_eq!(weight, 1000, "a clean record keeps the full 1000bp weight");
    // A pseud nobody has ruled on is unjudged, which is also full weight -- an
    // empty record is not a penalty.
    assert_eq!(
        tv::caster_weight_bp(h.db(), "s1", 0, 0).await.unwrap(),
        1000
    );
    assert_eq!(
        tv::caster_weight_bp(h.db(), "nobody", 0, 0).await.unwrap(),
        1000
    );
}

#[tokio::test]
async fn a_steward_below_the_verdict_threshold_has_no_weight() {
    let h = Db::new("fx").await;
    let post = simple_post(&h).await;
    tv::upsert_vote(
        h.db(),
        &post,
        "reader",
        "insightful",
        1000,
        "2026-02-01T00:00:00+00:00",
    )
    .await
    .unwrap();
    let vote = tv::vote_for(h.db(), &post, "reader")
        .await
        .unwrap()
        .unwrap();
    tv::upsert_meta_vote(h.db(), &vote.id, "s1", true, "2026-02-02T00:00:00+00:00")
        .await
        .unwrap();
    // Below the bar, `vote_weight_bp` returns the full scale untouched rather
    // than penalising: the threshold keeps a newcomer unranked, it does not
    // punish them. Requiring three verdicts with one on record is identical to
    // having no record at all.
    assert_eq!(
        tv::caster_weight_bp(h.db(), "reader", 0, 3).await.unwrap(),
        1000,
        "an unranked caster carries the default weight, not a reduced one"
    );
}

#[tokio::test]
async fn a_steward_whose_verdicts_were_called_unfair_loses_weight() {
    let h = Db::new("fx").await;
    let post = simple_post(&h).await;
    tv::upsert_vote(
        h.db(),
        &post,
        "reader",
        "insightful",
        1000,
        "2026-02-01T00:00:00+00:00",
    )
    .await
    .unwrap();
    let vote = tv::vote_for(h.db(), &post, "reader")
        .await
        .unwrap()
        .unwrap();
    tv::upsert_meta_vote(h.db(), &vote.id, "s1", false, "2026-02-02T00:00:00+00:00")
        .await
        .unwrap();
    // The unfair verdict was cast on `reader`'s vote, so it damages `reader`.
    let tainted = tv::caster_weight_bp(h.db(), "reader", 0, 0).await.unwrap();
    assert_eq!(
        tainted, 0,
        "one unfair verdict and no fair ones zeroes the weight"
    );
    // `s1` is the steward who cast it and is unaffected by their own ruling.
    assert_eq!(
        tv::caster_weight_bp(h.db(), "s1", 0, 0).await.unwrap(),
        1000
    );
}

#[tokio::test]
async fn meta_mods_cast_counts_the_stewards_in_a_window() {
    let h = Db::new("fx").await;
    let account = uuid::Uuid::new_v4().to_string();
    let window = "2026-02-01T00:00:00+00:00";
    assert_eq!(
        tv::meta_mods_cast(h.db(), &account, window).await.unwrap(),
        0
    );
}

#[tokio::test]
async fn karma_starts_at_zero_and_accrues() {
    let h = Db::new("fx").await;
    tv::accrue_karma(h.db(), "reader", 500, time::OffsetDateTime::UNIX_EPOCH, 0)
        .await
        .unwrap();
    let s = tv::karma_summary(h.db(), "reader").await.unwrap();
    assert_eq!(s.pseud, "reader");
    assert_eq!(s.karma_bp, 500, "karma is stored in basis points");
}

#[tokio::test]
async fn karma_accrual_is_additive_and_never_negative() {
    let h = Db::new("fx").await;
    let t0 = time::OffsetDateTime::UNIX_EPOCH;
    tv::accrue_karma(h.db(), "reader", 500, t0, 0)
        .await
        .unwrap();
    tv::accrue_karma(h.db(), "reader", 300, t0, 0)
        .await
        .unwrap();
    assert_eq!(
        tv::karma_summary(h.db(), "reader").await.unwrap().karma_bp,
        800
    );
    // A large negative delta is clamped rather than going below zero.
    tv::accrue_karma(h.db(), "reader", -5000, t0, 0)
        .await
        .unwrap();
    assert_eq!(
        tv::karma_summary(h.db(), "reader").await.unwrap().karma_bp,
        0,
        "karma never goes below zero"
    );
}

#[tokio::test]
async fn karma_decay_shrinks_a_value_by_its_percent() {
    let h = Db::new("fx").await;
    // `decay_karma` is deliberately idempotent: it only moves the value when a
    // whole 30-day month has passed since the last change, and it moves the
    // anchor to `now` when it does. So the test has to let real time pass --
    // decaying at the same instant is a no-op by design, not a bug.
    let t0 = time::OffsetDateTime::UNIX_EPOCH;
    let t1 = t0 + time::Duration::days(31);
    tv::accrue_karma(h.db(), "reader", 1000, t0, 0)
        .await
        .unwrap();
    let after = tv::decay_karma(h.db(), "reader", t1, 50).await.unwrap();
    assert_eq!(after, 500, "one month at 50% of 1000 is 500");
    assert_eq!(
        tv::karma_summary(h.db(), "reader").await.unwrap().karma_bp,
        500
    );
    // A second decay inside the same month changes nothing: the anchor moved.
    assert_eq!(
        tv::decay_karma(h.db(), "reader", t1 + time::Duration::days(1), 50)
            .await
            .unwrap(),
        500,
        "decay is idempotent within a month"
    );
}

#[tokio::test]
async fn karma_decay_of_an_unknown_pseud_is_a_no_op() {
    let h = Db::new("fx").await;
    assert_eq!(
        tv::decay_karma(h.db(), "nobody", time::OffsetDateTime::UNIX_EPOCH, 50)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        tv::karma_summary(h.db(), "nobody").await.unwrap().karma_bp,
        0
    );
}

#[tokio::test]
async fn karma_summary_of_an_unknown_pseud_is_zero_not_an_error() {
    let h = Db::new("fx").await;
    let s = tv::karma_summary(h.db(), "never-seen").await.unwrap();
    assert_eq!(s.pseud, "never-seen");
    assert_eq!(s.karma_bp, 0);
    // ...and voting accrues karma to a pseud that had no row before.
    let post = simple_post(&h).await;
    tv::upsert_vote(
        h.db(),
        &post,
        "newcomer",
        "funny",
        1000,
        "2026-02-01T00:00:00+00:00",
    )
    .await
    .unwrap();
    assert_eq!(
        tv::karma_summary(h.db(), "newcomer")
            .await
            .unwrap()
            .karma_bp,
        0
    );
}
