//! M34 — Spoilers, content warnings, drafts and scheduled posts (spec §35.4).
//!
//! `crates/db/src/spoilers.rs` has **thirteen public functions and no tests at
//! all**, and all four M34 requirements in `docs/requirements.csv` claimed
//! `implemented-locally-tested` while citing no test whatsoever.
//!
//! Most of this file is `ON CONFLICT ... DO UPDATE` upserts, and an upsert has a
//! failure that an insert test cannot see: nothing proves the *second* write
//! updated the row rather than colliding with it, and a wrong conflict target
//! raises a constraint error on the second call only. So every upsert here is
//! written twice, and the assertions are about the value after the second call.
//!
//! The other thing worth pinning down is that a reader's warning preferences are
//! *per type*: two types must not collapse into one row, and changing one must
//! not disturb the other.

use std::path::PathBuf;

use lorehaven_db::spoilers as sp;
use lorehaven_domain::spoilers::{WarningAction, WarningType};
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m34-{tag}-{}-{:?}",
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
}

/// Seed an account, a work and a topic, and return all three ids.
///
/// `accounts.id` and `works.id` are UUID on PostgreSQL and TEXT on SQLite, so
/// the ids are real UUIDs and the two dialects bind them differently. A readable
/// name would pass SQLite and be rejected as a malformed uuid by PostgreSQL.
async fn seed_world(h: &Db) -> World {
    let account = uuid::Uuid::new_v4().to_string();
    let work = uuid::Uuid::new_v4().to_string();
    let topic = uuid::Uuid::new_v4().to_string();
    let pseud = uuid::Uuid::new_v4().to_string();
    let post = uuid::Uuid::new_v4().to_string();

    match h.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, ?, ?)",
            )
            .bind(&account)
            .bind(format!("{account}@m34.test"))
            .bind("2026-01-01T00:00:00Z")
            .bind("2026-01-01T00:00:00Z")
            .execute(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("seed account");

            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&pseud)
            .bind(&account)
            .bind(format!("p{}", &account[..8]))
            .bind("A Writer")
            .bind("2026-01-01T00:00:00Z")
            .bind("2026-01-01T00:00:00Z")
            .execute(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("seed pseud");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at)
                 VALUES ($1::uuid, $2, $3, $4)",
            )
            .bind(&account)
            .bind(format!("{account}@m34.test"))
            .bind("2026-01-01T00:00:00Z")
            .bind("2026-01-01T00:00:00Z")
            .execute(h.db().postgres_pool().expect("postgres"))
            .await
            .expect("seed account");

            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)",
            )
            .bind(&pseud)
            .bind(&account)
            .bind(format!("p{}", &account[..8]))
            .bind("A Writer")
            .bind("2026-01-01T00:00:00Z")
            .bind("2026-01-01T00:00:00Z")
            .execute(h.db().postgres_pool().expect("postgres"))
            .await
            .expect("seed pseud");
        }
    }

    // A work row is needed for anything that references one.
    match h.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, visibility, lifecycle, created_at, updated_at)
                 VALUES (?, ?, ?, 'public', 'published', ?, ?)",
            )
            .bind(&work)
            .bind(&pseud)
            .bind("A Work")
            .bind("2026-01-01T00:00:00Z")
            .bind("2026-01-01T00:00:00Z")
            .execute(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("seed work");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, visibility, lifecycle, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, $3, 'public', 'published', $4, $5)",
            )
            .bind(&work)
            .bind(&pseud)
            .bind("A Work")
            .bind("2026-01-01T00:00:00Z")
            .bind("2026-01-01T00:00:00Z")
            .execute(h.db().postgres_pool().expect("postgres"))
            .await
            .expect("seed work");
        }
    }

    // After the work row: `reader_work_progress` has a foreign key to it, and on
    // PostgreSQL that is a violation rather than the dangling row SQLite allows.
    sp::upsert_reader_progress(h.db(), &account, &work, 0)
        .await
        .expect("seed progress");

    lorehaven_db::thread_modes::create_topic(
        h.db(),
        &topic,
        "general",
        &pseud,
        "A thread",
        "plain",
    )
    .await
    .expect("seed topic");

    match h.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO forum_posts (id, topic_id, author_pseud, body, created_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&post)
            .bind(&topic)
            .bind(&pseud)
            .bind("A post.")
            .bind("2026-01-01T00:00:00Z")
            .execute(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("seed post");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO forum_posts (id, topic_id, author_pseud, body, created_at)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(&post)
            .bind(&topic)
            .bind(&pseud)
            .bind("A post.")
            .bind("2026-01-01T00:00:00Z")
            .execute(h.db().postgres_pool().expect("postgres"))
            .await
            .expect("seed post");
        }
    }

    World {
        account,
        work,
        topic,
        post,
    }
}

struct World {
    account: String,
    work: String,
    topic: String,
    post: String,
}

// ---------------------------------------------------------------------------
// Reader progress through a work (M34-01)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reader_progress_is_upserted_not_duplicated() {
    let h = Db::new("progress").await;
    let w = seed_world(&h).await;

    assert_eq!(
        sp::get_reader_progress(h.db(), &w.account, &w.work)
            .await
            .expect("first"),
        Some(0)
    );

    sp::upsert_reader_progress(h.db(), &w.account, &w.work, 7)
        .await
        .expect("update progress");

    // The whole point of the upsert: one row, carrying the later value. A test
    // that only inserted once would pass against a query with no conflict clause
    // at all.
    assert_eq!(
        sp::get_reader_progress(h.db(), &w.account, &w.work)
            .await
            .expect("second"),
        Some(7),
        "progress moved forward, in place"
    );
}

#[tokio::test]
async fn reader_progress_is_absent_for_a_work_that_was_never_started() {
    let h = Db::new("progress-absent").await;
    let w = seed_world(&h).await;

    let other = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        sp::get_reader_progress(h.db(), &w.account, &other)
            .await
            .expect("absent"),
        None,
        "an unstarted work reads as None, not 0"
    );
}

#[tokio::test]
async fn two_accounts_keep_separate_progress_through_one_work() {
    let h = Db::new("progress-scope").await;
    let w = seed_world(&h).await;
    let second_account = uuid::Uuid::new_v4().to_string();

    // Only the first account exists as a row, and on PostgreSQL
    // `reader_work_progress.account` is a UUID with a foreign key, so the second
    // reader has to be a real account rather than a made-up id.
    match h.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?, ?, ?, ?)",
            )
            .bind(&second_account)
            .bind(format!("{second_account}@m34.test"))
            .bind("2026-01-01T00:00:00Z")
            .bind("2026-01-01T00:00:00Z")
            .execute(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("seed second account");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at)
                 VALUES ($1::uuid, $2, $3, $4)",
            )
            .bind(&second_account)
            .bind(format!("{second_account}@m34.test"))
            .bind("2026-01-01T00:00:00Z")
            .bind("2026-01-01T00:00:00Z")
            .execute(h.db().postgres_pool().expect("postgres"))
            .await
            .expect("seed second account");
        }
    }

    sp::upsert_reader_progress(h.db(), &w.account, &w.work, 3)
        .await
        .expect("first reader");
    sp::upsert_reader_progress(h.db(), &second_account, &w.work, 11)
        .await
        .expect("second reader");

    assert_eq!(
        sp::get_reader_progress(h.db(), &w.account, &w.work)
            .await
            .expect("first"),
        Some(3)
    );
    assert_eq!(
        sp::get_reader_progress(h.db(), &second_account, &w.work)
            .await
            .expect("second"),
        Some(11),
        "one reader's progress is not another's"
    );
}

// ---------------------------------------------------------------------------
// Topic spoiler scope (M34-01)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_topics_spoiler_scope_can_be_set_and_cleared() {
    let h = Db::new("scope").await;
    let w = seed_world(&h).await;

    sp::set_topic_spoiler_scope(h.db(), &w.topic, Some(4))
        .await
        .expect("set scope");

    let scope: Option<i64> = match h.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query_scalar("SELECT spoiler_scope_chapter FROM forum_topics WHERE id = ?")
                .bind(&w.topic)
                .fetch_one(h.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("read scope")
        }
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(
            "SELECT CAST(spoiler_scope_chapter AS BIGINT) FROM forum_topics WHERE id = $1",
        )
        .bind(&w.topic)
        .fetch_one(h.db().postgres_pool().expect("postgres"))
        .await
        .expect("read scope"),
    };
    assert_eq!(scope, Some(4), "spoilers are hidden from chapter 4");

    // Clearing is `None`, and it has to actually clear rather than write a 0 --
    // chapter 0 is a real chapter and a scope of 0 would hide the opening.
    sp::set_topic_spoiler_scope(h.db(), &w.topic, None)
        .await
        .expect("clear scope");

    let scope: Option<i64> = match h.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query_scalar("SELECT spoiler_scope_chapter FROM forum_topics WHERE id = ?")
                .bind(&w.topic)
                .fetch_one(h.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("read cleared scope")
        }
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(
            "SELECT CAST(spoiler_scope_chapter AS BIGINT) FROM forum_topics WHERE id = $1",
        )
        .bind(&w.topic)
        .fetch_one(h.db().postgres_pool().expect("postgres"))
        .await
        .expect("read cleared scope"),
    };
    assert_eq!(scope, None, "None clears; it does not become 0");
}

// ---------------------------------------------------------------------------
// Content warnings (M34-02)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_content_warning_is_recorded_and_read_back() {
    let h = Db::new("warning").await;
    let w = seed_world(&h).await;

    let id = sp::add_content_warning(h.db(), &w.post, WarningType::Violence, 2, None)
        .await
        .expect("add warning");

    let listed = sp::list_content_warnings(h.db(), &w.post)
        .await
        .expect("list");
    assert_eq!(listed.len(), 1, "one warning on the post");
    assert_eq!(listed[0].warning_type, WarningType::Violence.as_str());
    assert_eq!(listed[0].severity, 2);
    assert!(!id.is_empty(), "the insert returns the new id");
}

#[tokio::test]
async fn several_warnings_on_one_post_are_all_kept() {
    let h = Db::new("warning-many").await;
    let w = seed_world(&h).await;

    sp::add_content_warning(h.db(), &w.post, WarningType::Violence, 1, None)
        .await
        .expect("violence");
    sp::add_content_warning(h.db(), &w.post, WarningType::SexualContent, 3, None)
        .await
        .expect("sexual content");

    let listed = sp::list_content_warnings(h.db(), &w.post)
        .await
        .expect("list");
    assert_eq!(listed.len(), 2, "warnings accumulate rather than replace");
}

#[tokio::test]
async fn a_custom_warning_carries_its_own_text() {
    let h = Db::new("warning-custom").await;
    let w = seed_world(&h).await;

    sp::add_content_warning(
        h.db(),
        &w.post,
        WarningType::Custom,
        1,
        Some("Animal death"),
    )
    .await
    .expect("add custom");

    let listed = sp::list_content_warnings(h.db(), &w.post)
        .await
        .expect("list");
    assert_eq!(listed[0].warning_type, WarningType::Custom.as_str());
    assert_eq!(
        listed[0].custom_text.as_deref(),
        Some("Animal death"),
        "a Custom warning is meaningless without its text"
    );
}

#[tokio::test]
async fn a_post_with_no_warnings_lists_empty() {
    let h = Db::new("warning-none").await;
    let w = seed_world(&h).await;

    assert!(sp::list_content_warnings(h.db(), &w.post)
        .await
        .expect("list")
        .is_empty());
}

// ---------------------------------------------------------------------------
// Warning preferences (M34-02)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_warning_preference_is_upserted_rather_than_duplicated() {
    let h = Db::new("pref").await;
    let w = seed_world(&h).await;

    sp::set_warning_pref(
        h.db(),
        &w.account,
        WarningType::Violence,
        WarningAction::Blur,
    )
    .await
    .expect("set hide");
    sp::set_warning_pref(
        h.db(),
        &w.account,
        WarningType::Violence,
        WarningAction::Show,
    )
    .await
    .expect("set reveal");

    let prefs = sp::list_warning_prefs(h.db(), &w.account)
        .await
        .expect("list prefs");
    assert_eq!(prefs.len(), 1, "one preference per warning type");
    assert_eq!(prefs[0], (WarningType::Violence, WarningAction::Show));
}

#[tokio::test]
async fn preferences_for_different_warning_types_do_not_collapse() {
    let h = Db::new("pref-types").await;
    let w = seed_world(&h).await;

    // The conflict target is (account, warning_type). A wrong target -- on
    // (account, action), say -- would make this second write overwrite the first
    // and leave one row instead of two.
    sp::set_warning_pref(
        h.db(),
        &w.account,
        WarningType::Violence,
        WarningAction::Blur,
    )
    .await
    .expect("violence");
    sp::set_warning_pref(
        h.db(),
        &w.account,
        WarningType::SexualContent,
        WarningAction::Show,
    )
    .await
    .expect("sexual content");

    let prefs = sp::list_warning_prefs(h.db(), &w.account)
        .await
        .expect("list prefs");
    assert_eq!(prefs.len(), 2, "two types, two rows");
    assert!(prefs.contains(&(WarningType::Violence, WarningAction::Blur)));
    assert!(prefs.contains(&(WarningType::SexualContent, WarningAction::Show)));
}

#[tokio::test]
async fn changing_one_preference_leaves_the_others_alone() {
    let h = Db::new("pref-isolate").await;
    let w = seed_world(&h).await;

    // Both start as Blur, so a change to one is visible as an asymmetry: if the
    // upsert collapsed rows onto (account) alone, the second write would have
    // overwritten the first and both would read Show.
    sp::set_warning_pref(
        h.db(),
        &w.account,
        WarningType::Violence,
        WarningAction::Blur,
    )
    .await
    .expect("violence");
    sp::set_warning_pref(
        h.db(),
        &w.account,
        WarningType::SelfHarm,
        WarningAction::Blur,
    )
    .await
    .expect("self harm");

    sp::set_warning_pref(
        h.db(),
        &w.account,
        WarningType::Violence,
        WarningAction::Show,
    )
    .await
    .expect("change violence");

    // `list_warning_prefs` has no ORDER BY, and PostgreSQL and SQLite do not
    // agree on the order of two rows from a two-row table. Assert the contents
    // as a set -- what is under test is which value each type holds, not the
    // sequence they come back in.
    let prefs = sp::list_warning_prefs(h.db(), &w.account)
        .await
        .expect("list prefs");
    let mut keys: Vec<_> = prefs
        .iter()
        .map(|(t, a)| (t.as_str().to_string(), a.as_str().to_string()))
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            // Sorted by the stored strings, so this is order-independent on both
            // backends: "self_harm" < "violence".
            ("self_harm".to_string(), "blur".to_string()),
            ("violence".to_string(), "show".to_string()),
        ],
        "only the type that was re-set changed"
    );
}

#[tokio::test]
async fn an_account_with_no_preferences_lists_empty() {
    let h = Db::new("pref-none").await;
    let w = seed_world(&h).await;
    assert!(sp::list_warning_prefs(h.db(), &w.account)
        .await
        .expect("list")
        .is_empty());
}

// ---------------------------------------------------------------------------
// Drafts (M34-03)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_draft_round_trips_and_a_second_save_replaces_it() {
    let h = Db::new("draft").await;
    let w = seed_world(&h).await;

    sp::upsert_draft(h.db(), &w.account, &w.topic, "Half a thought")
        .await
        .expect("save draft");
    assert_eq!(
        sp::get_draft(h.db(), &w.account, &w.topic)
            .await
            .expect("read"),
        Some("Half a thought".to_string())
    );

    // Autosave means writing repeatedly, so the conflict clause is the contract.
    sp::upsert_draft(h.db(), &w.account, &w.topic, "A longer thought")
        .await
        .expect("save again");
    assert_eq!(
        sp::get_draft(h.db(), &w.account, &w.topic)
            .await
            .expect("read again"),
        Some("A longer thought".to_string()),
        "the draft is replaced, not appended to"
    );
}

#[tokio::test]
async fn a_draft_is_scoped_to_one_account_and_one_topic() {
    let h = Db::new("draft-scope").await;
    let w = seed_world(&h).await;

    sp::upsert_draft(h.db(), &w.account, &w.topic, "Mine")
        .await
        .expect("save");

    assert_eq!(
        sp::get_draft(h.db(), &w.account, &w.topic)
            .await
            .expect("mine"),
        Some("Mine".into())
    );
    assert_eq!(
        sp::get_draft(h.db(), &w.account, &uuid::Uuid::new_v4().to_string())
            .await
            .expect("other topic"),
        None,
        "another topic has no draft"
    );
}

#[tokio::test]
async fn deleting_a_draft_reports_whether_there_was_one() {
    let h = Db::new("draft-delete").await;
    let w = seed_world(&h).await;

    assert!(
        !sp::delete_draft(h.db(), &w.account, &w.topic)
            .await
            .expect("delete nothing"),
        "deleting a draft that was never saved reports false"
    );

    sp::upsert_draft(h.db(), &w.account, &w.topic, "Doomed")
        .await
        .expect("save");
    assert!(
        sp::delete_draft(h.db(), &w.account, &w.topic)
            .await
            .expect("delete"),
        "deleting an existing draft reports true"
    );
    assert_eq!(
        sp::get_draft(h.db(), &w.account, &w.topic)
            .await
            .expect("read"),
        None
    );
}

// ---------------------------------------------------------------------------
// Scheduled posts (M34-04)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scheduling_a_post_sets_its_time_and_withholds_it() {
    let h = Db::new("scheduled").await;
    let w = seed_world(&h).await;

    sp::schedule_post(h.db(), &w.post, "2026-06-01T09:00:00Z")
        .await
        .expect("schedule");

    // `forum_posts.published` is INTEGER on both dialects (migration 0041), so
    // this reads as an `i32` rather than a bool. A `bool` is a decode error on
    // PostgreSQL and a silent coercion on SQLite; an `i64` is the same trap one
    // step along, since PG types INTEGER as INT4.
    let (at, published): (Option<String>, i32) = match h.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query_as("SELECT scheduled_at, published FROM forum_posts WHERE id = ?")
                .bind(&w.post)
                .fetch_one(h.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("read scheduled post")
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query_as("SELECT scheduled_at, published FROM forum_posts WHERE id = $1")
                .bind(&w.post)
                .fetch_one(h.db().postgres_pool().expect("postgres"))
                .await
                .expect("read scheduled post")
        }
    };

    assert_eq!(
        at.as_deref(),
        Some("2026-06-01T09:00:00Z"),
        "the time is stored verbatim"
    );
    assert_eq!(published, 0, "a scheduled post is not published yet");
}

#[tokio::test]
async fn scheduling_the_same_post_twice_moves_the_time_rather_than_failing() {
    let h = Db::new("scheduled-twice").await;
    let w = seed_world(&h).await;

    sp::schedule_post(h.db(), &w.post, "2026-06-01T09:00:00Z")
        .await
        .expect("first");
    sp::schedule_post(h.db(), &w.post, "2026-07-02T10:00:00Z")
        .await
        .expect("reschedule");

    let at: Option<String> = match h.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query_scalar("SELECT scheduled_at FROM forum_posts WHERE id = ?")
                .bind(&w.post)
                .fetch_one(h.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("read")
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query_scalar("SELECT scheduled_at FROM forum_posts WHERE id = $1")
                .bind(&w.post)
                .fetch_one(h.db().postgres_pool().expect("postgres"))
                .await
                .expect("read")
        }
    };
    assert_eq!(at.as_deref(), Some("2026-07-02T10:00:00Z"));
}

#[tokio::test]
async fn scheduling_a_post_that_does_not_exist_is_a_no_op_not_an_error() {
    let h = Db::new("scheduled-missing").await;
    let _w = seed_world(&h).await;

    sp::schedule_post(
        h.db(),
        &uuid::Uuid::new_v4().to_string(),
        "2026-06-01T09:00:00Z",
    )
    .await
    .expect("an UPDATE matching zero rows is not a failure");
}
