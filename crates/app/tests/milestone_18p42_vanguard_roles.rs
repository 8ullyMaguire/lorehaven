//! M18-P4.2 — Taste Vanguard roles and pins (`crates/db/src/roles.rs`,
//! spec §16.18).
//!
//! Nine public functions with no test touching them, against a schema that
//! types its timestamps `TIMESTAMPTZ` on PostgreSQL and `TEXT` on SQLite — the
//! combination that hides a decode failure until the PG run.
//!
//! **The contribution-volume selection query named columns that exist in
//! neither backend.** It read `bookmarks.pseud_id`, `bookmarks.work_id` and a
//! table called `reviews`; the real schema is
//! `bookmarks(account_id, subject_type, subject_id)` and the table is `review`.
//! So `select_by_contribution_volume` — and therefore `select_vanguards` —
//! failed on *both* backends and had no callers to notice. Fixed here against
//! `analytics::reader_totals`, which reads the same two tables correctly.
//!
//! **Pinning also exposed that a re-pin returns an id that is not stored.**
//! `pin_work` mints a fresh `id` per call and returns it, but its
//! `ON CONFLICT(account_id, work_id) DO UPDATE` leaves the original row's id
//! alone. So pinning a work twice returns a second id for a row that only ever
//! had the first, and the route hands that id to the client in its 201 body.
//! Pinned below as a known defect rather than fixed, because the fix is a
//! product decision (see docs/known-gaps.md M18-P42-D03).

use std::path::PathBuf;

use lorehaven_db::roles;
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m18p42-{tag}-{}-{:?}",
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

    async fn work(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let owner = uuid::Uuid::new_v4().to_string();
        let acct = self.account().await;
        self.exec(&format!(
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
             VALUES ('{owner}', '{acct}', 'w{owner}', 'W', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at) \
             VALUES ('{id}', '{owner}', 'A Work', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        id
    }

    /// A `bookmarks` row. The real shape is `(account_id, subject_type,
    /// subject_id)` — the columns the broken query got wrong.
    async fn bookmark(&self, account: &str, work: &str, at: &str) {
        self.exec(&format!(
            "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, created_at, updated_at) \
             VALUES ('{}', '{account}', 'work', '{work}', '{at}', '{at}')",
            uuid::Uuid::new_v4()
        ))
        .await;
    }

    /// A `review` row (singular). `review` also carries `pseud_id`, so seed one.
    async fn review(&self, account: &str, work: &str, at: &str) {
        let pseud = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
             VALUES ('{pseud}', '{account}', 'r{pseud}', 'R', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO review (id, account_id, pseud_id, work_id, body, created_at, updated_at) \
             VALUES ('{}', '{account}', '{pseud}', '{work}', 'good', '{at}', '{at}')",
            uuid::Uuid::new_v4()
        ))
        .await;
    }

    /// Read a single `vanguard_roles` / `vanguard_pins` cell as text.
    ///
    /// `query_scalar::<_, Option<String>>` cannot tell SQL NULL from the empty
    /// string -- both arrive as `Some("")` -- and `expires_at` being NULL is
    /// exactly the distinction several tests here turn on. Selecting it into
    /// `Option<Option<String>>` keeps the outer `None` for "no row" and the
    /// inner one for NULL.
    async fn cell(&self, query: &str) -> Option<String> {
        let q = self.pg_cast(query);
        let row: Option<(Option<String>,)> = match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_as(&q)
                .fetch_optional(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("cell"),
            lorehaven_db::Backend::Postgres => sqlx::query_as(&q)
                .fetch_optional(self.db().postgres_pool().expect("pg"))
                .await
                .expect("cell"),
        };
        row.and_then(|(v,)| v)
    }

    /// Whether a `vanguard_roles` / `vanguard_pins` column is SQL NULL for a
    /// row. Separate from `cell` because a NULL and an empty string must be
    /// told apart, and both must be told apart from "no such row".
    ///
    /// Casts on PostgreSQL: these tables type their ids `UUID` and their
    /// timestamps `TIMESTAMPTZ`, and sqlx will not decode either into a
    /// `String`. This is the same cast the module's own PG arms carry, and it
    /// is the reason these two tests fail on PG without it.
    async fn cell_is_null(&self, query: &str) -> bool {
        let q = self.pg_cast(query);
        let row: Option<(Option<String>,)> = match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_as(&q)
                .fetch_optional(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("cell"),
            lorehaven_db::Backend::Postgres => sqlx::query_as(&q)
                .fetch_optional(self.db().postgres_pool().expect("pg"))
                .await
                .expect("cell"),
        };
        matches!(row, Some((None,)))
    }

    /// Rewrite a single-column probe into a form both drivers can decode: on
    /// PostgreSQL the first selected column is cast to text.
    fn pg_cast(&self, query: &str) -> String {
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => self.tdb.sql(query),
            lorehaven_db::Backend::Postgres => {
                // `SELECT <col> …` -> `SELECT <col>::text …`
                let Some(rest) = query.strip_prefix("SELECT ") else {
                    return self.tdb.sql(query);
                };
                // The cast is applied HERE, to the part before FROM, which is
                // what makes the resulting statement read `SELECT col::text
                // FROM ...`. Building it this way rather than casting in a
                // caller is why the column needs no cast of its own -- and why
                // the literal below is a *template*, not a statement anyone can
                // run. A checker reading it as SQL sees `SELECT {}::text{}` and
                // reports a "SELECT with no FROM", which is a true observation
                // about a string that was never a query.
                match rest.find(" FROM ") {
                    Some(idx) => format!("SELECT {}::text{}", &rest[..idx], &rest[idx..]),
                    None => self.tdb.sql(query),
                }
            }
        }
    }

    async fn pin_id(&self, account: &str, work: &str) -> String {
        self.cell(&format!(
            "SELECT id FROM vanguard_pins WHERE account_id = '{account}' AND work_id = '{work}'"
        ))
        .await
        .expect("a pin row")
    }

    async fn pin_count(&self) -> i64 {
        let q = self.tdb.sql("SELECT COUNT(*) FROM vanguard_pins");
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

/// An RFC 3339 stamp the given number of days from now, which is the form
/// every timestamp column stores on both backends.
fn days_from_now(days: i64) -> String {
    lorehaven_db::identity::in_seconds(days * 86_400)
}

// ---------------------------------------------------------------------------
// grant_vanguard / revoke_vanguard / is_vanguard
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_account_is_not_a_vanguard_until_it_is_granted_one() {
    let h = Db::new("vg-absent").await;
    let account = h.account().await;
    assert!(!roles::is_vanguard(h.db(), &account).await.unwrap());
}

#[tokio::test]
async fn a_granted_account_is_a_vanguard() {
    let h = Db::new("vg-grant").await;
    let account = h.account().await;
    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();
    assert!(roles::is_vanguard(h.db(), &account).await.unwrap());
}

#[tokio::test]
async fn granting_twice_leaves_one_role_row() {
    let h = Db::new("vg-idempotent").await;
    let account = h.account().await;
    for _ in 0..2 {
        roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
            .await
            .unwrap();
    }
    assert!(roles::is_vanguard(h.db(), &account).await.unwrap());
    let q = h.tdb.sql("SELECT COUNT(*) FROM vanguard_roles");
    let n: i64 = match h.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&q)
            .fetch_one(h.db().sqlite_pool().expect("sqlite"))
            .await
            .unwrap(),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&q)
            .fetch_one(h.db().postgres_pool().expect("pg"))
            .await
            .unwrap(),
    };
    assert_eq!(n, 1, "the ON CONFLICT keeps one row per account");
}

#[tokio::test]
async fn re_granting_records_the_new_method() {
    // The upsert's DO UPDATE carries granted_at, method, expires_at and
    // granted_by, so a second grant is a promotion rather than a no-op.
    let h = Db::new("vg-promote").await;
    let account = h.account().await;
    roles::grant_vanguard(h.db(), &account, "resonance_threshold", None, None)
        .await
        .unwrap();
    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();

    let method: String = h
        .cell(&format!(
            "SELECT method FROM vanguard_roles WHERE account_id = '{account}'"
        ))
        .await
        .expect("method");
    assert_eq!(method, "admin_appointment", "the later grant wins");
}

#[tokio::test]
async fn a_role_with_no_expiry_never_expires() {
    let h = Db::new("vg-no-expiry").await;
    let account = h.account().await;
    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();

    assert!(
        h.cell_is_null(&format!(
            "SELECT expires_at FROM vanguard_roles WHERE account_id = '{account}'"
        ))
        .await,
        "NULL expires_at means it does not expire"
    );
    assert!(roles::is_vanguard(h.db(), &account).await.unwrap());
}

#[tokio::test]
async fn a_role_that_expires_in_the_past_is_not_a_vanguard() {
    let h = Db::new("vg-expired").await;
    let account = h.account().await;
    roles::grant_vanguard(
        h.db(),
        &account,
        "admin_appointment",
        None,
        Some(&days_from_now(-1)),
    )
    .await
    .unwrap();

    assert!(
        !roles::is_vanguard(h.db(), &account).await.unwrap(),
        "an elapsed expires_at takes effect without a sweeper"
    );
}

#[tokio::test]
async fn a_role_that_expires_in_the_future_still_holds() {
    let h = Db::new("vg-expiring").await;
    let account = h.account().await;
    roles::grant_vanguard(
        h.db(),
        &account,
        "admin_appointment",
        None,
        Some(&days_from_now(30)),
    )
    .await
    .unwrap();
    assert!(roles::is_vanguard(h.db(), &account).await.unwrap());
}

#[tokio::test]
async fn a_grant_records_who_made_it() {
    let h = Db::new("vg-granted-by").await;
    let account = h.account().await;
    let granter = h.account().await;
    roles::grant_vanguard(h.db(), &account, "admin_appointment", Some(&granter), None)
        .await
        .unwrap();

    // `granted_by` is UUID on PG, so `cell` reads it as text on both backends
    // and the comparison is the same either way.
    let stored = h
        .cell(&format!(
            "SELECT granted_by FROM vanguard_roles WHERE account_id = '{account}'"
        ))
        .await;
    assert_eq!(stored.as_deref(), Some(granter.as_str()));
}

#[tokio::test]
async fn a_revoked_account_is_no_longer_a_vanguard() {
    let h = Db::new("vg-revoke").await;
    let account = h.account().await;
    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();

    roles::revoke_vanguard(h.db(), &account).await.unwrap();
    assert!(!roles::is_vanguard(h.db(), &account).await.unwrap());
}

#[tokio::test]
async fn revoking_an_account_that_holds_no_role_is_harmless() {
    let h = Db::new("vg-revoke-absent").await;
    let account = h.account().await;
    roles::revoke_vanguard(h.db(), &account).await.unwrap();
    assert!(!roles::is_vanguard(h.db(), &account).await.unwrap());
}

#[tokio::test]
async fn revoking_one_account_leaves_another_holding_the_role() {
    let h = Db::new("vg-revoke-one").await;
    let a = h.account().await;
    let b = h.account().await;
    for acc in [&a, &b] {
        roles::grant_vanguard(h.db(), acc, "admin_appointment", None, None)
            .await
            .unwrap();
    }
    roles::revoke_vanguard(h.db(), &a).await.unwrap();

    assert!(!roles::is_vanguard(h.db(), &a).await.unwrap());
    assert!(roles::is_vanguard(h.db(), &b).await.unwrap());
}

#[tokio::test]
async fn a_revoked_role_can_be_granted_again() {
    let h = Db::new("vg-regrant").await;
    let account = h.account().await;
    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();
    roles::revoke_vanguard(h.db(), &account).await.unwrap();
    roles::grant_vanguard(h.db(), &account, "contribution_volume", None, None)
        .await
        .unwrap();

    assert!(roles::is_vanguard(h.db(), &account).await.unwrap());
    let method: String = h
        .cell(&format!(
            "SELECT method FROM vanguard_roles WHERE account_id = '{account}'"
        ))
        .await
        .expect("method");
    assert_eq!(method, "contribution_volume");
}

// ---------------------------------------------------------------------------
// list_vanguards
// ---------------------------------------------------------------------------

#[tokio::test]
async fn there_are_no_vanguards_to_list_initially() {
    let h = Db::new("vg-list-empty").await;
    assert!(roles::list_vanguards(h.db()).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_granted_account_appears_in_the_list() {
    let h = Db::new("vg-list").await;
    let account = h.account().await;
    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();

    let got = roles::list_vanguards(h.db()).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["account_id"].as_str(), Some(account.as_str()));
    assert_eq!(got[0]["method"].as_str(), Some("admin_appointment"));
    assert_eq!(got[0]["expires_at"], serde_json::Value::Null);
    assert!(
        got[0]["granted_at"].as_str().is_some(),
        "granted_at is a string on both backends, so the JSON shape does not differ"
    );
}

#[tokio::test]
async fn an_expired_role_is_not_listed() {
    let h = Db::new("vg-list-expired").await;
    let expired = h.account().await;
    let current = h.account().await;
    roles::grant_vanguard(
        h.db(),
        &expired,
        "admin_appointment",
        None,
        Some(&days_from_now(-1)),
    )
    .await
    .unwrap();
    roles::grant_vanguard(h.db(), &current, "admin_appointment", None, None)
        .await
        .unwrap();

    let got = roles::list_vanguards(h.db()).await.unwrap();
    assert_eq!(got.len(), 1, "only the live role is listed");
    assert_eq!(got[0]["account_id"].as_str(), Some(current.as_str()));
}

#[tokio::test]
async fn a_revoked_account_leaves_the_list() {
    let h = Db::new("vg-list-revoked").await;
    let account = h.account().await;
    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();
    roles::revoke_vanguard(h.db(), &account).await.unwrap();

    assert!(roles::list_vanguards(h.db()).await.unwrap().is_empty());
}

#[tokio::test]
async fn the_list_covers_every_live_role() {
    let h = Db::new("vg-list-many").await;
    let mut accounts = Vec::new();
    for _ in 0..3 {
        let a = h.account().await;
        roles::grant_vanguard(h.db(), &a, "admin_appointment", None, None)
            .await
            .unwrap();
        accounts.push(a);
    }
    let got = roles::list_vanguards(h.db()).await.unwrap();
    assert_eq!(got.len(), 3);
    for a in &accounts {
        assert!(
            got.iter()
                .any(|v| v["account_id"].as_str() == Some(a.as_str())),
            "every granted account is present"
        );
    }
}

// ---------------------------------------------------------------------------
// pin_work / unpin_work / list_active_pins
// ---------------------------------------------------------------------------

#[tokio::test]
async fn only_a_vanguards_pins_show_up_on_the_shelf() {
    // `list_active_pins` joins `vanguard_roles`, so a pin from a non-vanguard
    // is stored but not displayed. That is the shape the route relies on.
    let h = Db::new("pin-gated").await;
    let account = h.account().await;
    let work = h.work().await;
    roles::pin_work(h.db(), &account, &work, "curated", None)
        .await
        .unwrap();

    assert_eq!(h.pin_count().await, 1, "the row is stored regardless");
    assert!(
        roles::list_active_pins(h.db()).await.unwrap().is_empty(),
        "but a non-vanguard's pin is not on the shelf"
    );

    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();
    assert_eq!(roles::list_active_pins(h.db()).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_pin_carries_its_reason_and_message() {
    let h = Db::new("pin-fields").await;
    let account = h.account().await;
    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();
    let work = h.work().await;
    roles::pin_work(h.db(), &account, &work, "underrated", Some("read this"))
        .await
        .unwrap();

    let got = roles::list_active_pins(h.db()).await.unwrap();
    assert_eq!(got[0]["pin_reason"].as_str(), Some("underrated"));
    assert_eq!(got[0]["message"].as_str(), Some("read this"));
    assert_eq!(got[0]["work_id"].as_str(), Some(work.as_str()));
    assert_eq!(got[0]["account_id"].as_str(), Some(account.as_str()));
    assert!(got[0]["pinned_at"].as_str().is_some());
}

#[tokio::test]
async fn a_pin_with_no_message_has_none() {
    let h = Db::new("pin-nomsg").await;
    let account = h.account().await;
    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();
    let work = h.work().await;
    roles::pin_work(h.db(), &account, &work, "curated", None)
        .await
        .unwrap();

    assert_eq!(
        roles::list_active_pins(h.db()).await.unwrap()[0]["message"],
        serde_json::Value::Null
    );
}

#[tokio::test]
async fn pinning_the_same_work_twice_keeps_one_row() {
    let h = Db::new("pin-upsert").await;
    let account = h.account().await;
    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();
    let work = h.work().await;
    roles::pin_work(h.db(), &account, &work, "first", None)
        .await
        .unwrap();
    roles::pin_work(h.db(), &account, &work, "second", None)
        .await
        .unwrap();

    assert_eq!(
        h.pin_count().await,
        1,
        "the unique index is on (account, work)"
    );
    let got = roles::list_active_pins(h.db()).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(
        got[0]["pin_reason"].as_str(),
        Some("second"),
        "the later reason replaces the first"
    );
}

#[tokio::test]
async fn re_pinning_returns_an_id_that_was_never_stored() {
    // KNOWN DEFECT. `pin_work` mints a fresh id every call and returns it, but
    // its ON CONFLICT clause updates the existing row without touching `id`.
    // So the second call reports an id that belongs to no row -- and the route
    // puts it in the 201 body. See docs/known-gaps.md M18-P42-D03.
    //
    // The emphatic spelling of the defect is in the assertion message, not the
    // test name: a SCREAMING_CASE fn trips clippy's non_snake_case, and a
    // #[allow] on it would be the only clippy suppression in the suite.
    let h = Db::new("pin-id-defect").await;
    let account = h.account().await;
    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();
    let work = h.work().await;

    let first = roles::pin_work(h.db(), &account, &work, "first", None)
        .await
        .unwrap();
    let second = roles::pin_work(h.db(), &account, &work, "second", None)
        .await
        .unwrap();

    assert_eq!(
        first,
        h.pin_id(&account, &work).await,
        "the first id is real"
    );
    assert_ne!(first, second, "each call mints a fresh id");
    assert_ne!(
        second,
        h.pin_id(&account, &work).await,
        "KNOWN DEFECT: the id returned by a re-pin is not the stored one"
    );
}

#[tokio::test]
async fn two_vanguards_can_pin_the_same_work() {
    let h = Db::new("pin-two-accounts").await;
    let a = h.account().await;
    let b = h.account().await;
    for acc in [&a, &b] {
        roles::grant_vanguard(h.db(), acc, "admin_appointment", None, None)
            .await
            .unwrap();
    }
    let work = h.work().await;
    roles::pin_work(h.db(), &a, &work, "curated", None)
        .await
        .unwrap();
    roles::pin_work(h.db(), &b, &work, "curated", None)
        .await
        .unwrap();

    assert_eq!(
        h.pin_count().await,
        2,
        "the unique key includes the account"
    );
    assert_eq!(roles::list_active_pins(h.db()).await.unwrap().len(), 2);
}

#[tokio::test]
async fn unpinning_takes_a_work_off_the_shelf() {
    let h = Db::new("pin-unpin").await;
    let account = h.account().await;
    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();
    let work = h.work().await;
    roles::pin_work(h.db(), &account, &work, "curated", None)
        .await
        .unwrap();

    roles::unpin_work(h.db(), &account, &work).await.unwrap();
    assert!(roles::list_active_pins(h.db()).await.unwrap().is_empty());
}

#[tokio::test]
async fn unpinning_keeps_the_row_for_audit() {
    let h = Db::new("pin-audit").await;
    let account = h.account().await;
    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();
    let work = h.work().await;
    roles::pin_work(h.db(), &account, &work, "curated", None)
        .await
        .unwrap();

    roles::unpin_work(h.db(), &account, &work).await.unwrap();
    assert_eq!(h.pin_count().await, 1, "soft delete, not a row removal");

    assert!(
        !h.cell_is_null(&format!(
            "SELECT deleted_at FROM vanguard_pins WHERE account_id = '{account}' AND work_id = '{work}'"
        ))
        .await,
        "deleted_at is stamped rather than left null"
    );
}

#[tokio::test]
async fn unpinning_twice_leaves_it_off_the_shelf() {
    let h = Db::new("pin-unpin-twice").await;
    let account = h.account().await;
    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();
    let work = h.work().await;
    roles::pin_work(h.db(), &account, &work, "curated", None)
        .await
        .unwrap();

    roles::unpin_work(h.db(), &account, &work).await.unwrap();
    roles::unpin_work(h.db(), &account, &work).await.unwrap();
    assert!(roles::list_active_pins(h.db()).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_revoked_vanguards_pins_leave_the_shelf() {
    // The join is on the *current* role, so revoking removes the picks without
    // touching the pin rows themselves.
    let h = Db::new("pin-revoked-role").await;
    let account = h.account().await;
    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();
    let work = h.work().await;
    roles::pin_work(h.db(), &account, &work, "curated", None)
        .await
        .unwrap();
    assert_eq!(roles::list_active_pins(h.db()).await.unwrap().len(), 1);

    roles::revoke_vanguard(h.db(), &account).await.unwrap();
    assert!(roles::list_active_pins(h.db()).await.unwrap().is_empty());
    assert_eq!(h.pin_count().await, 1, "the pin row itself survives");
}

#[tokio::test]
async fn unpinning_one_vanguards_pin_leaves_anothers_on_the_shelf() {
    let h = Db::new("pin-unpin-one").await;
    let a = h.account().await;
    let b = h.account().await;
    for acc in [&a, &b] {
        roles::grant_vanguard(h.db(), acc, "admin_appointment", None, None)
            .await
            .unwrap();
    }
    let work = h.work().await;
    roles::pin_work(h.db(), &a, &work, "curated", None)
        .await
        .unwrap();
    roles::pin_work(h.db(), &b, &work, "curated", None)
        .await
        .unwrap();

    roles::unpin_work(h.db(), &a, &work).await.unwrap();
    let got = roles::list_active_pins(h.db()).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["account_id"].as_str(), Some(b.as_str()));
}

#[tokio::test]
async fn a_revoked_role_stops_new_pins_being_shown_without_unpinping_them() {
    let h = Db::new("pin-role-back").await;
    let account = h.account().await;
    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();
    let work = h.work().await;
    roles::pin_work(h.db(), &account, &work, "curated", None)
        .await
        .unwrap();
    roles::revoke_vanguard(h.db(), &account).await.unwrap();
    assert!(roles::list_active_pins(h.db()).await.unwrap().is_empty());

    // Re-granting puts the same pin back on the shelf: the pin was never
    // deleted, only hidden by the missing role.
    roles::grant_vanguard(h.db(), &account, "admin_appointment", None, None)
        .await
        .unwrap();
    let got = roles::list_active_pins(h.db()).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["account_id"].as_str(), Some(account.as_str()));
}

// ---------------------------------------------------------------------------
// select_by_contribution_volume / select_vanguards
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nobody_has_contributed_yet() {
    let h = Db::new("sel-empty").await;
    assert!(roles::select_by_contribution_volume(h.db(), 10)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn an_account_with_no_bookmarks_or_reviews_is_not_selected() {
    let h = Db::new("sel-none").await;
    h.account().await;
    assert!(roles::select_by_contribution_volume(h.db(), 10)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_bookmark_alone_qualifies_an_account() {
    // The query the old code could not run at all: it asked for
    // `bookmarks.pseud_id` and `bookmarks.work_id`, and neither column exists.
    let h = Db::new("sel-bookmark").await;
    let account = h.account().await;
    h.bookmark(&account, &h.work().await, &days_from_now(-1))
        .await;

    assert_eq!(
        roles::select_by_contribution_volume(h.db(), 10)
            .await
            .unwrap(),
        vec![account]
    );
}

#[tokio::test]
async fn a_review_alone_qualifies_an_account() {
    // The other half of the same fix: the table is `review`, singular.
    let h = Db::new("sel-review").await;
    let account = h.account().await;
    h.review(&account, &h.work().await, &days_from_now(-1))
        .await;

    assert_eq!(
        roles::select_by_contribution_volume(h.db(), 10)
            .await
            .unwrap(),
        vec![account]
    );
}

#[tokio::test]
async fn bookmarks_and_reviews_are_counted_together() {
    let h = Db::new("sel-both").await;
    let account = h.account().await;
    h.bookmark(&account, &h.work().await, &days_from_now(-1))
        .await;
    h.review(&account, &h.work().await, &days_from_now(-1))
        .await;

    assert_eq!(
        roles::select_by_contribution_volume(h.db(), 10)
            .await
            .unwrap(),
        vec![account],
        "the UNION ALL sums both sources into one count"
    );
}

#[tokio::test]
async fn the_busiest_curator_comes_first() {
    let h = Db::new("sel-order").await;
    let busy = h.account().await;
    let quiet = h.account().await;
    h.bookmark(&busy, &h.work().await, &days_from_now(-1)).await;
    h.bookmark(&busy, &h.work().await, &days_from_now(-2)).await;
    h.bookmark(&busy, &h.work().await, &days_from_now(-3)).await;
    h.bookmark(&quiet, &h.work().await, &days_from_now(-1))
        .await;

    assert_eq!(
        roles::select_by_contribution_volume(h.db(), 10)
            .await
            .unwrap(),
        vec![busy, quiet]
    );
}

#[tokio::test]
async fn the_limit_caps_how_many_are_selected() {
    let h = Db::new("sel-limit").await;
    for _ in 0..4 {
        let a = h.account().await;
        h.bookmark(&a, &h.work().await, &days_from_now(-1)).await;
    }
    assert_eq!(
        roles::select_by_contribution_volume(h.db(), 2)
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        roles::select_by_contribution_volume(h.db(), 0)
            .await
            .unwrap()
            .len(),
        0,
        "a zero limit selects nobody rather than everybody"
    );
}

#[tokio::test]
async fn contribution_older_than_the_window_is_not_counted() {
    // The 90-day window is applied by the statement (`created_at > ?`), so an
    // old contribution must not qualify an account at all.
    let h = Db::new("sel-window").await;
    let account = h.account().await;
    h.bookmark(&account, &h.work().await, &days_from_now(-120))
        .await;

    assert!(
        roles::select_by_contribution_volume(h.db(), 10)
            .await
            .unwrap()
            .is_empty(),
        "a contribution from 120 days ago is outside the 90-day window"
    );
}

#[tokio::test]
async fn a_bookmark_on_something_other_than_a_work_does_not_count() {
    // The fixed query filters `subject_type = 'work'`; without it a chapter
    // bookmark would read as a work contribution.
    let h = Db::new("sel-subject").await;
    let account = h.account().await;
    let work = h.work().await;
    h.exec(&format!(
        "INSERT INTO bookmarks (id, account_id, subject_type, subject_id, created_at, updated_at) \
         VALUES ('{}', '{account}', 'series', '{work}', '{}', '{}')",
        uuid::Uuid::new_v4(),
        days_from_now(-1),
        days_from_now(-1)
    ))
    .await;

    assert!(roles::select_by_contribution_volume(h.db(), 10)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn one_account_appears_once_however_much_it_contributed() {
    // GROUP BY account_id, so volume orders accounts rather than repeating one.
    let h = Db::new("sel-group").await;
    let account = h.account().await;
    for d in [-1, -2, -3, -4] {
        h.bookmark(&account, &h.work().await, &days_from_now(d))
            .await;
    }
    assert_eq!(
        roles::select_by_contribution_volume(h.db(), 10)
            .await
            .unwrap(),
        vec![account]
    );
}

#[tokio::test]
async fn the_contribution_volume_method_runs_the_query() {
    let h = Db::new("sel-dispatch").await;
    let account = h.account().await;
    h.bookmark(&account, &h.work().await, &days_from_now(-1))
        .await;

    assert_eq!(
        roles::select_vanguards(h.db(), "contribution_volume", 10)
            .await
            .unwrap(),
        vec![account],
        "`select_vanguards` dispatches to the contribution query"
    );
}

#[tokio::test]
async fn an_externally_handled_method_selects_nobody() {
    // resonance_threshold and admin_appointment are handled outside this
    // module, so they select no one here rather than erroring.
    let h = Db::new("sel-other").await;
    let account = h.account().await;
    h.bookmark(&account, &h.work().await, &days_from_now(-1))
        .await;

    for method in ["resonance_threshold", "admin_appointment", "nonsense"] {
        assert!(
            roles::select_vanguards(h.db(), method, 10)
                .await
                .unwrap()
                .is_empty(),
            "{method} is not this module's job"
        );
    }
}
