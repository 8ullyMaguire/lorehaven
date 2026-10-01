//! Acceptance: curation must reach the vocabulary search reads, atomically.
//!
//! Migration 0082's own comment states the rule this file enforces:
//!
//! > Two tables describing one state must not spell it two ways, or a query
//! > joining them silently drops every curated row.
//!
//! §15.17 keeps two such tables: `taxonomy_nodes` (what a tag browser reads, what
//! `search_nodes` searches, what `work_tags` references) and `canonical_entities`
//! (the exchange's view, carrying the curator's account and timestamps). A curated
//! name has to agree across both.
//!
//! **What was actually wrong, and what was not.** `POST /exchange/entities/curate`
//! did call both writers, so curation was not invisible — an earlier diagnosis of
//! mine claimed otherwise and was wrong; the tests below were written to disprove it
//! and the route is what disproved it. The real defect is narrower and survives that
//! correction:
//!
//! 1. **The pairing lived in a route handler.** `curate_node` and `curate_entity`
//!    are both `pub`, so a job, an import or an admin path can curate one table and
//!    silently miss the other. Nothing but that one handler knew they must be
//!    written together.
//! 2. **The two writes were not atomic.** The first committed before the second
//!    began, so a failure between them left a name curated in one vocabulary and
//!    unverified in the other — exactly the state 0082 says a join drops.
//!
//! The fix is `taxonomy::curate_name`: one db-layer call, one transaction, both
//! tables. The tests pin the end state rather than the mechanism, so they hold
//! whichever writer is used.

use lorehaven_db::{exchange, taxonomy};
use test_support::{scratch_dir, TestDb};

async fn scratch(tag: &str) -> TestDb {
    let dir = scratch_dir(tag);
    TestDb::connect_with_dir(tag, &dir).await
}

/// A signal-created name, as §15.17 describes it: usable, and visibly not curated.
async fn signal_created_node(db: &TestDb) -> (String, String) {
    let node = taxonomy::ensure_node_from_signal(db.db(), "character", "Alice")
        .await
        .expect("a signal may create a usable node (§15.17)");
    // A real signal populates BOTH vocabularies: `record_entity_signal` writes
    // `canonical_entities` and `ensure_node_from_signal` writes `taxonomy_nodes`.
    // Reproduced here so the curation path has a row to update in each — without
    // the second table the "both tables agree" test would pass vacuously.
    let hash = test_support::id("hash");
    exchange::store_signal(
        db.db(),
        &hash,
        None,
        "sender-1",
        Some("peer-instance"),
        "{}",
    )
    .await
    .expect("the signal row exists, which exchange_signal_entities references");
    exchange::reinforce_entities(
        db.db(),
        &hash,
        &[(
            "character".to_string(),
            "Alice".to_string(),
            lorehaven_domain::exchange::normalise("Alice"),
        )],
    )
    .await
    .expect("the same signal records the entity in the exchange's vocabulary");
    (node.id, node.norm)
}

#[tokio::test]
async fn curating_a_name_updates_the_table_search_reads() {
    let db = scratch("curate_reaches_taxonomy_nodes").await;

    let (node_id, norm) = signal_created_node(&db).await;
    let before = taxonomy::node_by_id(db.db(), &node_id)
        .await
        .expect("node lookup runs")
        .expect("the node exists in taxonomy_nodes");
    assert_eq!(
        before.review_status, "unverified",
        "a node created from a signal is usable but never presented as curated \
         (§15.17's second acceptance line)"
    );

    let affected =
        taxonomy::curate_name(db.db(), "character", &norm, "Alice (Curated)", "curator-1")
            .await
            .expect("curation runs");
    assert!(
        affected,
        "the name was in the review queue, so a row was updated"
    );

    let after = taxonomy::node_by_id(db.db(), &node_id)
        .await
        .expect("node lookup runs")
        .expect("the node still exists");
    assert_eq!(
        after.review_status, "curated",
        "curation must be visible in taxonomy_nodes: that is the table a tag \
         browser reads and search_nodes searches, so this is the assertion that \
         distinguishes 'the curator's action was recorded' from 'the curator's \
         action took effect'"
    );
    assert_eq!(
        after.canonical, "Alice (Curated)",
        "the curator's chosen display form must be the one readers see"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn the_two_tables_agree_about_one_name() {
    let db = scratch("curate_tables_agree").await;

    let (node_id, norm) = signal_created_node(&db).await;
    taxonomy::curate_name(db.db(), "character", &norm, "Alice (Curated)", "curator-1")
        .await
        .expect("curation runs");

    // The shape 0082 warned about: a query joining the two tables must not drop
    // rows. If curation still wrote only one of them, this returns zero and the
    // name reads as unverified everywhere at once.
    let sql = db.db().sql(
        "SELECT COUNT(*) FROM taxonomy_nodes t
         JOIN canonical_entities c ON c.kind = t.kind AND c.norm = t.norm
         WHERE t.id = ? AND t.review_status = c.review_status",
        "SELECT COUNT(*) FROM taxonomy_nodes t
         JOIN canonical_entities c ON c.kind = t.kind AND c.norm = t.norm
         WHERE t.id = $1 AND t.review_status = c.review_status",
    );
    let count: i64 = match db.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&sql)
            .bind(&node_id)
            .fetch_one(db.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("count on sqlite"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&sql)
            .bind(&node_id)
            .fetch_one(db.db().postgres_pool().expect("postgres"))
            .await
            .expect("count on postgres"),
    };
    assert_eq!(
        count, 1,
        "a curated name must agree across both vocabularies, or a join drops it \
         (0082)"
    );

    // Also assert the exchange's own column directly rather than only via the
    // join. The join is the load-bearing assertion — a mutant that leaves
    // `canonical_entities` unwritten makes it return 0 — but it is silent about
    // *which* vocabulary went wrong, and a defect that curated both tables with
    // the wrong canonical form would leave it at 1. Naming the column says it in
    // the failure message.
    let status_sql = db.db().sql(
        "SELECT review_status FROM canonical_entities WHERE kind = ? AND norm = ?",
        "SELECT review_status FROM canonical_entities WHERE kind = $1 AND norm = $2",
    );
    let status: String = match db.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_scalar(&status_sql)
            .bind("character")
            .bind(&norm)
            .fetch_one(db.db().sqlite_pool().expect("sqlite"))
            .await
            .expect("the exchange vocabulary has a row for this name"),
        lorehaven_db::Backend::Postgres => sqlx::query_scalar(&status_sql)
            .bind("character")
            .bind(&norm)
            .fetch_one(db.db().postgres_pool().expect("postgres"))
            .await
            .expect("the exchange vocabulary has a row for this name"),
    };
    assert_eq!(
        status, "curated",
        "curation must land in canonical_entities too: it is the exchange's own \
         view of the same state, and 0082 requires the two to agree"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn reinforcing_a_node_does_not_undo_curation() {
    let db = scratch("curate_survives_reinforcement").await;

    let (node_id, norm) = signal_created_node(&db).await;
    taxonomy::curate_name(db.db(), "character", &norm, "Alice (Curated)", "curator-1")
        .await
        .expect("curation runs");

    // A second signal for the same name. §15.17: reinforcing a node must not
    // rewrite a curator's chosen display form, and agreement is not curation.
    taxonomy::ensure_node_from_signal(db.db(), "character", "Alice")
        .await
        .expect("reinforcement runs");

    let after = taxonomy::node_by_id(db.db(), &node_id)
        .await
        .expect("node lookup runs")
        .expect("the node still exists");
    assert_eq!(
        after.review_status, "curated",
        "a later signal must not demote a curated name back to unverified"
    );
    assert_eq!(
        after.canonical, "Alice (Curated)",
        "a later signal must not rewrite the curator's display form"
    );

    db.cleanup().await;
}
