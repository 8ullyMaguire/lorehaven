//! M43.4 — Browse sort preferences (`crates/db/src/browse.rs`).
//!
//! Three `pub async fn` with no tests, behind sixteen references in the route
//! layer: every browse surface remembers the sort a reader last chose, so the
//! second visit reopens in the same order.
//!
//! The table is small and the semantics are narrow, so the suite concentrates on
//! the three things that can actually be wrong:
//!
//! - **the upsert key** is `(pseud_id, surface)`, so a reader has one preference
//!   per surface, and changing the sort must not create a second row;
//! - **`pseud_id` is a foreign key with `ON DELETE CASCADE`**, so deleting a
//!   pseud takes its preferences with it and orphans nothing;
//! - **`updated_at` is `TIMESTAMPTZ` on PostgreSQL but `TEXT` on SQLite**, and
//!   `get_sort_preference` has to return it as a `String` from both. The
//!   PostgreSQL arm carries an `updated_at::text` cast for exactly this reason,
//!   and the round-trip test is what keeps it honest.
//!
//! There is deliberately no test for "the sort value is one of the §43.2
//! vocabulary": this layer stores the string it is given and does not validate
//! it, so such a test would be asserting a rule that lives in the route.

use std::path::PathBuf;

use lorehaven_db::browse::{delete_sort_preference, get_sort_preference, set_sort_preference};
use lorehaven_db::identity::{create_account, create_pseud, AccountStatus};
use lorehaven_domain::ids::AccountId;
use lorehaven_domain::policy::AgeState;
use test_support::TestDb;
use uuid::Uuid;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-browse-{tag}-{}-{:?}",
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

    /// A pseud to hold preferences. `(account, pseud_id)`.
    async fn reader(&self) -> (AccountId, String) {
        let account = create_account(
            self.db(),
            &format!("browse-{}@example.test", Uuid::new_v4()),
            AgeState::DeclaredAdult,
            AccountStatus::Active,
        )
        .await
        .expect("create_account");
        let pseud = create_pseud(
            self.db(),
            account,
            &format!("b-{}", Uuid::new_v4()),
            "Browser",
        )
        .await
        .expect("create_pseud");
        (account, pseud.to_string())
    }
}

// ---------------------------------------------------------------------------
// set / get
// ---------------------------------------------------------------------------

/// A preference set is read back exactly as it was written — the round trip that
/// covers the `updated_at::text` cast on PostgreSQL.
#[tokio::test]
async fn a_preference_round_trips() {
    let h = Harness::new("browse-round-trip").await;
    let (_, pseud) = h.reader().await;

    set_sort_preference(h.db(), &pseud, "discover", "recent")
        .await
        .expect("set");

    let got = get_sort_preference(h.db(), &pseud, "discover")
        .await
        .expect("get")
        .expect("a preference was written");
    assert_eq!(got.pseud_id, pseud);
    assert_eq!(got.surface, "discover");
    assert_eq!(got.sort_value, "recent");
    assert!(
        !got.updated_at.is_empty(),
        "updated_at is non-empty text on both backends"
    );
}

/// A reader with no preference for a surface gets `None`, not a default sort —
/// the route decides the default, not the store.
#[tokio::test]
async fn an_unset_preference_is_none() {
    let h = Harness::new("browse-unset").await;
    let (_, pseud) = h.reader().await;
    assert!(
        get_sort_preference(h.db(), &pseud, "discover")
            .await
            .expect("get")
            .is_none(),
        "nothing stored, so nothing read back"
    );
}

/// **The upsert replaces rather than appends.** The key is `(pseud_id, surface)`,
/// so choosing a second sort for the same surface must leave exactly one row.
#[tokio::test]
async fn choosing_again_replaces_the_preference() {
    let h = Harness::new("browse-upsert").await;
    let (_, pseud) = h.reader().await;

    set_sort_preference(h.db(), &pseud, "discover", "recent")
        .await
        .expect("first");
    set_sort_preference(h.db(), &pseud, "discover", "top-rated")
        .await
        .expect("second");

    assert_eq!(
        h.count("reader_sort_preferences").await,
        1,
        "one row per (pseud, surface)"
    );
    assert_eq!(
        get_sort_preference(h.db(), &pseud, "discover")
            .await
            .expect("get")
            .expect("present")
            .sort_value,
        "top-rated",
        "and it holds the new value"
    );
}

/// Setting the same preference twice is idempotent.
#[tokio::test]
async fn setting_the_same_value_twice_is_idempotent() {
    let h = Harness::new("browse-idem").await;
    let (_, pseud) = h.reader().await;

    set_sort_preference(h.db(), &pseud, "discover", "recent")
        .await
        .expect("first");
    set_sort_preference(h.db(), &pseud, "discover", "recent")
        .await
        .expect("second");

    assert_eq!(h.count("reader_sort_preferences").await, 1);
}

/// **Preferences are per surface** — a reader who sorted `discover` one way and
/// `people` another gets both back independently.
#[tokio::test]
async fn preferences_are_per_surface() {
    let h = Harness::new("browse-surfaces").await;
    let (_, pseud) = h.reader().await;

    set_sort_preference(h.db(), &pseud, "discover", "recent")
        .await
        .expect("discover");
    set_sort_preference(h.db(), &pseud, "people", "name")
        .await
        .expect("people");

    assert_eq!(h.count("reader_sort_preferences").await, 2);
    assert_eq!(
        get_sort_preference(h.db(), &pseud, "discover")
            .await
            .expect("get")
            .map(|p| p.sort_value),
        Some("recent".to_string())
    );
    assert_eq!(
        get_sort_preference(h.db(), &pseud, "people")
            .await
            .expect("get")
            .map(|p| p.sort_value),
        Some("name".to_string())
    );
}

/// **Preferences are per reader** — one reader's choice never leaks into another's.
#[tokio::test]
async fn preferences_are_per_reader() {
    let h = Harness::new("browse-per-reader").await;
    let (_, alice) = h.reader().await;
    let (_, bob) = h.reader().await;

    set_sort_preference(h.db(), &alice, "discover", "recent")
        .await
        .expect("alice");
    set_sort_preference(h.db(), &bob, "discover", "name")
        .await
        .expect("bob");

    assert_eq!(h.count("reader_sort_preferences").await, 2);
    assert_eq!(
        get_sort_preference(h.db(), &alice, "discover")
            .await
            .expect("get")
            .expect("present")
            .sort_value,
        "recent"
    );
    assert_eq!(
        get_sort_preference(h.db(), &bob, "discover")
            .await
            .expect("get")
            .expect("present")
            .sort_value,
        "name"
    );
}

/// **The upsert restamps `updated_at` from the clock** rather than preserving the
/// row's old value, so a changed preference is distinguishable from an untouched
/// one.
///
/// The row is first forced to a fixed old stamp, so the test does not depend on
/// the clock advancing: if the upsert carried `updated_at` forward, the stored
/// value would still be `2000-01-01...` and the assertion would fail. (SQLite's
/// `datetime('now')` only has one-second resolution, so comparing two reads taken
/// in the same second would prove nothing.)
#[tokio::test]
async fn resetting_advances_updated_at() {
    let h = Harness::new("browse-updated").await;
    let (_, pseud) = h.reader().await;

    set_sort_preference(h.db(), &pseud, "discover", "recent")
        .await
        .expect("first");
    h.exec(&format!(
        "UPDATE reader_sort_preferences SET updated_at = '2000-01-01T00:00:00Z' \
         WHERE pseud_id = '{pseud}'"
    ))
    .await;

    set_sort_preference(h.db(), &pseud, "discover", "name")
        .await
        .expect("second");
    let stamp = get_sort_preference(h.db(), &pseud, "discover")
        .await
        .expect("get")
        .expect("present")
        .updated_at;

    assert_ne!(
        stamp, "2000-01-01T00:00:00Z",
        "the upsert restamps from the clock, not from the row"
    );
}

/// A pseud that does not exist is refused by the foreign key.
#[tokio::test]
async fn a_preference_for_an_unknown_pseud_is_refused() {
    let h = Harness::new("browse-fk").await;
    assert!(
        set_sort_preference(h.db(), &Uuid::new_v4().to_string(), "discover", "recent")
            .await
            .is_err()
    );
    assert_eq!(h.count("reader_sort_preferences").await, 0);
}

/// An arbitrary sort string is stored as given — this layer does not validate
/// the §43.2 vocabulary, and the round trip must not mangle it.
#[tokio::test]
async fn the_sort_value_is_stored_verbatim() {
    let h = Harness::new("browse-verbatim").await;
    let (_, pseud) = h.reader().await;
    for value in [
        "",
        "  spaced  ",
        "MiXeD-Case",
        "a'b\"c",
        "unicode-åß-日本語",
    ] {
        set_sort_preference(h.db(), &pseud, "discover", value)
            .await
            .expect("set");
        assert_eq!(
            get_sort_preference(h.db(), &pseud, "discover")
                .await
                .expect("get")
                .expect("present")
                .sort_value,
            value,
            "round trips {value:?} unchanged"
        );
    }
}

/// An arbitrary surface name is likewise stored verbatim.
#[tokio::test]
async fn the_surface_name_is_stored_verbatim() {
    let h = Harness::new("browse-surface-verbatim").await;
    let (_, pseud) = h.reader().await;
    set_sort_preference(h.db(), &pseud, "MiXeD Surface", "recent")
        .await
        .expect("set");
    assert_eq!(
        get_sort_preference(h.db(), &pseud, "MiXeD Surface")
            .await
            .expect("get")
            .expect("present")
            .surface,
        "MiXeD Surface"
    );
}

// ---------------------------------------------------------------------------
// delete
// ---------------------------------------------------------------------------

/// A deleted preference reads as `None` afterwards, and the row is gone.
#[tokio::test]
async fn a_deleted_preference_is_gone() {
    let h = Harness::new("browse-delete").await;
    let (_, pseud) = h.reader().await;
    set_sort_preference(h.db(), &pseud, "discover", "recent")
        .await
        .expect("set");

    delete_sort_preference(h.db(), &pseud, "discover")
        .await
        .expect("delete");

    assert_eq!(h.count("reader_sort_preferences").await, 0);
    assert!(
        get_sort_preference(h.db(), &pseud, "discover")
            .await
            .expect("get")
            .is_none(),
        "nothing stored, so nothing read back"
    );
}

/// **Deleting one surface leaves the others alone.**
#[tokio::test]
async fn deleting_one_surface_leaves_the_others() {
    let h = Harness::new("browse-delete-one").await;
    let (_, pseud) = h.reader().await;
    set_sort_preference(h.db(), &pseud, "discover", "recent")
        .await
        .expect("discover");
    set_sort_preference(h.db(), &pseud, "people", "name")
        .await
        .expect("people");

    delete_sort_preference(h.db(), &pseud, "discover")
        .await
        .expect("delete");

    assert_eq!(h.count("reader_sort_preferences").await, 1);
    assert!(
        get_sort_preference(h.db(), &pseud, "discover")
            .await
            .expect("get")
            .is_none(),
        "nothing stored, so nothing read back"
    );
    assert!(
        get_sort_preference(h.db(), &pseud, "people")
            .await
            .expect("get")
            .is_some(),
        "the other surface survives"
    );
}

/// Deleting something that is not there is not an error — the route treats
/// "forget my sort" as a normal action, and re-submitting the form is common.
#[tokio::test]
async fn deleting_an_absent_preference_is_not_an_error() {
    let h = Harness::new("browse-delete-absent").await;
    let (_, pseud) = h.reader().await;
    delete_sort_preference(h.db(), &pseud, "discover")
        .await
        .expect("deleting nothing succeeds");
    assert_eq!(h.count("reader_sort_preferences").await, 0);
}

/// A deleted preference can be set again — the upsert does not assume the row
/// still exists.
#[tokio::test]
async fn a_preference_can_be_restored_after_deleting() {
    let h = Harness::new("browse-restore").await;
    let (_, pseud) = h.reader().await;
    set_sort_preference(h.db(), &pseud, "discover", "recent")
        .await
        .expect("set");
    delete_sort_preference(h.db(), &pseud, "discover")
        .await
        .expect("delete");
    set_sort_preference(h.db(), &pseud, "discover", "name")
        .await
        .expect("set again");

    assert_eq!(h.count("reader_sort_preferences").await, 1);
    assert_eq!(
        get_sort_preference(h.db(), &pseud, "discover")
            .await
            .expect("get")
            .expect("present")
            .sort_value,
        "name"
    );
}

// ---------------------------------------------------------------------------
// Cascade
// ---------------------------------------------------------------------------

/// **Deleting a pseud takes its preferences with it.** The foreign key is
/// `ON DELETE CASCADE` on both backends, so a removed reader leaves no orphaned
/// rows behind.
#[tokio::test]
async fn deleting_a_pseud_cascades_its_preferences() {
    let h = Harness::new("browse-cascade").await;
    let (_account, pseud) = h.reader().await;
    set_sort_preference(h.db(), &pseud, "discover", "recent")
        .await
        .expect("discover");
    set_sort_preference(h.db(), &pseud, "people", "name")
        .await
        .expect("people");
    assert_eq!(h.count("reader_sort_preferences").await, 2);

    h.exec(&format!("DELETE FROM pseuds WHERE id = '{pseud}'"))
        .await;

    assert_eq!(
        h.count("reader_sort_preferences").await,
        0,
        "no orphans after the pseud is gone"
    );
    // The account outlives the pseud -- a reader may re-pseud -- so only the
    // preferences cascade, not the identity behind them.
    assert_eq!(h.count("accounts").await, 1, "the account is untouched");
}

/// Several readers' preferences coexist and cascade independently.
#[tokio::test]
async fn deleting_one_reader_leaves_the_others() {
    let h = Harness::new("browse-cascade-one").await;
    let (_, keep) = h.reader().await;
    let (_, drop) = h.reader().await;
    set_sort_preference(h.db(), &keep, "discover", "recent")
        .await
        .expect("keep");
    set_sort_preference(h.db(), &drop, "discover", "recent")
        .await
        .expect("drop");

    h.exec(&format!("DELETE FROM pseuds WHERE id = '{drop}'"))
        .await;

    assert_eq!(h.count("reader_sort_preferences").await, 1);
    assert!(
        get_sort_preference(h.db(), &keep, "discover")
            .await
            .expect("get")
            .is_some(),
        "the surviving reader keeps their sort"
    );
}
