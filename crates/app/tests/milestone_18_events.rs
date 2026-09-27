//! M18 — Collections, challenges, requests, claims, wishlists and events
//! (`crates/db/src/events.rs`).
//!
//! Twenty-four public functions with no test referencing any of them — the
//! largest remaining zero-coverage db module once the alias-aware scan
//! (module-or-alias `::fn`) is used rather than the plain `module::fn` form.
//!
//! Three things in here are worth writing a test for rather than reading:
//!
//!   * **Privacy depends on ids parsing as UUIDs.** `get_wishlist` builds
//!     `AccountId::from_uuid(Uuid::parse_str(account).unwrap_or_default())`.
//!     `unwrap_or_default` turns a non-UUID account id into the *nil* UUID, so
//!     two distinct non-UUID accounts compare equal, and an owner id that fails
//!     to parse can equal a viewer id that also fails. A seed using a readable
//!     id like `acct-owner` therefore tests nothing about privacy. Every id
//!     here is a real UUID for that reason.
//!   * **`claim_request` is a conditional insert.** `INSERT … SELECT … WHERE
//!     NOT EXISTS (an unfulfilled claim)`, so it reports `rows_affected() > 0`.
//!     Whether a second claimant is refused is the whole point of the function,
//!     and it is only visible when the request already has a live claim.
//!   * **`fulfil_claim` rejects (work, claimant) duplication, not (work,
//!     request) duplication.** The same work may fulfil two different requests
//!     from one claimant; the same work may not fulfil two claims for the same
//!     request. Those are different rules and a test that conflates them passes
//!     under either implementation.
//!   * **A duplicate is a soft `false`, not an error (M18-06).** `fulfil_claim`
//!     and `join_event` used to disagree: one bailed, the other let the unique
//!     key raise. Both are `Result<bool>`, so a caller following both signatures
//!     correctly was still wrong on one of them. A test that asserts a duplicate
//!     *errors* pins that asymmetry back in, so these say `false` deliberately.

use std::path::PathBuf;

use lorehaven_db::events as ev;
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m18-{tag}-{}-{:?}",
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

/// An account id that survives `Uuid::parse_str`. The privacy logic in
/// `get_wishlist` parses its account argument, so a readable id would collapse
/// to the nil UUID and make owner and viewer indistinguishable.
fn account(_tag: &str) -> String {
    uuid::Uuid::new_v4().to_string()
}

// ------------------------------------------------------------- collections

#[tokio::test]
async fn a_collection_round_trips_with_its_privacy_flag() {
    let h = Db::new("collection").await;
    let owner = account("o");
    let id = ev::create_collection(
        h.db(),
        "Reading for later",
        Some("things to come back to"),
        &owner,
        "any",
        false,
    )
    .await
    .expect("create collection");

    let c = ev::get_collection(h.db(), &id)
        .await
        .expect("read")
        .expect("the collection is there");
    assert_eq!(c.name, "Reading for later");
    assert_eq!(c.description.as_deref(), Some("things to come back to"));
    assert_eq!(c.owner, owner);
    assert_eq!(c.item_policy, "any");
    assert!(
        !c.is_public,
        "a collection created private reads back private -- is_public is stored \
         as an integer on both dialects (i64::from(is_public)), not a bool"
    );
    assert!(
        ev::list_collection_items(h.db(), &id)
            .await
            .expect("list items")
            .is_empty(),
        "a new collection has no items, and they are read through the items          function rather than off the collection"
    );
}

#[tokio::test]
async fn only_public_collections_are_listed() {
    let h = Db::new("public-list").await;
    let owner = account("o");
    ev::create_collection(h.db(), "Open shelf", None, &owner, "any", true)
        .await
        .expect("public");
    ev::create_collection(h.db(), "Private shelf", None, &owner, "any", false)
        .await
        .expect("private");

    let listed = ev::list_public_collections(h.db(), 50).await.expect("list");
    assert_eq!(
        listed.len(),
        1,
        "a private collection is not offered to the public listing"
    );
    assert_eq!(listed[0].name, "Open shelf");
}

#[tokio::test]
async fn collection_items_accumulate_and_carry_their_note() {
    let h = Db::new("items").await;
    let owner = account("o");
    let c = ev::create_collection(h.db(), "Shelf", None, &owner, "any", true)
        .await
        .expect("create");
    let work = uuid::Uuid::new_v4().to_string();

    ev::add_collection_item(h.db(), &c, &work, &owner, Some("for the reread list"))
        .await
        .expect("add with a note");
    ev::add_collection_item(h.db(), &c, &uuid::Uuid::new_v4().to_string(), &owner, None)
        .await
        .expect("add without a note");

    let items = ev::list_collection_items(h.db(), &c)
        .await
        .expect("list items");
    assert_eq!(items.len(), 2, "both items are on the collection");
    let noted = items
        .iter()
        .find(|i| i.work_id == work)
        .expect("the work we added");
    assert_eq!(
        noted.note.as_deref(),
        Some("for the reread list"),
        "the note round-trips"
    );
    assert!(
        items.iter().any(|i| i.note.is_none()),
        "an item added without a note has none rather than an empty string"
    );

    assert_eq!(
        ev::get_collection(h.db(), &c)
            .await
            .expect("read")
            .expect("row")
            .name,
        "Shelf",
        "the collection itself is still readable after items were added to it"
    );
    assert_eq!(
        ev::list_collection_items(h.db(), &c)
            .await
            .expect("list again")
            .len(),
        2,
        "and the items are still both there"
    );
}

// ---------------------------------------------------------- request claims

#[tokio::test]
async fn a_request_can_be_claimed_only_while_it_is_unclaimed() {
    let h = Db::new("claim").await;
    let requester = account("r");
    let r = ev::create_request(h.db(), &requester, "a fic about a lighthouse", None)
        .await
        .expect("create request");

    assert!(
        ev::claim_request(h.db(), &r, &account("c1"))
            .await
            .expect("first claim"),
        "the first claimant gets the request"
    );
    assert!(
        !ev::claim_request(h.db(), &r, &account("c2"))
            .await
            .expect("second claim"),
        "a second claimant is refused while a live claim exists -- this is the \
         WHERE NOT EXISTS guard, and it is invisible unless a claim already exists"
    );
    assert!(
        !ev::claim_request(h.db(), &r, &account("c3"))
            .await
            .expect("third claim"),
        "and it stays refused for everyone else"
    );
}

#[tokio::test]
async fn a_fulfilled_request_can_be_claimed_again() {
    let h = Db::new("claim-reopen").await;
    let r = ev::create_request(h.db(), &account("r"), "a second lighthouse fic", None)
        .await
        .expect("create");

    let first = account("c1");
    assert!(ev::claim_request(h.db(), &r, &first).await.expect("claim"));
    assert!(
        ev::fulfil_claim(h.db(), &r, &first, &uuid::Uuid::new_v4().to_string())
            .await
            .expect("fulfil"),
        "the claimant fulfils the request"
    );
    assert!(
        ev::claim_request(h.db(), &r, &account("c2"))
            .await
            .expect("re-claim"),
        "once the first claim is fulfilled it no longer blocks, so the guard on \
         `fulfilled_at IS NULL` lets a new claimant in"
    );
}

#[tokio::test]
async fn one_work_cannot_fulfil_the_same_request_twice() {
    let h = Db::new("fulfil-dup").await;
    let r = ev::create_request(h.db(), &account("r"), "a lighthouse fic", None)
        .await
        .expect("create");
    let claimant = account("c");
    let work = uuid::Uuid::new_v4().to_string();
    ev::claim_request(h.db(), &r, &claimant)
        .await
        .expect("claim first");

    assert!(
        ev::fulfil_claim(h.db(), &r, &claimant, &work)
            .await
            .expect("first fulfil"),
        "the first fulfilment of a request goes through"
    );
    // The second call is a soft `false`, matching `join_event` (M18-06). The
    // anti-gaming rule is unchanged in force -- the duplicate is still refused --
    // but refusal is reported the same way as every other "no" from this
    // function, so a caller does not have to know that this particular refusal
    // used to arrive as an error.
    let second = ev::fulfil_claim(h.db(), &r, &claimant, &work).await;
    assert!(
        !second.expect("a duplicate is refused, not raised"),
        "the same work cannot fulfil the same request a second time"
    );
}

#[tokio::test]
async fn fulfilling_without_a_claim_reports_false_rather_than_erroring() {
    let h = Db::new("fulfil-noclaim").await;
    let r = ev::create_request(h.db(), &account("r"), "an unclaimed lighthouse fic", None)
        .await
        .expect("create");

    assert!(
        !ev::fulfil_claim(h.db(), &r, &account("c"), &uuid::Uuid::new_v4().to_string())
            .await
            .expect("fulfil with no claim"),
        "fulfilling a request this claimant never claimed updates nothing and \
         reports false -- the doc comment's stated contract"
    );
}

#[tokio::test]
async fn the_anti_gaming_rule_is_per_work_and_claimant() {
    let h = Db::new("fulfil-two").await;
    let claimant = account("c");
    let first = ev::create_request(h.db(), &account("r1"), "a lighthouse fic", None)
        .await
        .expect("create one");
    let second = ev::create_request(h.db(), &account("r2"), "a lighthouse sequel", None)
        .await
        .expect("create two");
    let work = uuid::Uuid::new_v4().to_string();
    ev::claim_request(h.db(), &first, &claimant)
        .await
        .expect("claim one");
    ev::claim_request(h.db(), &second, &claimant)
        .await
        .expect("claim two");

    // The rule is `(fulfilled_by_work, claimant)`, so the same work cannot
    // fulfil a *second* claim held by the same claimant even when the requests
    // differ. An earlier comment here said the opposite; the SQL was always this
    // way, and the comment was the thing that was wrong.
    assert!(
        ev::fulfil_claim(h.db(), &first, &claimant, &work)
            .await
            .expect("first"),
        "one work fulfils the first of a claimant's two claims"
    );
    assert!(
        !ev::fulfil_claim(h.db(), &second, &claimant, &work)
            .await
            .expect("refused, not raised"),
        "and it cannot fulfil the second claim from the same claimant"
    );

    // A different claimant holding the same work is a different (work, claimant)
    // pair, so the rule does not fire. They need a request of their own: a
    // request holds at most one unfulfilled claim, so a second claimant cannot
    // take over one that is still live.
    let third = ev::create_request(h.db(), &account("r3"), "a lighthouse anthology", None)
        .await
        .expect("create three");
    let other = account("c2");
    assert!(
        !ev::claim_request(h.db(), &second, &other)
            .await
            .expect("take over a live claim"),
        "a second claimant cannot take a request that already has a live claim"
    );
    ev::claim_request(h.db(), &third, &other)
        .await
        .expect("claim three");
    assert!(
        ev::fulfil_claim(h.db(), &third, &other, &work)
            .await
            .expect("another claimant"),
        "and the same work fulfils a claim held by a *different* claimant -- the \
         constraint is the (work, claimant) pair, not the work alone"
    );
}

#[tokio::test]
async fn claims_expire_once_past_the_grace_period() {
    let h = Db::new("expire").await;
    let r = ev::create_request(h.db(), &account("r"), "a lighthouse fic", None)
        .await
        .expect("create");
    ev::claim_request(h.db(), &r, &account("c1"))
        .await
        .expect("claim");

    // `expire_claims` expires claims whose `claimed_at` is older than
    // `now - grace_seconds`, and `claim_request` stamps `claimed_at` from the
    // wall clock. A zero grace period therefore expires everything already
    // claimed, and the caller's clock is the only lever -- a negative grace is
    // clamped to zero (`grace_seconds.max(0)`), so it cannot rewind it.
    let now = now_rfc3339();
    assert_eq!(
        ev::expire_claims(h.db(), &now, 3600).await.expect("expire"),
        0,
        "a claim made moments ago is not past a one-hour grace period: the \
         cutoff is now-3600, which is before it was claimed"
    );
    assert_eq!(
        ev::expire_claims(h.db(), &add_seconds(&now, 7200), 0)
            .await
            .expect("expire"),
        1,
        "with the clock two hours ahead even a zero grace period reaches it, and \
         exactly one claim expired"
    );
    // The second call must expire nothing (M18-05).
    //
    // The statement used to filter on `fulfilled_at IS NULL` and then *set*
    // `fulfilled_at = NULL`, so the row came out of the statement in exactly the
    // state the statement matched on. A scheduled job therefore reported the
    // same stale claim as expired on every pass, and the count never fell to
    // zero for a request nobody ever fulfilled. `claims.status` is the fix: an
    // expired claim leaves the `status = 'outstanding'` set, so the statement
    // does not match it again.
    assert_eq!(
        ev::expire_claims(h.db(), &add_seconds(&now, 7200), 0)
            .await
            .expect("expire again"),
        0,
        "a claim is expired exactly once: the second pass finds nothing to do"
    );
    // And a third pass, to rule out a count that is merely lagging.
    assert_eq!(
        ev::expire_claims(h.db(), &add_seconds(&now, 14400), 0)
            .await
            .expect("expire a third time"),
        0,
        "still nothing: the claim is expired, not outstanding"
    );
}

#[tokio::test]
async fn an_expired_claim_can_be_claimed_again() {
    // The other half of M18-05, and the one a fix for the counting bug can
    // easily break. "At most one active claim" has to mean one *outstanding*
    // claim; if the guard still filtered on `fulfilled_at IS NULL`, then an
    // expired claim would keep satisfying it and the request could never be
    // picked up again. The expiry would have released the work for nobody,
    // which is a worse defect than the one it fixed.
    let h = Db::new("reclaim").await;
    let requester = account("rq");
    let r = ev::create_request(h.db(), &requester, "a very specific request", None)
        .await
        .expect("create");

    let first = account("c1");
    assert!(
        ev::claim_request(h.db(), &r, &first).await.expect("claim"),
        "the first claim succeeds"
    );

    // Past the grace period, the first claim expires.
    let now = now_rfc3339();
    assert_eq!(
        ev::expire_claims(h.db(), &add_seconds(&now, 7200), 0)
            .await
            .expect("expire"),
        1
    );

    // The original claimant may re-claim it. Checked *first*, and on its own
    // database state: once another reader claims, the row is outstanding again
    // and this same call is correctly refused. Testing it after a second reader
    // takes it would assert a bug and read as a fix.
    assert!(
        ev::claim_request(h.db(), &r, &first)
            .await
            .expect("re-claim"),
        "the original claimant may pick their claim back up after the expiry"
    );

    // And a different reader cannot take it while it is outstanding again.
    assert!(
        !ev::claim_request(h.db(), &r, &account("c2"))
            .await
            .expect("second"),
        "and the re-claim is live, so a second reader is still refused"
    );
}

#[tokio::test]
async fn joining_an_event_twice_is_false_not_an_error() {
    // M18-06. `join_event`'s signature and doc comment both promised "false if
    // already joined" while the statement was a bare INSERT against a composite
    // primary key, so the unique violation escaped through `?` and a caller
    // following the documentation got an error where they expected a bool.
    let h = Db::new("join-twice").await;
    let event = ev::create_event(h.db(), "a reading event", "", &account("host"))
        .await
        .expect("create event");
    let account = account("joiner");

    assert!(
        ev::join_event(h.db(), &event, &account)
            .await
            .expect("first join"),
        "the first join is a change"
    );
    assert!(
        !ev::join_event(h.db(), &event, &account)
            .await
            .expect("second join"),
        "the second join is reported as a no-op, not raised as a constraint \
         violation -- a duplicate is a normal outcome for this function"
    );
    // And the row is still there exactly once.
    let participants = ev::list_event_participants(h.db(), &event)
        .await
        .expect("list");
    assert_eq!(
        participants.len(),
        1,
        "re-joining must not duplicate the participation row: {participants:?}"
    );
}

#[tokio::test]
async fn a_public_wishlist_is_visible_to_anyone() {
    let h = Db::new("wl-public").await;
    let owner = account("o");
    ev::upsert_wishlist(h.db(), &owner, true)
        .await
        .expect("make public");

    let seen = ev::get_wishlist(h.db(), &owner, Some(&account("v")))
        .await
        .expect("read")
        .expect("a public wishlist is visible to another account");
    assert!(seen.is_public);
    assert!(seen.items.is_empty());
}

#[tokio::test]
async fn a_private_wishlist_is_visible_only_to_its_owner() {
    let h = Db::new("wl-private").await;
    let owner = account("o");
    ev::upsert_wishlist(h.db(), &owner, false)
        .await
        .expect("make private");

    assert!(
        ev::get_wishlist(h.db(), &owner, Some(&owner))
            .await
            .expect("owner reads")
            .is_some(),
        "the owner sees their own private wishlist"
    );
    assert!(
        ev::get_wishlist(h.db(), &owner, Some(&account("v")))
            .await
            .expect("stranger reads")
            .is_none(),
        "another account does not -- an absent wishlist and a hidden one are \
         indistinguishable from outside, which is the point"
    );
    assert!(
        ev::get_wishlist(h.db(), &owner, None)
            .await
            .expect("anonymous reads")
            .is_none(),
        "and neither does an anonymous viewer"
    );
}

#[tokio::test]
async fn two_owners_private_wishlists_do_not_leak_into_each_other() {
    let h = Db::new("wl-two").await;
    let first = account("a");
    let second = account("b");
    ev::upsert_wishlist(h.db(), &first, false)
        .await
        .expect("first");
    ev::upsert_wishlist(h.db(), &second, false)
        .await
        .expect("second");

    // If the owner id failed to parse, `unwrap_or_default` would give both the
    // nil UUID and each would be seen as its own owner. With real UUIDs the
    // two are distinct, so the cross-read is refused.
    assert!(
        ev::get_wishlist(h.db(), &first, Some(&second))
            .await
            .expect("cross-read")
            .is_none(),
        "one owner cannot read another's private wishlist"
    );
    assert!(
        ev::get_wishlist(h.db(), &second, Some(&first))
            .await
            .expect("cross-read back")
            .is_none(),
        "nor the other way round"
    );
}

#[tokio::test]
async fn a_wishlist_can_be_made_private_again() {
    let h = Db::new("wl-toggle").await;
    let owner = account("o");
    ev::upsert_wishlist(h.db(), &owner, true)
        .await
        .expect("public");
    ev::add_wishlist_item(
        h.db(),
        &owner,
        None,
        Some(&uuid::Uuid::new_v4().to_string()),
        Some("a sequel"),
    )
    .await
    .expect("add an item");

    ev::upsert_wishlist(h.db(), &owner, false)
        .await
        .expect("make private again");
    let mine = ev::get_wishlist(h.db(), &owner, Some(&owner))
        .await
        .expect("read")
        .expect("still mine");
    assert!(
        !mine.is_public,
        "the upsert changed the flag, not just the row count"
    );
    assert_eq!(mine.items.len(), 1, "and kept the items that were on it");
    assert!(
        ev::get_wishlist(h.db(), &owner, Some(&account("v")))
            .await
            .expect("stranger")
            .is_none(),
        "after the change another account cannot see it"
    );
}

#[tokio::test]
async fn wishlist_items_keep_their_note_and_work_reference() {
    let h = Db::new("wl-items").await;
    let owner = account("o");
    ev::upsert_wishlist(h.db(), &owner, true)
        .await
        .expect("public");
    let work = uuid::Uuid::new_v4().to_string();
    let node = uuid::Uuid::new_v4().to_string();

    ev::add_wishlist_item(h.db(), &owner, Some(&node), None, Some("a pairing"))
        .await
        .expect("a node item");
    ev::add_wishlist_item(h.db(), &owner, None, Some(&work), None)
        .await
        .expect("a work item");

    let wl = ev::get_wishlist(h.db(), &owner, Some(&owner))
        .await
        .expect("read")
        .expect("row");
    assert_eq!(wl.items.len(), 2);
    let by_node = wl
        .items
        .iter()
        .find(|i| i.node_id.as_deref() == Some(node.as_str()))
        .expect("the node item");
    assert_eq!(by_node.work_id, None, "a node item has no work");
    assert_eq!(by_node.note.as_deref(), Some("a pairing"));
    let by_work = wl
        .items
        .iter()
        .find(|i| i.work_id.as_deref() == Some(work.as_str()))
        .expect("the work item");
    assert_eq!(by_work.node_id, None, "a work item has no node");
}

#[tokio::test]
async fn an_account_with_no_wishlist_reads_as_none() {
    let h = Db::new("wl-absent").await;
    assert!(
        ev::get_wishlist(h.db(), &account("nobody"), Some(&account("v")))
            .await
            .expect("read")
            .is_none(),
        "an account that never made a wishlist has none to see"
    );
}

// ------------------------------------------------------------------ events

#[tokio::test]
async fn joining_an_event_records_the_participant_and_refuses_a_rejoin() {
    let h = Db::new("event").await;
    let e = ev::create_event(
        h.db(),
        "Winter Readathon",
        r#"{"goal": 50000}"#,
        &account("o"),
    )
    .await
    .expect("create event");

    let participant = account("p1");
    assert!(
        ev::join_event(h.db(), &e, &participant)
            .await
            .expect("join"),
        "joining an event reports true"
    );
    // A rejoin is a soft `false`, not an error (M18-06). `event_participation`
    // has a composite primary key on (event_id, account); the statement is now
    // `ON CONFLICT DO NOTHING`, so the duplicate is answered in the statement
    // rather than raised through `?`. `fulfil_claim` reports its duplicate the
    // same way, so a caller needs no per-function knowledge of which duplicates
    // are errors.
    let rejoin = ev::join_event(h.db(), &e, &participant).await;
    assert!(
        !rejoin.expect("a duplicate join is not an error"),
        "joining an event twice is a no-op reported as false, not a raised \
         constraint violation"
    );
    ev::join_event(h.db(), &e, &account("p2"))
        .await
        .expect("second participant");

    let participants = ev::list_event_participants(h.db(), &e).await.expect("list");
    assert_eq!(
        participants.len(),
        2,
        "two distinct accounts, and the duplicate did not add a third row"
    );
    assert!(participants.contains(&participant));
}

#[tokio::test]
async fn an_event_round_trips_its_document() {
    let h = Db::new("event-get").await;
    let e = ev::create_event(
        h.db(),
        "Summer Sprint",
        r#"{"goal": 100000, "unit": "words"}"#,
        &account("o"),
    )
    .await
    .expect("create");

    let got = ev::get_event(h.db(), &e).await.expect("read").expect("row");
    assert_eq!(got.name, "Summer Sprint");
    assert!(
        got.document.contains("100000"),
        "the event document survives the round trip: {}",
        got.document
    );
    assert!(
        ev::get_event(h.db(), "no-such-event")
            .await
            .expect("read missing")
            .is_none(),
        "an event that is not there reads as None"
    );
}

/// The current time in the format the module stores timestamps in, via the same
/// helper the module itself uses -- so the two cannot disagree about the shape
/// of the text that gets compared lexically.
fn now_rfc3339() -> String {
    lorehaven_db::identity::now_rfc3339()
}

/// `base` shifted by `secs`, formatted by the module's own formatter.
fn add_seconds(base: &str, secs: i64) -> String {
    let at = time::OffsetDateTime::parse(base, &time::format_description::well_known::Rfc3339)
        .expect("parse base");
    lorehaven_db::identity::format_rfc3339(
        at.checked_add(time::Duration::seconds(secs))
            .expect("no overflow"),
    )
}
