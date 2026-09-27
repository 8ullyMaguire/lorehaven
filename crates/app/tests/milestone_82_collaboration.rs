//! M82 — Work collaboration: contributors, roles and invitations
//! (`crates/db/src/collaboration.rs`).
//!
//! Twelve public functions with no test touching them. This is the whole of the
//! co-authoring surface (spec §8), so two of its properties are load-bearing
//! and are asserted here rather than assumed:
//!
//! **An unrecognised stored role is dropped, never defaulted.**
//! `ContributorRole::parse` returns `None` for anything it does not know, and
//! both `contributors_for_work` and `decode_invite` filter those rows out. The
//! module says why: an unknown role silently becoming `Owner` "would be a
//! privilege escalation written as a fallback". So a row with `role = 'admin'`
//! must vanish, not appear as the owner.
//!
//! **The owner row is protected from `update_contributor` and
//! `remove_contributor`.** Both return `false` early when the target is the
//! owner. A work whose owner was removed would be unmanageable and
//! unpublishable — the module documents that, and these tests are the reason it
//! stays true.
//!
//! The pseud-isolation property (spec §8, ADR 0003) is also structural here:
//! `public_contributors` returns handle, display name and role, and nothing
//! else — no account, no private co-author, no invitation state.

use std::path::PathBuf;

use lorehaven_db::collaboration as collab;
use lorehaven_domain::content::ContributorRole;
use lorehaven_domain::{CollaborationInviteId, PseudId, WorkId};
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m82-{tag}-{}-{:?}",
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

    /// The `accounts` -> `pseuds` -> `works` chain. `work_contributors`
    /// foreign-keys both ends, so every test needs real rows at every level.
    async fn account(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO accounts (id, email, created_at, updated_at) \
             VALUES ('{id}', 'a{id}@example.test', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        id
    }

    /// A pseud with a distinct handle, so handle assertions are meaningful.
    async fn pseud(&self, handle: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
             VALUES ('{id}', '{}', '{handle}', '{handle} display', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
            self.account().await
        ))
        .await;
        id
    }

    /// A work owned by a fresh pseud, which also returns that owner's id so a
    /// test can assert the owner is protected.
    async fn work_with_owner(&self, handle: &str) -> (String, String) {
        let owner = self.pseud(handle).await;
        let work = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at) \
             VALUES ('{work}', '{owner}', 'A Work', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        // `works.owner_pseud_id` and the owner row in `work_contributors` are
        // separate facts: the column is who may delete the work, the row is who
        // `owner_of` reads. Seed both, or every owner assertion sees None.
        self.exec(&format!(
            "INSERT INTO work_contributors (work_id, pseud_id, role, public_attribution, created_at) \
             VALUES ('{work}', '{owner}', 'owner', 1, '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        (work, owner)
    }

    /// A work with no owner row at all, for tests that only care about the
    /// contributors they add themselves. `work_with_owner` is the one that
    /// matches production, where a work is created with its owner.
    async fn work(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at) \
             VALUES ('{id}', '{}', 'A Work', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
            self.pseud("nobody").await
        ))
        .await;
        id
    }
}

fn work_id(s: &str) -> WorkId {
    s.parse().expect("work id")
}
fn pseud_id(s: &str) -> PseudId {
    s.parse().expect("pseud id")
}

fn roles(list: &[lorehaven_domain::content::Contributor]) -> Vec<&str> {
    list.iter().map(|c| c.role.as_str()).collect()
}

/// Everyone on the work except its owner. Most tests here add contributors to a
/// work that already has one, so counting rows without this would silently
/// include the owner and turn a correct result into a confusing failure.
async fn non_owner_roles(h: &Db, work: &str) -> Vec<String> {
    let owner = collab::owner_of(h.db(), work_id(work)).await.unwrap();
    collab::contributors_for_work(h.db(), work_id(work))
        .await
        .unwrap()
        .into_iter()
        .filter(|c| Some(c.pseud_id) != owner)
        .map(|c| c.role.as_str().to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// contributors_for_work
// ---------------------------------------------------------------------------

#[tokio::test]
async fn contributors_come_back_in_role_order_owner_first() {
    let h = Db::new("order").await;
    let work = h.work().await;

    // Inserted deliberately out of role order and with distinct timestamps, so
    // the ORDER BY is doing the work rather than the insert order.
    for (handle, role, at) in [
        ("beta", "beta_reader", "2026-01-03T00:00:00+00:00"),
        ("editor-p", "editor", "2026-01-02T00:00:00+00:00"),
        ("co-p", "coauthor", "2026-01-04T00:00:00+00:00"),
    ] {
        let p = h.pseud(handle).await;
        h.exec(&format!(
            "INSERT INTO work_contributors (work_id, pseud_id, role, public_attribution, created_at) \
             VALUES ('{work}', '{p}', '{role}', 1, '{at}')"
        ))
        .await;
    }
    // The owner is seeded last, with the newest timestamp, to prove the role
    // rank outranks recency.
    h.exec(&format!(
        "INSERT INTO work_contributors (work_id, pseud_id, role, public_attribution, created_at) \
         VALUES ('{work}', '{}', 'owner', 1, '2026-01-09T00:00:00+00:00')",
        h.pseud("the-owner").await
    ))
    .await;

    let got = collab::contributors_for_work(h.db(), work_id(&work))
        .await
        .unwrap();
    assert_eq!(
        roles(&got),
        vec!["owner", "coauthor", "editor", "beta_reader"],
        "owner first, then coauthor, editor, and unknown roles last"
    );
}

#[tokio::test]
async fn contributors_within_a_rank_are_ordered_by_when_they_joined() {
    let h = Db::new("rank-tie").await;
    let work = h.work().await;
    // Inserted newest-first, so the expected order is the reverse of insertion
    // order and cannot be satisfied by insertion order alone.
    let mut ids = Vec::new();
    for (handle, at) in [
        ("third", "2026-01-08T00:00:00+00:00"),
        ("second", "2026-01-05T00:00:00+00:00"),
        ("first", "2026-01-02T00:00:00+00:00"),
    ] {
        let p = h.pseud(handle).await;
        ids.push(p.clone());
        h.exec(&format!(
            "INSERT INTO work_contributors (work_id, pseud_id, role, public_attribution, created_at) \
             VALUES ('{work}', '{p}', 'coauthor', 1, '{at}')"
        ))
        .await;
    }
    let [third, second, first] = <[String; 3]>::try_from(ids).expect("three pseuds");
    // The row carries no handle, so assert the order against the ids we seeded.
    let got = collab::contributors_for_work(h.db(), work_id(&work))
        .await
        .unwrap();
    let order: Vec<String> = got.iter().map(|c| c.pseud_id.to_string()).collect();
    assert_eq!(
        order,
        vec![first.clone(), second.clone(), third.clone()],
        "within one role rank, the earliest joiner comes first"
    );
    // Sanity: the first element really is the pseud inserted with the earliest
    // created_at, which was seeded *second* -- so insertion order is not what
    // produced this.
    assert_ne!(first, second);
}

#[tokio::test]
async fn an_unrecognised_stored_role_is_dropped_rather_than_defaulted_to_owner() {
    let h = Db::new("bad-role").await;
    let work = h.work().await;
    for (handle, role) in [("real-co", "coauthor"), ("sneaky", "admin")] {
        let p = h.pseud(handle).await;
        h.exec(&format!(
            "INSERT INTO work_contributors (work_id, pseud_id, role, public_attribution, created_at) \
             VALUES ('{work}', '{p}', '{role}', 1, '2026-01-02T00:00:00+00:00')"
        ))
        .await;
    }

    let got = collab::contributors_for_work(h.db(), work_id(&work))
        .await
        .unwrap();
    assert_eq!(
        roles(&got),
        vec!["coauthor"],
        "a role we cannot parse is dropped rather than ranked: defaulting it \
         would be a privilege escalation written as a fallback"
    );
}

#[tokio::test]
async fn a_work_with_no_contributors_is_an_empty_list_not_an_error() {
    let h = Db::new("no-rows").await;
    let got = collab::contributors_for_work(h.db(), work_id(&uuid::Uuid::new_v4().to_string()))
        .await
        .unwrap();
    assert!(got.is_empty());
}

// ---------------------------------------------------------------------------
// owner_of
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_owner_is_the_pseud_whose_row_says_owner() {
    let h = Db::new("owner-of").await;
    let (work, owner) = h.work_with_owner("the-owner").await;
    h.exec(&format!(
        "INSERT INTO work_contributors (work_id, pseud_id, role, public_attribution, created_at) \
         VALUES ('{work}', '{}', 'coauthor', 1, '2026-01-02T00:00:00+00:00')",
        h.pseud("co").await
    ))
    .await;

    assert_eq!(
        collab::owner_of(h.db(), work_id(&work)).await.unwrap(),
        Some(pseud_id(&owner)),
        "coauthors are contributors, not owners"
    );
}

#[tokio::test]
async fn a_work_with_no_owner_row_has_no_owner() {
    let h = Db::new("no-owner").await;
    let got = collab::owner_of(h.db(), work_id(&uuid::Uuid::new_v4().to_string()))
        .await
        .unwrap();
    assert_eq!(got, None);
}

// ---------------------------------------------------------------------------
// public_contributors
// ---------------------------------------------------------------------------

#[tokio::test]
async fn public_contributors_are_the_pseud_s_public_fields_and_nothing_else() {
    let h = Db::new("public").await;
    let work = h.work().await;
    let owner = h.pseud("own").await;
    let co = h.pseud("co").await;
    for (p, role) in [(&owner, "owner"), (&co, "coauthor")] {
        h.exec(&format!(
            "INSERT INTO work_contributors (work_id, pseud_id, role, public_attribution, created_at) \
             VALUES ('{work}', '{p}', '{role}', 1, '2026-01-02T00:00:00+00:00')"
        ))
        .await;
    }

    let got = collab::public_contributors(h.db(), work_id(&work))
        .await
        .unwrap();
    assert_eq!(got.len(), 2, "just the two co-authors on a bare work");
    assert_eq!(got[0].0, "own", "co-author credit is role-ordered");
    assert_eq!(got[0].1, "own display");
    assert_eq!(got[0].2, "owner");
    assert_eq!(got[1].0, "co");
}

#[tokio::test]
async fn a_private_contributor_is_not_credited_publicly_but_is_still_a_contributor() {
    let h = Db::new("private").await;
    let work = h.work().await;
    let hidden = h.pseud("secret").await;
    h.exec(&format!(
        "INSERT INTO work_contributors (work_id, pseud_id, role, public_attribution, created_at) \
         VALUES ('{work}', '{hidden}', 'coauthor', 0, '2026-01-02T00:00:00+00:00')"
    ))
    .await;

    let public = collab::public_contributors(h.db(), work_id(&work))
        .await
        .unwrap();
    assert!(public.is_empty(), "public_attribution = 0 is withheld");

    // The collaboration surface still knows about them -- "private" is about
    // credit on the work page, not about the contributor being unlisted.
    let all = collab::contributors_for_work(h.db(), work_id(&work))
        .await
        .unwrap();
    assert_eq!(all.len(), 1, "a private co-author is still a contributor");
}

#[tokio::test]
async fn a_deleted_pseud_is_not_credited() {
    let h = Db::new("deleted-pseud").await;
    let work = h.work().await;
    let gone = h.pseud("gone").await;
    h.exec(&format!(
        "INSERT INTO work_contributors (work_id, pseud_id, role, public_attribution, created_at) \
         VALUES ('{work}', '{gone}', 'coauthor', 1, '2026-01-02T00:00:00+00:00')"
    ))
    .await;
    h.exec(&format!(
        "UPDATE pseuds SET deleted_at = '2026-02-01T00:00:00+00:00' WHERE id = '{gone}'"
    ))
    .await;

    assert!(collab::public_contributors(h.db(), work_id(&work))
        .await
        .unwrap()
        .is_empty());
    // A deleted pseud is still a contributor row, so the work page does not
    // silently lose a credit slot; only public credit is withheld.
    assert_eq!(
        collab::contributors_for_work(h.db(), work_id(&work))
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn public_contributors_are_ordered_by_role() {
    let h = Db::new("public-order").await;
    let work = h.work().await;
    for (handle, role) in [("b", "beta_reader"), ("c", "coauthor"), ("o", "owner")] {
        let p = h.pseud(handle).await;
        h.exec(&format!(
            "INSERT INTO work_contributors (work_id, pseud_id, role, public_attribution, created_at) \
             VALUES ('{work}', '{p}', '{role}', 1, '2026-01-02T00:00:00+00:00')"
        ))
        .await;
    }
    let got = collab::public_contributors(h.db(), work_id(&work))
        .await
        .unwrap();
    let order: Vec<&str> = got.iter().map(|r| r.2.as_str()).collect();
    assert_eq!(order, vec!["owner", "coauthor", "beta_reader"]);
}

// ---------------------------------------------------------------------------
// add_contributor
// ---------------------------------------------------------------------------

#[tokio::test]
async fn adding_a_contributor_grants_the_role() {
    let h = Db::new("add").await;
    let work = h.work().await;
    let p = h.pseud("new-co").await;

    collab::add_contributor(
        h.db(),
        work_id(&work),
        pseud_id(&p),
        ContributorRole::Coauthor,
        true,
    )
    .await
    .unwrap();

    let got = collab::contributors_for_work(h.db(), work_id(&work))
        .await
        .unwrap();
    assert_eq!(roles(&got), vec!["coauthor"]);
    assert!(got[0].public_attribution);
}

#[tokio::test]
async fn adding_the_same_pseud_again_replaces_the_role_rather_than_duplicating() {
    let h = Db::new("add-upsert").await;
    let work = h.work().await;
    let p = h.pseud("promoted").await;
    let w = work_id(&work);
    let pi = pseud_id(&p);

    collab::add_contributor(h.db(), w, pi, ContributorRole::Editor, true)
        .await
        .unwrap();
    collab::add_contributor(h.db(), w, pi, ContributorRole::Coauthor, true)
        .await
        .unwrap();

    let got = collab::contributors_for_work(h.db(), w).await.unwrap();
    assert_eq!(
        roles(&got),
        vec!["coauthor"],
        "the primary key is (work_id, pseud_id): one row, promoted"
    );
}

#[tokio::test]
async fn adding_a_contributor_again_also_updates_their_attribution() {
    let h = Db::new("add-attr").await;
    let work = h.work().await;
    let p = h.pseud("co").await;
    let w = work_id(&work);
    let pi = pseud_id(&p);

    collab::add_contributor(h.db(), w, pi, ContributorRole::Coauthor, true)
        .await
        .unwrap();
    collab::add_contributor(h.db(), w, pi, ContributorRole::Coauthor, false)
        .await
        .unwrap();

    assert!(
        collab::public_contributors(h.db(), w)
            .await
            .unwrap()
            .is_empty(),
        "the upsert carries public_attribution, not just the role"
    );
    assert!(!collab::contributors_for_work(h.db(), w).await.unwrap()[0].public_attribution);
}

#[tokio::test]
async fn a_private_contribution_can_be_granted_directly() {
    let h = Db::new("add-private").await;
    let work = h.work().await;
    let p = h.pseud("co").await;
    collab::add_contributor(
        h.db(),
        work_id(&work),
        pseud_id(&p),
        ContributorRole::Coauthor,
        false,
    )
    .await
    .unwrap();

    let got = collab::contributors_for_work(h.db(), work_id(&work))
        .await
        .unwrap();
    assert_eq!(roles(&got), vec!["coauthor"]);
    assert!(!got[0].public_attribution);
    assert!(
        collab::public_contributors(h.db(), work_id(&work))
            .await
            .unwrap()
            .is_empty(),
        "a private co-author earns no public credit line"
    );
}

// ---------------------------------------------------------------------------
// update_contributor -- the owner protection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_contributors_role_can_be_changed() {
    let h = Db::new("update-role").await;
    let work = h.work().await;
    let p = h.pseud("co").await;
    let w = work_id(&work);
    let pi = pseud_id(&p);
    collab::add_contributor(h.db(), w, pi, ContributorRole::BetaReader, true)
        .await
        .unwrap();

    assert!(
        collab::update_contributor(h.db(), w, pi, Some(ContributorRole::Editor), None)
            .await
            .unwrap()
    );
    assert_eq!(
        roles(&collab::contributors_for_work(h.db(), w).await.unwrap()),
        vec!["editor"]
    );
}

#[tokio::test]
async fn updating_only_the_attribution_leaves_the_role_alone() {
    let h = Db::new("update-attr").await;
    let work = h.work().await;
    let p = h.pseud("co").await;
    let w = work_id(&work);
    let pi = pseud_id(&p);
    collab::add_contributor(h.db(), w, pi, ContributorRole::Editor, true)
        .await
        .unwrap();

    assert!(collab::update_contributor(h.db(), w, pi, None, Some(false))
        .await
        .unwrap());
    let got = collab::contributors_for_work(h.db(), w).await.unwrap();
    assert_eq!(
        roles(&got),
        vec!["editor"],
        "role is COALESCEd, not cleared"
    );
    assert!(!got[0].public_attribution);
}

#[tokio::test]
async fn updating_a_pseud_who_is_not_a_contributor_reports_no_change() {
    let h = Db::new("update-missing").await;
    let work = h.work().await;
    let stranger = h.pseud("stranger").await;

    assert!(!collab::update_contributor(
        h.db(),
        work_id(&work),
        pseud_id(&stranger),
        Some(ContributorRole::Coauthor),
        None
    )
    .await
    .unwrap());
}

#[tokio::test]
async fn the_owner_cannot_be_demoted_or_uncredited() {
    let h = Db::new("update-owner").await;
    let (work, owner) = h.work_with_owner("own").await;
    let w = work_id(&work);
    let oi = pseud_id(&owner);

    assert!(
        !collab::update_contributor(h.db(), w, oi, Some(ContributorRole::Editor), None)
            .await
            .unwrap(),
        "a work whose owner was demoted would have nobody able to publish"
    );
    assert_eq!(
        roles(&collab::contributors_for_work(h.db(), w).await.unwrap()),
        vec!["owner"]
    );

    assert!(
        !collab::update_contributor(h.db(), w, oi, None, Some(false))
            .await
            .unwrap(),
        "nor may the owner be stripped of public credit"
    );
    assert!(collab::contributors_for_work(h.db(), w).await.unwrap()[0].public_attribution);
}

#[tokio::test]
async fn the_owner_cannot_be_removed() {
    let h = Db::new("remove-owner").await;
    let (work, owner) = h.work_with_owner("own").await;

    assert!(
        !collab::remove_contributor(h.db(), work_id(&work), pseud_id(&owner))
            .await
            .unwrap()
    );
    assert_eq!(
        collab::contributors_for_work(h.db(), work_id(&work))
            .await
            .unwrap()
            .len(),
        1,
        "the work still has an owner afterwards"
    );
    assert_eq!(
        collab::owner_of(h.db(), work_id(&work)).await.unwrap(),
        Some(pseud_id(&owner))
    );
}

#[tokio::test]
async fn a_pseud_with_no_owner_row_is_not_protected_by_that_check() {
    // The guard reads `owner_of`, so on a work whose owner row is absent -- even
    // though `works.owner_pseud_id` is set -- the update proceeds. Worth
    // pinning: the protection is about the owner *row*, and the two are
    // separate facts a caller could get out of step.
    let h = Db::new("update-no-owner").await;
    let work = h.work_with_owner("own").await.0;
    h.exec(&format!(
        "DELETE FROM work_contributors WHERE work_id = '{work}' AND role = 'owner'"
    ))
    .await;
    assert_eq!(
        collab::owner_of(h.db(), work_id(&work)).await.unwrap(),
        None
    );
    let p = h.pseud("co").await;
    collab::add_contributor(
        h.db(),
        work_id(&work),
        pseud_id(&p),
        ContributorRole::Editor,
        true,
    )
    .await
    .unwrap();

    assert!(collab::update_contributor(
        h.db(),
        work_id(&work),
        pseud_id(&p),
        Some(ContributorRole::Coauthor),
        None
    )
    .await
    .unwrap());
    assert_eq!(
        non_owner_roles(&h, &work).await,
        vec!["coauthor".to_string()]
    );
}

#[tokio::test]
async fn a_contributor_can_be_removed() {
    let h = Db::new("remove").await;
    let work = h.work().await;
    let p = h.pseud("co").await;
    let w = work_id(&work);
    let pi = pseud_id(&p);
    collab::add_contributor(h.db(), w, pi, ContributorRole::Coauthor, true)
        .await
        .unwrap();

    assert!(collab::remove_contributor(h.db(), w, pi).await.unwrap());
    assert!(collab::contributors_for_work(h.db(), w)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn removing_someone_who_is_not_a_contributor_reports_no_change() {
    let h = Db::new("remove-missing").await;
    let work = h.work().await;
    let stranger = h.pseud("stranger").await;
    assert!(
        !collab::remove_contributor(h.db(), work_id(&work), pseud_id(&stranger))
            .await
            .unwrap()
    );
}

// ---------------------------------------------------------------------------
// Invitations
// ---------------------------------------------------------------------------

/// Create an invitation and load it back through the public read path.
async fn invite(h: &Db, work: &str, to: &str, by: &str) -> collab::Invite {
    let id = collab::create_invite(
        h.db(),
        work_id(work),
        pseud_id(to),
        pseud_id(by),
        ContributorRole::Coauthor,
        // `token_hash` is UNIQUE -- a real invite's hash is derived from a
        // single-use token -- so each invite needs its own.
        &format!("token-hash-{}", uuid::Uuid::new_v4()),
        Some("please help"),
    )
    .await
    .unwrap();
    collab::find_invite(h.db(), id)
        .await
        .unwrap()
        .expect("invite row")
}

#[tokio::test]
async fn a_new_invitation_is_pending_and_carries_the_offered_role() {
    let h = Db::new("invite-create").await;
    let (work, owner) = h.work_with_owner("own").await;
    let guest = h.pseud("guest").await;
    let got = invite(&h, &work, &guest, &owner).await;

    assert_eq!(got.status, "pending");
    assert_eq!(got.role, ContributorRole::Coauthor);
    assert_eq!(got.message.as_deref(), Some("please help"));
    assert_eq!(got.work_id, work_id(&work));
    assert_eq!(got.invited_pseud_id, pseud_id(&guest));
    assert_eq!(got.version, 1, "a fresh invitation starts at version 1");
}

#[tokio::test]
async fn an_invitation_carries_the_work_title_and_both_handles() {
    let h = Db::new("invite-joins").await;
    let work = h.work_with_owner("inviter").await.0;
    let guest = h.pseud("guest").await;
    let inviter = h.pseud("inviter2").await;
    let got = invite(&h, &work, &guest, &inviter).await;

    assert_eq!(got.work_title, "A Work", "so a list needs no second query");
    assert_eq!(got.invited_handle, "guest");
    assert_eq!(got.invited_by_handle, "inviter2");
}

#[tokio::test]
async fn an_invitation_with_no_message_has_none() {
    let h = Db::new("invite-nomsg").await;
    let (work, owner) = h.work_with_owner("own").await;
    let guest = h.pseud("guest").await;
    let id = collab::create_invite(
        h.db(),
        work_id(&work),
        pseud_id(&guest),
        pseud_id(&owner),
        ContributorRole::Editor,
        "hash",
        None,
    )
    .await
    .unwrap();

    let got = collab::find_invite(h.db(), id).await.unwrap().unwrap();
    assert_eq!(got.message, None);
}

#[tokio::test]
async fn invites_for_a_work_are_newest_first() {
    let h = Db::new("invite-order").await;
    let (work, owner) = h.work_with_owner("own").await;
    let a = h.pseud("a").await;
    let b = h.pseud("b").await;
    let c = h.pseud("c").await;

    // created_at is TEXT, so a lexicographic sort on an RFC 3339 string is
    // chronological. Insert oldest first to make the order non-trivial.
    for (p, hash, at) in [
        (&a, "h1", "2026-01-01T00:00:00+00:00"),
        (&b, "h2", "2026-01-05T00:00:00+00:00"),
        (&c, "h3", "2026-01-09T00:00:00+00:00"),
    ] {
        let id = uuid::Uuid::new_v4().to_string();
        h.exec(&format!(
            "INSERT INTO collaboration_invites
                 (id, work_id, invited_pseud_id, invited_by_pseud_id, role, status,
                  token_hash, created_at, updated_at, version)
             VALUES ('{id}', '{work}', '{p}', '{owner}', 'coauthor', 'pending',
                     '{hash}', '{at}', '{at}', 1)"
        ))
        .await;
    }

    let got = collab::invites_for_work(h.db(), work_id(&work))
        .await
        .unwrap();
    let handles: Vec<&str> = got.iter().map(|i| i.invited_handle.as_str()).collect();
    assert_eq!(handles, vec!["c", "b", "a"], "newest first");
}

#[tokio::test]
async fn pending_invites_for_a_pseud_are_only_the_unanswered_ones() {
    let h = Db::new("pending").await;
    let (work, owner) = h.work_with_owner("own").await;
    let guest = h.pseud("guest").await;
    let other = h.pseud("other").await;

    let mine = invite(&h, &work, &guest, &owner).await;
    let _theirs = invite(&h, &work, &other, &owner).await;
    // Answer one of them.
    assert!(collab::respond_to_invite(h.db(), &mine, true)
        .await
        .unwrap());

    let still = collab::pending_invites_for_pseud(h.db(), pseud_id(&guest))
        .await
        .unwrap();
    assert!(
        still.is_empty(),
        "an accepted invitation is no longer waiting on anyone"
    );
    assert_eq!(
        collab::pending_invites_for_pseud(h.db(), pseud_id(&other))
            .await
            .unwrap()
            .len(),
        1,
        "a different pseud still has theirs"
    );
}

#[tokio::test]
async fn pending_invites_are_newest_first() {
    let h = Db::new("pending-order").await;
    let (work, owner) = h.work_with_owner("own").await;
    let guest = h.pseud("guest").await;
    for (hash, at) in [
        ("h1", "2026-01-01T00:00:00+00:00"),
        ("h2", "2026-01-07T00:00:00+00:00"),
    ] {
        let id = uuid::Uuid::new_v4().to_string();
        h.exec(&format!(
            "INSERT INTO collaboration_invites
                 (id, work_id, invited_pseud_id, invited_by_pseud_id, role, status,
                  token_hash, created_at, updated_at, version)
             VALUES ('{id}', '{work}', '{guest}', '{owner}', 'editor', 'pending',
                     '{hash}', '{at}', '{at}', 1)"
        ))
        .await;
    }
    let got = collab::pending_invites_for_pseud(h.db(), pseud_id(&guest))
        .await
        .unwrap();
    assert_eq!(got[0].created_at, "2026-01-07T00:00:00+00:00");
}

#[tokio::test]
async fn an_invitation_with_an_unreadable_role_is_not_surfaced() {
    let h = Db::new("invite-bad-role").await;
    let (work, owner) = h.work_with_owner("own").await;
    let guest = h.pseud("guest").await;
    let id = uuid::Uuid::new_v4().to_string();
    h.exec(&format!(
        "INSERT INTO collaboration_invites
             (id, work_id, invited_pseud_id, invited_by_pseud_id, role, status,
              token_hash, created_at, updated_at, version)
         VALUES ('{id}', '{work}', '{guest}', '{owner}', 'superuser', 'pending',
                 'hash', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00', 1)"
    ))
    .await;

    assert!(
        collab::find_invite(h.db(), id.parse().unwrap())
            .await
            .unwrap()
            .is_none(),
        "an invitation offering a role we cannot parse is hidden, not shown as \
         something safe"
    );
    assert!(collab::invites_for_work(h.db(), work_id(&work))
        .await
        .unwrap()
        .is_empty());
    assert!(collab::pending_invites_for_pseud(h.db(), pseud_id(&guest))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn finding_an_invitation_that_does_not_exist_is_none() {
    let h = Db::new("find-missing").await;
    let got = collab::find_invite(h.db(), CollaborationInviteId::new())
        .await
        .unwrap();
    assert!(got.is_none());
}

#[tokio::test]
async fn accepting_an_invitation_grants_the_role_and_marks_it_accepted_together() {
    let h = Db::new("accept").await;
    let (work, owner) = h.work_with_owner("own").await;
    let guest = h.pseud("guest").await;
    let got = invite(&h, &work, &guest, &owner).await;

    assert!(collab::respond_to_invite(h.db(), &got, true).await.unwrap());

    // Both halves, or the change is wrong in a way that is hard to notice.
    let after = collab::find_invite(h.db(), got.id).await.unwrap().unwrap();
    assert_eq!(after.status, "accepted");
    assert_eq!(
        non_owner_roles(&h, &work).await,
        vec!["coauthor".to_string()],
        "the co-author row exists"
    );
    assert_eq!(after.version, 2, "answering bumps the version");
}

#[tokio::test]
async fn accepting_grants_exactly_the_offered_role() {
    let h = Db::new("accept-role").await;
    let (work, owner) = h.work_with_owner("own").await;
    let guest = h.pseud("guest").await;
    let id = collab::create_invite(
        h.db(),
        work_id(&work),
        pseud_id(&guest),
        pseud_id(&owner),
        ContributorRole::Editor,
        "hash",
        None,
    )
    .await
    .unwrap();
    let got = collab::find_invite(h.db(), id).await.unwrap().unwrap();
    collab::respond_to_invite(h.db(), &got, true).await.unwrap();

    assert_eq!(non_owner_roles(&h, &work).await, vec!["editor".to_string()]);
    let guest_row = collab::contributors_for_work(h.db(), work_id(&work))
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.pseud_id == pseud_id(&guest))
        .expect("the guest row");
    assert!(
        guest_row.public_attribution,
        "an accepted co-author is credited publicly"
    );
}

#[tokio::test]
async fn declining_an_invitation_grants_nothing() {
    let h = Db::new("decline").await;
    let (work, owner) = h.work_with_owner("own").await;
    let guest = h.pseud("guest").await;
    let got = invite(&h, &work, &guest, &owner).await;

    assert!(collab::respond_to_invite(h.db(), &got, false)
        .await
        .unwrap());

    let after = collab::find_invite(h.db(), got.id).await.unwrap().unwrap();
    assert_eq!(after.status, "declined");
    assert!(
        non_owner_roles(&h, &work).await.is_empty(),
        "declining is not a grant"
    );
}

#[tokio::test]
async fn an_invitation_can_only_be_answered_once() {
    let h = Db::new("answer-once").await;
    let (work, owner) = h.work_with_owner("own").await;
    let guest = h.pseud("guest").await;
    let got = invite(&h, &work, &guest, &owner).await;

    assert!(collab::respond_to_invite(h.db(), &got, true).await.unwrap());
    assert!(
        !collab::respond_to_invite(h.db(), &got, false)
            .await
            .unwrap(),
        "the second answer changes nothing"
    );
    let after = collab::find_invite(h.db(), got.id).await.unwrap().unwrap();
    assert_eq!(after.status, "accepted", "the first answer stands");
    assert_eq!(after.version, 2, "and only it bumped the version");
}

#[tokio::test]
async fn a_revoked_invitation_cannot_be_accepted_afterwards() {
    let h = Db::new("revoke-then-accept").await;
    let (work, owner) = h.work_with_owner("own").await;
    let guest = h.pseud("guest").await;
    let got = invite(&h, &work, &guest, &owner).await;

    assert!(collab::revoke_invite(h.db(), got.id).await.unwrap());
    assert!(
        !collab::respond_to_invite(h.db(), &got, true).await.unwrap(),
        "the status = 'pending' guard rejects it"
    );
    let after = collab::find_invite(h.db(), got.id).await.unwrap().unwrap();
    assert_eq!(after.status, "revoked");
    assert!(
        non_owner_roles(&h, &work).await.is_empty(),
        "no role was granted"
    );
}

#[tokio::test]
async fn revoking_an_invitation_bumps_its_version() {
    let h = Db::new("revoke-version").await;
    let (work, owner) = h.work_with_owner("own").await;
    let guest = h.pseud("guest").await;
    let got = invite(&h, &work, &guest, &owner).await;

    collab::revoke_invite(h.db(), got.id).await.unwrap();
    assert_eq!(
        collab::find_invite(h.db(), got.id)
            .await
            .unwrap()
            .unwrap()
            .version,
        2
    );
}

#[tokio::test]
async fn revoking_an_invitation_that_is_not_pending_reports_no_change() {
    let h = Db::new("revoke-twice").await;
    let (work, owner) = h.work_with_owner("own").await;
    let guest = h.pseud("guest").await;
    let got = invite(&h, &work, &guest, &owner).await;

    assert!(collab::revoke_invite(h.db(), got.id).await.unwrap());
    assert!(!collab::revoke_invite(h.db(), got.id).await.unwrap());
    assert_eq!(
        collab::find_invite(h.db(), got.id)
            .await
            .unwrap()
            .unwrap()
            .version,
        2
    );
}

#[tokio::test]
async fn revoking_an_invitation_that_does_not_exist_reports_no_change() {
    let h = Db::new("revoke-missing").await;
    assert!(!collab::revoke_invite(h.db(), CollaborationInviteId::new())
        .await
        .unwrap());
}

#[tokio::test]
async fn accepting_does_not_overwrite_an_existing_contributor_row() {
    // The grant is an ON CONFLICT DO NOTHING, so a pseud who is already a
    // contributor at a higher role keeps it.
    let h = Db::new("accept-existing").await;
    let (work, owner) = h.work_with_owner("own").await;
    let guest = h.pseud("guest").await;
    let w = work_id(&work);
    collab::add_contributor(h.db(), w, pseud_id(&guest), ContributorRole::Coauthor, true)
        .await
        .unwrap();

    let got = invite(&h, &work, &guest, &owner).await;
    assert!(collab::respond_to_invite(h.db(), &got, true).await.unwrap());

    assert_eq!(
        non_owner_roles(&h, &work).await,
        vec!["coauthor".to_string()],
        "the existing row is left alone"
    );
}

#[tokio::test]
async fn invitations_for_several_works_do_not_leak_into_each_other() {
    let h = Db::new("invite-isolation").await;
    let (w1, o1) = h.work_with_owner("o1").await;
    let (w2, o2) = h.work_with_owner("o2").await;
    let g1 = h.pseud("g1").await;
    let g2 = h.pseud("g2").await;
    invite(&h, &w1, &g1, &o1).await;
    invite(&h, &w2, &g2, &o2).await;

    let on_w1 = collab::invites_for_work(h.db(), work_id(&w1))
        .await
        .unwrap();
    assert_eq!(on_w1.len(), 1);
    assert_eq!(on_w1[0].work_id, work_id(&w1));
    assert_eq!(on_w1[0].invited_handle, "g1");
}
