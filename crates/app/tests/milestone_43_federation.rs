//! M43 — ActivityPub federation: actors, activities, follows, instance
//! fingerprints, peer similarity and the delivery queue
//! (`crates/db/src/federation.rs`).
//!
//! Fourteen public functions, none of them referenced by any test. Two things
//! here are worth a test rather than a read, and both are pinned as known
//! defects rather than fixed:
//!
//!   * **`regenerate_fingerprint` is an INSERT against a UNIQUE column.** The
//!     function is named for regenerating a fingerprint and is the only way to
//!     refresh one, but `instance_fingerprints.instance_host` is `UNIQUE` and
//!     the statement has no `ON CONFLICT` clause — so the second call for a
//!     host is a unique violation, not a refresh. Every fingerprint in the
//!     database is therefore permanent after its first generation, and the
//!     30-day `valid_until` means it silently stops being returned by
//!     `get_fingerprint` rather than ever being replaced.
//!   * **`mark_queue_failed` is terminal.** It sets `status = 'failed'` and
//!     increments `attempts`, but `get_pending_queue` only selects
//!     `status = 'pending'`, and the schema has exactly three states. So a
//!     delivery that fails once is never retried, and the `attempts` column
//!     that exists to drive a backoff is written and never read.
//!
//! The `f64` binds and the `f64` tuple reads are the other thing to watch here:
//! `similarity` is `REAL` on SQLite and `f64` decodes from it, but on PostgreSQL
//! the peer queries read a `REAL` column into an `f64` tuple directly, which
//! only works because the driver maps `REAL` to `f64`. The
//! `fingerprint_version` read is the interesting one -- it is `INTEGER` and the
//! PostgreSQL arm has to `CAST(... AS BIGINT)` to decode into an `i64`, the same
//! trap that broke `category_governance` and `thread_modes`.

use std::path::PathBuf;

use lorehaven_db::federation as fed;
use serde_json::json;
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m43-{tag}-{}-{:?}",
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
    /// A remote actor with every optional column populated.
    async fn actor(&self, tag: &str) -> String {
        fed::create_actor(
            self.db(),
            "person",
            None,
            Some("example.test"),
            &format!("https://example.test/actors/{tag}"),
            &format!("https://example.test/inbox/{tag}"),
            &format!("https://example.test/outbox/{tag}"),
            Some(&format!("https://example.test/followers/{tag}")),
            Some(&format!("https://example.test/following/{tag}")),
            "public-key",
        )
        .await
        .expect("create actor")
    }
    async fn activity(&self, actor_id: &str, tag: &str) -> String {
        fed::create_activity(
            self.db(),
            "Create",
            actor_id,
            Some(&format!("object-{tag}")),
            Some("Note"),
            &json!({"tag": tag}),
        )
        .await
        .expect("create activity")
    }
}

// ------------------------------------------------------------------- actors

#[tokio::test]
async fn an_actor_round_trips_with_every_column() {
    let h = Db::new("actor").await;
    let id = h.actor("a1").await;
    let got = fed::get_actor_by_ap_id(h.db(), "https://example.test/actors/a1")
        .await
        .unwrap()
        .expect("found by ap_id");
    assert_eq!(got.id, id);
    assert_eq!(got.actor_type, "person");
    assert_eq!(got.instance_host.as_deref(), Some("example.test"));
    assert_eq!(got.ap_id, "https://example.test/actors/a1");
    assert_eq!(got.inbox_url, "https://example.test/inbox/a1");
    assert_eq!(got.outbox_url, "https://example.test/outbox/a1");
    assert_eq!(
        got.followers_url.as_deref(),
        Some("https://example.test/followers/a1")
    );
    assert_eq!(got.public_key, "public-key");
}

#[tokio::test]
async fn a_local_actor_may_have_no_instance_host_and_no_user() {
    let h = Db::new("actor-local").await;
    fed::create_actor(
        h.db(),
        "instance",
        None,
        None,
        "https://local.test/instance",
        "https://local.test/inbox",
        "https://local.test/outbox",
        None,
        None,
        "key",
    )
    .await
    .unwrap();
    let got = fed::get_actor_by_ap_id(h.db(), "https://local.test/instance")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.instance_host, None, "a local actor has no remote host");
    assert_eq!(got.user_id, None);
    assert_eq!(
        got.followers_url, None,
        "the optional URLs stay null, not empty strings"
    );
}

#[tokio::test]
async fn an_unknown_ap_id_is_none_rather_than_an_error() {
    let h = Db::new("actor-missing").await;
    assert!(
        fed::get_actor_by_ap_id(h.db(), "https://nowhere.test/actors/x")
            .await
            .unwrap()
            .is_none()
    );
}

// --------------------------------------------------------------- activities

#[tokio::test]
async fn an_activity_stores_its_payload_and_object() {
    let h = Db::new("activity").await;
    let actor = h.actor("a1").await;
    let id = h.activity(&actor, "one").await;
    // Read it back through the queue path, which is the only reader here.
    let queued = fed::enqueue_activity(h.db(), &id, "https://peer.test/inbox")
        .await
        .unwrap();
    let pending = fed::get_pending_queue(h.db(), 10).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].0, queued);
    assert_eq!(
        pending[0].1, id,
        "the activity id is what the queue carries"
    );
    assert_eq!(pending[0].2, "https://peer.test/inbox");
}

#[tokio::test]
async fn an_activity_needs_a_real_actor_because_of_the_foreign_key() {
    let h = Db::new("activity-fk").await;
    // `ap_activities.actor_id` references `ap_actors(id)`, so an activity for an
    // actor that was never created is a constraint violation on both backends.
    let err = fed::create_activity(
        h.db(),
        "Create",
        &uuid::Uuid::new_v4().to_string(),
        None,
        None,
        &json!({}),
    )
    .await;
    assert!(
        err.is_err(),
        "the foreign key is enforced, not silently ignored"
    );
}

// ------------------------------------------------------------------ follows

#[tokio::test]
async fn a_follow_starts_unaccepted_and_can_be_accepted() {
    let h = Db::new("follow").await;
    let a = h.actor("a1").await;
    let b = h.actor("a2").await;
    let follow = fed::create_follow(h.db(), &a, &b).await.unwrap();
    fed::accept_follow(h.db(), &follow).await.unwrap();
    // No read helper exists for follows, so assert through the raw row.
    let q = h.tdb.sql(&format!(
        "SELECT accepted FROM ap_follows WHERE id = '{follow}'"
    ));
    let v: bool = match h.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&q)
            .fetch_one(h.db().sqlite_pool().expect("sqlite"))
            .await
            .unwrap(),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&q)
            .fetch_one(h.db().postgres_pool().expect("pg"))
            .await
            .unwrap(),
    };
    assert!(v, "the follow is accepted");
}

#[tokio::test]
async fn accepting_an_unknown_follow_is_a_quiet_no_op() {
    let h = Db::new("follow-missing").await;
    fed::accept_follow(h.db(), &uuid::Uuid::new_v4().to_string())
        .await
        .expect("no error for a follow that does not exist");
}

#[tokio::test]
async fn a_follow_needs_two_real_actors() {
    let h = Db::new("follow-fk").await;
    let a = h.actor("a1").await;
    assert!(
        fed::create_follow(h.db(), &a, &uuid::Uuid::new_v4().to_string())
            .await
            .is_err(),
        "the followed actor is a foreign key too"
    );
}

// ------------------------------------------------------------ fingerprints

#[tokio::test]
async fn a_fingerprint_is_generated_with_a_thirty_day_validity() {
    let h = Db::new("fp-gen").await;
    let out = fed::regenerate_fingerprint(
        h.db(),
        "peer.test",
        &json!(["lighthouse", "foundfamily"]),
        &json!({"lang": "en"}),
        &json!({"fic_count": 12}),
    )
    .await
    .unwrap();
    assert_eq!(out["instance_host"], "peer.test");
    assert_eq!(out["fingerprint_version"], 1);
    let got = fed::get_fingerprint(h.db(), "peer.test")
        .await
        .unwrap()
        .expect("stored");
    assert_eq!(got.instance_host, "peer.test");
    assert_eq!(
        got.fingerprint_version, 1,
        "decodes as an i64 on both backends"
    );
    assert_eq!(got.theme_vector.as_ref().unwrap()[0], "lighthouse");
    assert_eq!(got.cultural_signals["lang"], "en");
    assert_eq!(got.content_signals["fic_count"], 12);
    assert!(
        got.signature.starts_with("sig_"),
        "the signature is derived from the host"
    );
    assert!(
        got.valid_until > got.created_at,
        "a fresh fingerprint is valid until later than it was made"
    );
}

#[tokio::test]
async fn regenerating_a_fingerprint_fails_instead_of_refreshing_it() {
    // KNOWN DEFECT. The function is named for regenerating a fingerprint and is
    // the only way to refresh one, but `instance_fingerprints.instance_host` is
    // UNIQUE and the INSERT has no ON CONFLICT clause. The second call for a
    // host is a unique violation, so a fingerprint is permanent once written --
    // and since `get_fingerprint` filters on `valid_until > now`, it stops being
    // returned after 30 days and can never be replaced. Nothing in the
    // workspace calls this yet, which is why it has gone unnoticed.
    let h = Db::new("fp-regen").await;
    fed::regenerate_fingerprint(h.db(), "peer.test", &json!([]), &json!({}), &json!({}))
        .await
        .expect("the first fingerprint is written");
    let second =
        fed::regenerate_fingerprint(h.db(), "peer.test", &json!(["new"]), &json!({}), &json!({}))
            .await;
    assert!(
        second.is_err(),
        "regenerating a fingerprint is a unique violation, not a refresh"
    );
    // The original is untouched and still the one served.
    let got = fed::get_fingerprint(h.db(), "peer.test")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        got.theme_vector.as_ref().unwrap().as_array().unwrap().len(),
        0
    );
}

#[tokio::test]
async fn a_fingerprint_that_has_expired_is_not_returned() {
    let h = Db::new("fp-expired").await;
    fed::regenerate_fingerprint(h.db(), "peer.test", &json!([]), &json!({}), &json!({}))
        .await
        .unwrap();
    // Force `valid_until` into the past: the query filters on it, so an expired
    // fingerprint is invisible rather than returned-and-judged-by-the-caller.
    h.exec("UPDATE instance_fingerprints SET valid_until = '2000-01-01T00:00:00+00:00'")
        .await;
    assert!(
        fed::get_fingerprint(h.db(), "peer.test")
            .await
            .unwrap()
            .is_none(),
        "an expired fingerprint is not served"
    );
    assert!(fed::get_fingerprint(h.db(), "unknown.test")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_theme_vector_may_be_absent_for_the_jaccard_fallback() {
    let h = Db::new("fp-null").await;
    fed::regenerate_fingerprint(h.db(), "peer.test", &json!([]), &json!({}), &json!({}))
        .await
        .unwrap();
    h.exec("UPDATE instance_fingerprints SET theme_vector = NULL")
        .await;
    let got = fed::get_fingerprint(h.db(), "peer.test")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        got.theme_vector, None,
        "a null theme vector decodes as None, not an error"
    );
}

impl Db {
    /// Raw SQL escape hatch, for the two places a test needs to age a row.
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
}

// ------------------------------------------------------------------- peers

#[tokio::test]
async fn a_peer_upserts_and_replaces_in_place() {
    let h = Db::new("peer").await;
    fed::upsert_peer(h.db(), "peer.test", 82.5, "friendly", true, Some("admin"))
        .await
        .unwrap();
    let peers = fed::list_all_peers(h.db()).await.unwrap();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].peer_host, "peer.test");
    assert_eq!(
        peers[0].similarity, 82.5,
        "a REAL column round-trips an f64"
    );
    assert_eq!(peers[0].state, "friendly");
    assert!(peers[0].auto_federate);
    assert_eq!(peers[0].set_by.as_deref(), Some("admin"));
    // Upserting the same host updates rather than duplicating.
    fed::upsert_peer(h.db(), "peer.test", 10.0, "muted", false, None)
        .await
        .unwrap();
    let peers = fed::list_all_peers(h.db()).await.unwrap();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].similarity, 10.0);
    assert_eq!(peers[0].state, "muted");
    assert!(!peers[0].auto_federate);
}

#[tokio::test]
async fn similar_peers_are_filtered_by_threshold_and_ordered_by_similarity() {
    let h = Db::new("peer-sim").await;
    for (host, score) in [("low.test", 20.0), ("mid.test", 55.0), ("high.test", 91.0)] {
        fed::upsert_peer(h.db(), host, score, "unknown", false, None)
            .await
            .unwrap();
    }
    let above = fed::list_similar_peers(h.db(), 50.0, 10).await.unwrap();
    assert_eq!(above.len(), 2, "the peer below the threshold is excluded");
    assert_eq!(
        above[0].peer_host, "high.test",
        "ordered by similarity descending"
    );
    assert_eq!(above[1].peer_host, "mid.test");
    // The boundary is inclusive.
    assert_eq!(
        fed::list_similar_peers(h.db(), 20.0, 10)
            .await
            .unwrap()
            .len(),
        3,
        "a peer exactly at the threshold is included"
    );
}

#[tokio::test]
async fn the_similar_peer_list_respects_its_limit() {
    let h = Db::new("peer-limit").await;
    for (host, score) in [("a.test", 90.0), ("b.test", 80.0), ("c.test", 70.0)] {
        fed::upsert_peer(h.db(), host, score, "unknown", false, None)
            .await
            .unwrap();
    }
    let top_two = fed::list_similar_peers(h.db(), 0.0, 2).await.unwrap();
    assert_eq!(top_two.len(), 2);
    assert_eq!(top_two[0].peer_host, "a.test");
    assert_eq!(top_two[1].peer_host, "b.test");
    assert!(
        fed::list_all_peers(h.db()).await.unwrap().len() >= 3,
        "the full list is unaffected by the limit"
    );
}

#[tokio::test]
async fn a_threshold_nobody_meets_returns_nothing() {
    let h = Db::new("peer-none").await;
    fed::upsert_peer(h.db(), "low.test", 5.0, "unknown", false, None)
        .await
        .unwrap();
    assert!(fed::list_similar_peers(h.db(), 99.0, 10)
        .await
        .unwrap()
        .is_empty());
}

// ------------------------------------------------------------------- queue

#[tokio::test]
async fn a_queued_activity_starts_pending_with_no_attempts() {
    let h = Db::new("queue").await;
    let actor = h.actor("a1").await;
    let activity = h.activity(&actor, "one").await;
    let id = fed::enqueue_activity(h.db(), &activity, "https://peer.test/inbox")
        .await
        .unwrap();
    let pending = fed::get_pending_queue(h.db(), 10).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].0, id);
    fed::mark_queue_sent(h.db(), &id).await.unwrap();
    assert!(
        fed::get_pending_queue(h.db(), 10).await.unwrap().is_empty(),
        "a sent activity leaves the pending queue"
    );
}

#[tokio::test]
async fn the_queue_is_ordered_oldest_first_and_respects_its_limit() {
    let h = Db::new("queue-limit").await;
    let actor = h.actor("a1").await;
    for tag in ["one", "two", "three"] {
        let a = h.activity(&actor, tag).await;
        fed::enqueue_activity(h.db(), &a, "https://peer.test/inbox")
            .await
            .unwrap();
    }
    let first_two = fed::get_pending_queue(h.db(), 2).await.unwrap();
    assert_eq!(first_two.len(), 2, "the limit is honoured");
    assert_eq!(fed::get_pending_queue(h.db(), 10).await.unwrap().len(), 3);
}

#[tokio::test]
async fn a_failed_delivery_leaves_the_queue_for_good() {
    // KNOWN DEFECT. `mark_queue_failed` sets status = 'failed' and bumps
    // `attempts`, but the schema has only pending | sent | failed and
    // `get_pending_queue` selects `status = 'pending'`. So a delivery that fails
    // once is never retried, and the `attempts` counter that exists to drive a
    // backoff is written and never read. The index is
    // (status, created_at), which is the shape a retry sweep would want -- the
    // missing piece is a 'retryable' state or a query that includes 'failed'.
    let h = Db::new("queue-fail").await;
    let actor = h.actor("a1").await;
    let activity = h.activity(&actor, "one").await;
    let id = fed::enqueue_activity(h.db(), &activity, "https://peer.test/inbox")
        .await
        .unwrap();
    fed::mark_queue_failed(h.db(), &id).await.unwrap();
    assert!(
        fed::get_pending_queue(h.db(), 10).await.unwrap().is_empty(),
        "a failed item is not pending, so it is never picked up again"
    );
    let q = h.tdb.sql(&format!(
        "SELECT status, attempts FROM federation_queue WHERE id = '{id}'"
    ));
    let (status, attempts): (String, i32) = match h.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_as(&q)
            .fetch_one(h.db().sqlite_pool().expect("sqlite"))
            .await
            .unwrap(),
        lorehaven_db::Backend::Postgres => sqlx::query_as(&q)
            .fetch_one(h.db().postgres_pool().expect("pg"))
            .await
            .unwrap(),
    };
    assert_eq!(status, "failed");
    assert_eq!(
        attempts, 1,
        "the counter is incremented but nothing ever reads it"
    );
    // Failing it again is indistinguishable from a first failure, except that
    // the counter moves.
    fed::mark_queue_failed(h.db(), &id).await.unwrap();
    let attempts_after: i32 = match h.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&h.tdb.sql(&format!(
            "SELECT attempts FROM federation_queue WHERE id = '{id}'"
        )))
        .fetch_one(h.db().sqlite_pool().expect("sqlite"))
        .await
        .unwrap(),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&h.tdb.sql(&format!(
            "SELECT attempts FROM federation_queue WHERE id = '{id}'"
        )))
        .fetch_one(h.db().postgres_pool().expect("pg"))
        .await
        .unwrap(),
    };
    assert_eq!(attempts_after, 2);
}

#[tokio::test]
async fn marking_an_unknown_queue_row_is_a_quiet_no_op() {
    let h = Db::new("queue-missing").await;
    let id = uuid::Uuid::new_v4().to_string();
    fed::mark_queue_sent(h.db(), &id).await.expect("no error");
    fed::mark_queue_failed(h.db(), &id).await.expect("no error");
}

#[tokio::test]
async fn enqueueing_needs_a_real_activity_because_of_the_foreign_key() {
    let h = Db::new("queue-fk").await;
    assert!(
        fed::enqueue_activity(
            h.db(),
            &uuid::Uuid::new_v4().to_string(),
            "https://peer.test/inbox"
        )
        .await
        .is_err(),
        "the activity is a foreign key"
    );
}
