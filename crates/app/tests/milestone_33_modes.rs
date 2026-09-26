//! M33 — Thread modes: reading-group schedules, critique queues, wiki pins.
//!
//! The domain half had five unit tests (`crates/domain/src/thread_modes.rs`,
//! all of them about the mode vocabulary parsing and round-tripping). The data
//! half had **eight public functions and no tests at all** -- `create_topic`,
//! `add_schedule_section`, `get_schedule`, `create_wiki_pin`, `approve_wiki_pin`,
//! `get_wiki_pin`, `join_critique`, `get_critique_queue`, `set_topic_mode` -- and
//! three of the four M33 requirements in `docs/requirements.csv` cited no test
//! while claiming `implemented-locally-tested`.
//!
//! These are the tests worth having, because the behaviour is subtle in two
//! places. A wiki pin is **invisible until approved**, so `get_wiki_pin` filters
//! on `approved_by IS NOT NULL` and a test that only ever creates a pin and then
//! looks for it will find nothing and conclude the write failed. And the critique
//! queue assigns `MAX(position) + 1`, which is `0` for the first member and the
//! `NULL`-vs-`-1` shape that a bare `unwrap_or_default()` would turn into a
//! second member at position 0.

use std::path::PathBuf;

use lorehaven_db::thread_modes as tm;
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m33-{tag}-{}-{:?}",
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

/// A topic with the given mode, plus a real pseud row to point at.
///
/// `forum_topics.category_id` holds a *slug* and `author_pseud` is a TEXT
/// pseud id, so this seeds by hand rather than going through the API — the same
/// shape `create_topic` binds.
/// Seed an account and a pseud, and create a topic in `mode`.
///
/// The ids are real UUIDs rather than readable strings: `pseuds.id` and
/// `accounts.id` are TEXT on SQLite but UUID on PostgreSQL, and a fake
/// `pseud-topic-mode` slides into the SQLite column and is rejected as a
/// malformed uuid by the PostgreSQL one. The topic id is likewise UUID, so it
/// is generated rather than named -- which is why the helpers that assert on
/// scope take the ids `create` hands back.
async fn topic(h: &Db, mode: &str) -> (String, String) {
    let account_id = uuid::Uuid::new_v4().to_string();
    let pseud_id = uuid::Uuid::new_v4().to_string();
    let topic_id = uuid::Uuid::new_v4().to_string();

    match h.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(&account_id)
            .bind(format!("{account_id}@m33.test"))
            .bind("2026-01-01T00:00:00Z")
            .bind("2026-01-01T00:00:00Z")
            .execute(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("seed account");

            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&pseud_id)
            .bind(&account_id)
            .bind(format!("p{}", &account_id[..8]))
            .bind("A Writer")
            .bind("2026-01-01T00:00:00Z")
            .bind("2026-01-01T00:00:00Z")
            .execute(h.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("seed pseud");
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query(
                // `accounts.id` is UUID here and TEXT on SQLite, so the bind
                // needs the cast on this side only -- the column side stays bare.
                "INSERT INTO accounts (id, email, created_at, updated_at)
                 VALUES ($1::uuid, $2, $3, $4)",
            )
            .bind(&account_id)
            .bind(format!("{account_id}@m33.test"))
            .bind("2026-01-01T00:00:00Z")
            .bind("2026-01-01T00:00:00Z")
            .execute(h.db().postgres_pool().expect("postgres"))
            .await
            .expect("seed account");

            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)",
            )
            .bind(&pseud_id)
            .bind(&account_id)
            .bind(format!("p{}", &account_id[..8]))
            .bind("A Writer")
            .bind("2026-01-01T00:00:00Z")
            .bind("2026-01-01T00:00:00Z")
            .execute(h.db().postgres_pool().expect("postgres"))
            .await
            .expect("seed pseud");
        }
    }

    tm::create_topic(h.db(), &topic_id, "general", &pseud_id, "A thread", mode)
        .await
        .expect("create topic");

    // A post in the topic, because `topic_wiki_pins.post_id` has a foreign key
    // to `forum_posts` and a pin pointing at nothing is a constraint violation
    // rather than a row that quietly fails to read.
    let post_id = uuid::Uuid::new_v4().to_string();
    match h.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO forum_posts (id, topic_id, author_pseud, body, created_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&post_id)
            .bind(&topic_id)
            .bind(&pseud_id)
            .bind("The opening post.")
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
            .bind(&post_id)
            .bind(&topic_id)
            .bind(&pseud_id)
            .bind("The opening post.")
            .bind("2026-01-01T00:00:00Z")
            .execute(h.db().postgres_pool().expect("postgres"))
            .await
            .expect("seed post");
        }
    }

    (topic_id, post_id)
}

/// Read a topic's mode on whichever backend is in play.
async fn read_mode(h: &Db, id: &str) -> String {
    match h.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            sqlx::query_scalar("SELECT mode FROM forum_topics WHERE id = ?")
                .bind(id)
                .fetch_one(h.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("read mode")
        }
        lorehaven_db::Backend::Postgres => {
            sqlx::query_scalar("SELECT mode FROM forum_topics WHERE id = $1")
                .bind(id)
                .fetch_one(h.db().postgres_pool().expect("postgres"))
                .await
                .expect("read mode")
        }
    }
}

// ---------------------------------------------------------------------------
// Thread modes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_topic_keeps_the_mode_it_was_created_with() {
    let h = Db::new("mode").await;
    let (t, _post) = topic(&h, "critique").await;

    assert_eq!(read_mode(&h, t.as_str()).await, "critique");
}

#[tokio::test]
async fn the_mode_can_be_changed_after_the_fact() {
    let h = Db::new("set-mode").await;
    let (t, _post) = topic(&h, "plain").await;

    tm::set_topic_mode(h.db(), t.as_str(), "reading-group")
        .await
        .expect("set mode");

    assert_eq!(read_mode(&h, t.as_str()).await, "reading-group");
}

// ---------------------------------------------------------------------------
// Reading-group schedule (M33-02)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn schedule_sections_come_back_in_position_order_not_insertion_order() {
    let h = Db::new("schedule-order").await;
    let (t, _post) = topic(&h, "reading-group").await;

    // Inserted out of order on purpose: `ORDER BY position` is the contract, and
    // a test that inserts in order cannot tell an ordered read from an
    // insertion-ordered one.
    tm::add_schedule_section(
        h.db(),
        t.as_str(),
        2,
        "Third",
        20,
        30,
        "2026-02-01T00:00:00Z",
    )
    .await
    .expect("add third");
    tm::add_schedule_section(
        h.db(),
        t.as_str(),
        0,
        "First",
        1,
        10,
        "2026-01-01T00:00:00Z",
    )
    .await
    .expect("add first");
    tm::add_schedule_section(
        h.db(),
        t.as_str(),
        1,
        "Second",
        11,
        19,
        "2026-01-15T00:00:00Z",
    )
    .await
    .expect("add second");

    let schedule = tm::get_schedule(h.db(), t.as_str())
        .await
        .expect("get schedule");
    let titles: Vec<&str> = schedule
        .iter()
        .map(|s| s["title"].as_str().expect("title"))
        .collect();
    assert_eq!(titles, ["First", "Second", "Third"], "ordered by position");
}

#[tokio::test]
async fn a_schedules_chapter_range_and_unlock_time_survive_the_round_trip() {
    let h = Db::new("schedule-fields").await;
    let (t, _post) = topic(&h, "reading-group").await;

    tm::add_schedule_section(
        h.db(),
        t.as_str(),
        0,
        "Act One",
        3,
        17,
        "2026-03-04T05:06:07Z",
    )
    .await
    .expect("add section");

    let schedule = tm::get_schedule(h.db(), t.as_str()).await.expect("get");
    assert_eq!(schedule.len(), 1);
    let row = &schedule[0];
    assert_eq!(row["title"], "Act One");
    assert_eq!(row["position"], 0);
    assert_eq!(row["chapter_start"], 3);
    assert_eq!(row["chapter_end"], 17);
    // The unlock time is a string in both dialects; on PostgreSQL a TIMESTAMPTZ
    // column read into `String` is a decode error, so this assertion is the one
    // that would catch a cast being dropped.
    assert!(
        row["unlocks_at"]
            .as_str()
            .expect("unlocks")
            .contains("2026-03-04"),
        "unlock time came back as {:?}",
        row["unlocks_at"]
    );
}

#[tokio::test]
async fn a_schedule_is_scoped_to_its_own_topic() {
    let h = Db::new("schedule-scope").await;
    let (a, _post) = topic(&h, "reading-group").await;
    let (b, _post_b) = topic(&h, "reading-group").await;
    // Written against the ids `topic` returned. Querying a literal that was
    // never inserted would make the first assertion fail and the second pass
    // vacuously, which is how a scope test ends up testing nothing.
    tm::add_schedule_section(h.db(), &a, 0, "Only A", 1, 5, "2026-01-01T00:00:00Z")
        .await
        .expect("add to a");

    let on_a = tm::get_schedule(h.db(), &a).await.expect("get a");
    let on_b = tm::get_schedule(h.db(), &b).await.expect("get b");
    assert_eq!(on_a.len(), 1, "topic A has its section");
    assert!(on_b.is_empty(), "topic B must not borrow A's schedule");
}

// ---------------------------------------------------------------------------
// Critique circle (M33-03)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_first_critique_member_gets_position_zero() {
    let h = Db::new("queue-first").await;
    let (t, _post) = topic(&h, "critique").await;

    // `MAX(position)` over an empty table is NULL, so this is the line where a
    // bare `unwrap_or_default()` would also produce 0 -- and the *second* member
    // is where it would break. Assert both.
    let first = tm::join_critique(h.db(), t.as_str(), "pseud-topic-q")
        .await
        .expect("join first");
    assert_eq!(first, 0, "the queue starts at zero, not one");
}

#[tokio::test]
async fn critique_positions_increment_and_the_queue_comes_back_in_turn_order() {
    let h = Db::new("queue-order").await;
    let (t, _post) = topic(&h, "critique").await;

    // Real uuids: `critique_queue.pseud` is UUID on PostgreSQL and TEXT on
    // SQLite, so a readable name passes one backend and is rejected by the other.
    let first = uuid::Uuid::new_v4().to_string();
    let second = uuid::Uuid::new_v4().to_string();
    let third = uuid::Uuid::new_v4().to_string();

    let a = tm::join_critique(h.db(), t.as_str(), &first)
        .await
        .expect("join a");
    let b = tm::join_critique(h.db(), t.as_str(), &second)
        .await
        .expect("join b");
    let c = tm::join_critique(h.db(), t.as_str(), &third)
        .await
        .expect("join c");

    assert_eq!((a, b, c), (0, 1, 2), "each member takes the next turn");

    let queue = tm::get_critique_queue(h.db(), t.as_str())
        .await
        .expect("get queue");
    let pseuds: Vec<&str> = queue
        .iter()
        .map(|q| q["pseud"].as_str().expect("pseud"))
        .collect();
    assert_eq!(
        pseuds,
        [first.as_str(), second.as_str(), third.as_str()],
        "the queue is in turn order"
    );
}

#[tokio::test]
async fn a_critique_queue_is_scoped_to_its_own_topic() {
    let h = Db::new("queue-scope").await;
    let (a, _post) = topic(&h, "critique").await;
    let (b, _post_b) = topic(&h, "critique").await;

    tm::join_critique(h.db(), a.as_str(), &uuid::Uuid::new_v4().to_string())
        .await
        .expect("join q1");

    let q1 = tm::get_critique_queue(h.db(), a.as_str())
        .await
        .expect("get q1");
    let q2 = tm::get_critique_queue(h.db(), b.as_str())
        .await
        .expect("get q2");
    assert_eq!(q1.len(), 1);
    assert!(q2.is_empty(), "topic Q2 must not inherit Q1's queue");
}

#[tokio::test]
async fn an_empty_critique_queue_is_empty_not_an_error() {
    let h = Db::new("queue-empty").await;
    let (t, _post) = topic(&h, "critique").await;

    let queue = tm::get_critique_queue(h.db(), t.as_str())
        .await
        .expect("an empty queue reads fine");
    assert!(queue.is_empty());
}

// ---------------------------------------------------------------------------
// Wiki pin (M33-04)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_wiki_pin_is_invisible_until_it_is_approved() {
    let h = Db::new("pin-approval").await;
    let (t, post) = topic(&h, "plain").await;

    tm::create_wiki_pin(h.db(), t.as_str(), &post, "The agreed name", "editor")
        .await
        .expect("create pin");

    // The whole point of the approval queue: a proposed edit is not the wiki.
    let before = tm::get_wiki_pin(h.db(), t.as_str())
        .await
        .expect("get before approval");
    assert!(
        before.is_none(),
        "an unapproved pin must not be readable as the wiki, got {before:?}"
    );

    tm::approve_wiki_pin(h.db(), t.as_str(), &post, "moderator")
        .await
        .expect("approve pin");

    let after = tm::get_wiki_pin(h.db(), t.as_str())
        .await
        .expect("get after approval");
    let pin = after.expect("an approved pin is readable");
    assert_eq!(pin["body"], "The agreed name");
}

#[tokio::test]
async fn approving_a_pin_bumps_its_revision() {
    let h = Db::new("pin-revision").await;
    let (t, post) = topic(&h, "plain").await;

    tm::create_wiki_pin(h.db(), t.as_str(), &post, "First cut", "editor")
        .await
        .expect("create");

    // Revision 0 is the proposal; approval is revision 1. Asserting the bump is
    // what catches a `revision = revision` typo, which no read of `body` shows.
    tm::approve_wiki_pin(h.db(), t.as_str(), &post, "mod-one")
        .await
        .expect("approve once");
    let pin = tm::get_wiki_pin(h.db(), t.as_str())
        .await
        .expect("get")
        .expect("pin");
    assert_eq!(pin["revision"], 1, "approval is a revision");

    tm::approve_wiki_pin(h.db(), t.as_str(), &post, "mod-two")
        .await
        .expect("approve twice");
    let pin = tm::get_wiki_pin(h.db(), t.as_str())
        .await
        .expect("get")
        .expect("pin");
    assert_eq!(pin["revision"], 2, "each approval is a further revision");
}

#[tokio::test]
async fn a_wiki_pin_is_scoped_to_its_own_topic() {
    let h = Db::new("pin-scope").await;
    let (a, post) = topic(&h, "plain").await;
    let (b, _post_b) = topic(&h, "plain").await;

    tm::create_wiki_pin(h.db(), a.as_str(), &post, "Only P1", "editor")
        .await
        .expect("create");
    tm::approve_wiki_pin(h.db(), a.as_str(), &post, "mod")
        .await
        .expect("approve");

    let p1 = tm::get_wiki_pin(h.db(), a.as_str()).await.expect("get p1");
    let p2 = tm::get_wiki_pin(h.db(), b.as_str()).await.expect("get p2");
    assert!(p1.is_some());
    assert!(p2.is_none(), "topic P2 must not borrow P1's approved pin");
}

#[tokio::test]
async fn approving_a_pin_that_does_not_exist_is_not_an_error() {
    let h = Db::new("pin-missing").await;
    let (t, _post) = topic(&h, "plain").await;

    // An UPDATE matching zero rows. It must not claim success loudly or panic,
    // and it must not create a phantom approved row.
    tm::approve_wiki_pin(h.db(), t.as_str(), &uuid::Uuid::new_v4().to_string(), "mod")
        .await
        .expect("approving nothing is a no-op");
    let pin = tm::get_wiki_pin(h.db(), t.as_str()).await.expect("get");
    assert!(pin.is_none(), "no phantom pin was created");
}
