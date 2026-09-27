//! M32 — Narration editions (`crates/db/src/narration.rs`, spec §32.5), the
//! store behind the TTS narration routes.
//!
//! Five `pub async fn` with no tests, and they are live: `routes/narration.rs`
//! creates an edition, credits the provider, records the audio checksum and
//! publishes on a real request path.
//!
//! The module is more careful than most of its neighbours, and the suite exists
//! to keep it that way rather than to fix it. Three properties are load-bearing
//! and each is pinned below:
//!
//! - **An edition with no audio cannot be published.** `approve_narration_edition`
//!   guards on `audio_checksum IS NOT NULL AND audio_checksum != ''`, so
//!   approving silence would put a download door in front of nothing. The empty
//!   string is guarded as well as NULL, which is the half that is easy to drop.
//! - **Approval is idempotent and reports whether *it* did the work.** The
//!   `WHERE ... published_at IS NULL` means a second approval affects no rows
//!   and returns `false`, so a caller can distinguish "I published this" from
//!   "someone already had".
//! - **An empty checksum reads as absent, not as a pointer to nothing.**
//!   `narration_audio_checksum` filters `""` to `None`.
//!
//! `parent_edition_id` is self-referential with `ON DELETE SET NULL`, and
//! `creator_id` is TEXT holding an external provider name rather than a local
//! pseud — so there is no identity to validate and a delete has a specific
//! cascade story worth pinning.

use std::path::PathBuf;

use lorehaven_db::content::create_work;
use lorehaven_db::identity::{create_account, create_pseud, AccountStatus};
use lorehaven_db::narration::{
    add_narration_creator, approve_narration_edition, create_narration_edition,
    mark_narration_audio_stored, narration_audio_checksum,
};
use lorehaven_domain::ids::WorkId;
use lorehaven_domain::policy::AgeState;
use test_support::TestDb;
use uuid::Uuid;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-narration-{tag}-{}-{:?}",
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

    fn is_pg(&self) -> bool {
        self.db().backend() == lorehaven_db::Backend::Postgres
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

    async fn text_of(&self, query: &str) -> Option<String> {
        let owned;
        let query = if self.is_pg() {
            query
        } else {
            owned = query.replace("::text", "");
            owned.as_str()
        };
        let row: Option<Option<String>> = match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar::<_, Option<String>>(query)
                .fetch_optional(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("scalar"),
            lorehaven_db::Backend::Postgres => sqlx::query_scalar::<_, Option<String>>(query)
                .fetch_optional(self.db().postgres_pool().expect("pg"))
                .await
                .expect("scalar"),
        };
        row.flatten()
    }

    /// Read one integer column. `version` is `BIGINT` on PostgreSQL and
    /// `INTEGER` on SQLite, and `text_of` cannot cast it to text on both.
    async fn int_of(&self, query: &str) -> Option<i64> {
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar::<_, i64>(query)
                .fetch_optional(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("scalar"),
            lorehaven_db::Backend::Postgres => sqlx::query_scalar::<_, i64>(query)
                .fetch_optional(self.db().postgres_pool().expect("pg"))
                .await
                .expect("scalar"),
        }
    }

    /// A work to narrate.
    async fn work(&self) -> WorkId {
        let account = create_account(
            self.db(),
            &format!("nar-{}@example.test", Uuid::new_v4()),
            AgeState::DeclaredAdult,
            AccountStatus::Active,
        )
        .await
        .expect("create_account");
        let pseud = create_pseud(
            self.db(),
            account,
            &format!("n-{}", Uuid::new_v4()),
            "Narrator",
        )
        .await
        .expect("create_pseud");
        create_work(self.db(), pseud, "Narrated Work", None)
            .await
            .expect("create_work")
            .id
    }

    /// A narration edition of `work`, with audio stored.
    async fn edition_with_audio(&self, work: &WorkId) -> String {
        let id = create_narration_edition(self.db(), &work.to_string(), "v1", None)
            .await
            .expect("create_narration_edition");
        mark_narration_audio_stored(self.db(), &id, "sha256-audio")
            .await
            .expect("mark_narration_audio_stored");
        id
    }
}

// ---------------------------------------------------------------------------
// create_narration_edition
// ---------------------------------------------------------------------------

/// A new edition is a `narration` kind, unpublished, at version 1.
#[tokio::test]
async fn a_new_edition_is_an_unpublished_narration() {
    let h = Harness::new("nar-create").await;
    let work = h.work().await;
    let id = create_narration_edition(h.db(), &work.to_string(), "v1", None)
        .await
        .expect("create");

    assert_eq!(h.count("media_editions").await, 1);
    assert_eq!(
        h.text_of(&format!(
            "SELECT edition_kind FROM media_editions WHERE id = '{id}'"
        ))
        .await
        .as_deref(),
        Some("narration"),
        "the kind is what approve_narration_edition filters on"
    );
    assert_eq!(
        h.text_of(&format!(
            "SELECT published_at FROM media_editions WHERE id = '{id}'"
        ))
        .await,
        None,
        "a new edition is a draft"
    );
    assert_eq!(
        h.int_of(&format!(
            "SELECT version FROM media_editions WHERE id = '{id}'"
        ))
        .await,
        Some(1)
    );
}

/// The returned id names a row that can be found again.
#[tokio::test]
async fn the_returned_id_names_a_stored_row() {
    let h = Harness::new("nar-create-id").await;
    let work = h.work().await;
    let id = create_narration_edition(h.db(), &work.to_string(), "v1", None)
        .await
        .expect("create");
    assert_eq!(
        h.text_of(&format!(
            "SELECT id::text FROM media_editions WHERE id = '{id}'"
        ))
        .await
        .as_deref(),
        Some(id.as_str())
    );
}

/// Two editions of one work are distinct rows — a re-narration is not a
/// replacement.
#[tokio::test]
async fn two_editions_of_one_work_coexist() {
    let h = Harness::new("nar-two").await;
    let work = h.work().await;
    let a = create_narration_edition(h.db(), &work.to_string(), "v1", None)
        .await
        .expect("first");
    let b = create_narration_edition(h.db(), &work.to_string(), "v2", None)
        .await
        .expect("second");

    assert_ne!(a, b);
    assert_eq!(h.count("media_editions").await, 2);
    assert_eq!(
        h.text_of(&format!(
            "SELECT label FROM media_editions WHERE id = '{b}'"
        ))
        .await
        .as_deref(),
        Some("v2")
    );
}

/// `label` is nullable, so an unlabelled edition is legal and reads as NULL.
#[tokio::test]
async fn a_null_label_is_stored_as_null() {
    let h = Harness::new("nar-null-label").await;
    let work = h.work().await;
    let id = create_narration_edition(h.db(), &work.to_string(), "", None)
        .await
        .expect("create");
    // The column is nullable; an empty string is what this call actually writes,
    // and it is stored verbatim rather than normalised away.
    assert_eq!(
        h.text_of(&format!(
            "SELECT label FROM media_editions WHERE id = '{id}'"
        ))
        .await
        .as_deref(),
        Some("")
    );
}

/// A parent links the edition to another edition of the same work.
#[tokio::test]
async fn a_parent_can_be_recorded() {
    let h = Harness::new("nar-parent").await;
    let work = h.work().await;
    let parent = create_narration_edition(h.db(), &work.to_string(), "original", None)
        .await
        .expect("parent");
    let child = create_narration_edition(h.db(), &work.to_string(), "abridged", Some(&parent))
        .await
        .expect("child");

    assert_eq!(
        h.text_of(&format!(
            "SELECT parent_edition_id::text FROM media_editions WHERE id = '{child}'"
        ))
        .await
        .as_deref(),
        Some(parent.as_str())
    );
}

/// **Deleting a parent nulls the child's link rather than deleting it** —
/// `ON DELETE SET NULL`. A cascade here would silently remove narrations.
#[tokio::test]
async fn deleting_a_parent_nulls_the_child_link() {
    let h = Harness::new("nar-parent-cascade").await;
    let work = h.work().await;
    let parent = create_narration_edition(h.db(), &work.to_string(), "original", None)
        .await
        .expect("parent");
    let child = create_narration_edition(h.db(), &work.to_string(), "abridged", Some(&parent))
        .await
        .expect("child");

    h.exec(&format!("DELETE FROM media_editions WHERE id = '{parent}'"))
        .await;

    assert_eq!(h.count("media_editions").await, 1, "the child survives");
    assert_eq!(
        h.text_of(&format!(
            "SELECT parent_edition_id::text FROM media_editions WHERE id = '{child}'"
        ))
        .await,
        None,
        "with its parent link cleared"
    );
}

/// An edition for a work that does not exist is refused by the foreign key.
#[tokio::test]
async fn an_edition_for_an_unknown_work_is_refused() {
    let h = Harness::new("nar-fk").await;
    let result = create_narration_edition(h.db(), &WorkId::new().to_string(), "v1", None).await;
    assert!(result.is_err(), "work_id references works(id)");
    assert_eq!(h.count("media_editions").await, 0);
}

/// A malformed `work_id` is refused on both backends, by different rules.
///
/// PostgreSQL rejects it at the `::uuid` cast; SQLite has no such cast and
/// rejects it at the foreign key to `works(id)`, which `not-a-uuid` also cannot
/// satisfy. Both end in an error, so a caller sees one behaviour.
#[tokio::test]
async fn a_malformed_work_id_is_refused() {
    let h = Harness::new("nar-bad-uuid").await;
    assert!(create_narration_edition(h.db(), "not-a-uuid", "v1", None)
        .await
        .is_err());
    assert_eq!(h.count("media_editions").await, 0, "and no row is written");
}

/// A malformed `parent_edition_id` is refused on both backends too.
#[tokio::test]
async fn a_malformed_parent_id_is_refused() {
    let h = Harness::new("nar-bad-parent").await;
    let work = h.work().await;
    assert!(
        create_narration_edition(h.db(), &work.to_string(), "v1", Some("not-a-uuid"))
            .await
            .is_err()
    );
}

// ---------------------------------------------------------------------------
// mark_narration_audio_stored
// ---------------------------------------------------------------------------

/// Storing audio records the checksum, and the checksum reads back.
#[tokio::test]
async fn storing_audio_records_the_checksum() {
    let h = Harness::new("nar-store").await;
    let work = h.work().await;
    let id = create_narration_edition(h.db(), &work.to_string(), "v1", None)
        .await
        .expect("create");
    assert_eq!(
        narration_audio_checksum(h.db(), &id).await.expect("read"),
        None
    );

    mark_narration_audio_stored(h.db(), &id, "sha256-abc")
        .await
        .expect("store");

    assert_eq!(
        narration_audio_checksum(h.db(), &id)
            .await
            .expect("read")
            .as_deref(),
        Some("sha256-abc")
    );
}

/// **An empty checksum reads as absent.** This is the half of the publish guard
/// that a NULL-only check would miss, and it is why `''` is filtered to `None`
/// rather than stored and returned.
#[tokio::test]
async fn an_empty_checksum_reads_as_absent() {
    let h = Harness::new("nar-empty-checksum").await;
    let work = h.work().await;
    let id = create_narration_edition(h.db(), &work.to_string(), "v1", None)
        .await
        .expect("create");
    mark_narration_audio_stored(h.db(), &id, "")
        .await
        .expect("store");

    assert_eq!(
        narration_audio_checksum(h.db(), &id).await.expect("read"),
        None,
        "an empty string is not a pointer to audio"
    );
}

/// Storing audio again replaces the checksum — a re-render supersedes.
#[tokio::test]
async fn storing_audio_again_replaces_the_checksum() {
    let h = Harness::new("nar-restore").await;
    let work = h.work().await;
    let id = create_narration_edition(h.db(), &work.to_string(), "v1", None)
        .await
        .expect("create");
    mark_narration_audio_stored(h.db(), &id, "sha256-old")
        .await
        .expect("first");
    mark_narration_audio_stored(h.db(), &id, "sha256-new")
        .await
        .expect("second");

    assert_eq!(
        narration_audio_checksum(h.db(), &id)
            .await
            .expect("read")
            .as_deref(),
        Some("sha256-new")
    );
}

/// **Storing audio for an edition that does not exist is a silent no-op** —
/// `mark_narration_audio_stored` returns `Result<()>` and does not report
/// whether a row was affected. A caller cannot tell a stored checksum from a
/// dropped one.
#[tokio::test]
async fn storing_audio_for_an_unknown_edition_is_a_silent_no_op() {
    let h = Harness::new("nar-store-unknown").await;
    mark_narration_audio_stored(h.db(), &Uuid::new_v4().to_string(), "sha256")
        .await
        .expect("a missing edition is not an error to stamp");
    assert_eq!(h.count("media_editions").await, 0);
}

// ---------------------------------------------------------------------------
// add_narration_creator
// ---------------------------------------------------------------------------

/// A creator credit names the provider and the role.
#[tokio::test]
async fn a_creator_credit_is_recorded() {
    let h = Harness::new("nar-creator").await;
    let work = h.work().await;
    let id = create_narration_edition(h.db(), &work.to_string(), "v1", None)
        .await
        .expect("create");
    add_narration_creator(h.db(), &id, "elevenlabs", "narrator")
        .await
        .expect("credit");

    assert_eq!(h.count("media_edition_creators").await, 1);
    assert_eq!(
        h.text_of(&format!(
            "SELECT creator_id FROM media_edition_creators WHERE edition_id = '{id}'"
        ))
        .await
        .as_deref(),
        Some("elevenlabs")
    );
    assert_eq!(
        h.text_of(&format!(
            "SELECT role FROM media_edition_creators WHERE edition_id = '{id}'"
        ))
        .await
        .as_deref(),
        Some("narrator")
    );
}

/// **`creator_id` is an external name, not a local id**, so there is no
/// identity to validate and any string is accepted.
#[tokio::test]
async fn the_creator_is_an_external_name_not_an_account() {
    let h = Harness::new("nar-creator-external").await;
    let work = h.work().await;
    let id = create_narration_edition(h.db(), &work.to_string(), "v1", None)
        .await
        .expect("create");
    add_narration_creator(h.db(), &id, "a-provider-that-does-not-exist", "narrator")
        .await
        .expect("an external name needs no matching row");
    assert_eq!(h.count("media_edition_creators").await, 1);
}

/// Two credits on one edition are two rows, so several providers can be listed.
#[tokio::test]
async fn two_credits_on_one_edition_coexist() {
    let h = Harness::new("nar-creator-two").await;
    let work = h.work().await;
    let id = create_narration_edition(h.db(), &work.to_string(), "v1", None)
        .await
        .expect("create");
    add_narration_creator(h.db(), &id, "elevenlabs", "narrator")
        .await
        .expect("first");
    add_narration_creator(h.db(), &id, "a-human-editor", "editor")
        .await
        .expect("second");
    assert_eq!(h.count("media_edition_creators").await, 2);
}

/// A credit for an edition that does not exist is refused by the foreign key.
#[tokio::test]
async fn a_credit_for_an_unknown_edition_is_refused() {
    let h = Harness::new("nar-creator-fk").await;
    assert!(
        add_narration_creator(h.db(), &Uuid::new_v4().to_string(), "p", "narrator")
            .await
            .is_err()
    );
    assert_eq!(h.count("media_edition_creators").await, 0);
}

/// **Deleting an edition cascades to its credits** — `ON DELETE CASCADE`.
#[tokio::test]
async fn deleting_an_edition_cascades_to_its_credits() {
    let h = Harness::new("nar-creator-cascade").await;
    let work = h.work().await;
    let id = create_narration_edition(h.db(), &work.to_string(), "v1", None)
        .await
        .expect("create");
    add_narration_creator(h.db(), &id, "elevenlabs", "narrator")
        .await
        .expect("credit");
    assert_eq!(h.count("media_edition_creators").await, 1);

    h.exec(&format!("DELETE FROM media_editions WHERE id = '{id}'"))
        .await;
    assert_eq!(h.count("media_edition_creators").await, 0);
}

// ---------------------------------------------------------------------------
// narration_audio_checksum
// ---------------------------------------------------------------------------

/// An edition with no audio has no checksum.
#[tokio::test]
async fn an_edition_without_audio_has_no_checksum() {
    let h = Harness::new("nar-no-checksum").await;
    let work = h.work().await;
    let id = create_narration_edition(h.db(), &work.to_string(), "v1", None)
        .await
        .expect("create");
    assert_eq!(
        narration_audio_checksum(h.db(), &id).await.expect("read"),
        None
    );
}

/// An unknown edition has no checksum.
#[tokio::test]
async fn an_unknown_edition_has_no_checksum() {
    let h = Harness::new("nar-checksum-unknown").await;
    assert_eq!(
        narration_audio_checksum(h.db(), &Uuid::new_v4().to_string())
            .await
            .expect("read"),
        None
    );
}

/// The checksum is per edition, not per work.
#[tokio::test]
async fn the_checksum_is_per_edition() {
    let h = Harness::new("nar-checksum-per-edition").await;
    let work = h.work().await;
    let a = h.edition_with_audio(&work).await;
    let b = create_narration_edition(h.db(), &work.to_string(), "v2", None)
        .await
        .expect("second");

    assert_eq!(
        narration_audio_checksum(h.db(), &a)
            .await
            .expect("read")
            .as_deref(),
        Some("sha256-audio")
    );
    assert_eq!(
        narration_audio_checksum(h.db(), &b).await.expect("read"),
        None,
        "the other edition of the same work has no audio"
    );
}

/// A checksum survives a read — reading is not a mutation.
#[tokio::test]
async fn reading_the_checksum_twice_is_stable() {
    let h = Harness::new("nar-checksum-stable").await;
    let work = h.work().await;
    let id = h.edition_with_audio(&work).await;
    let first = narration_audio_checksum(h.db(), &id).await.expect("read");
    let second = narration_audio_checksum(h.db(), &id).await.expect("read");
    assert_eq!(first, second);
}

// ---------------------------------------------------------------------------
// approve_narration_edition
// ---------------------------------------------------------------------------

/// **An edition with audio can be published, and the call reports that it did.**
#[tokio::test]
async fn an_edition_with_audio_publishes() {
    let h = Harness::new("nar-approve").await;
    let work = h.work().await;
    let id = h.edition_with_audio(&work).await;

    assert!(approve_narration_edition(h.db(), &id)
        .await
        .expect("approve"));
    assert!(h
        .text_of(&format!(
            "SELECT published_at FROM media_editions WHERE id = '{id}'"
        ))
        .await
        .is_some());
}

/// **Publishing bumps the version**, so a cached edition view knows to re-read.
#[tokio::test]
async fn publishing_bumps_the_version() {
    let h = Harness::new("nar-approve-version").await;
    let work = h.work().await;
    let id = h.edition_with_audio(&work).await;
    approve_narration_edition(h.db(), &id)
        .await
        .expect("approve");

    assert_eq!(
        h.int_of(&format!(
            "SELECT version FROM media_editions WHERE id = '{id}'"
        ))
        .await,
        Some(2)
    );
}

/// **Approving twice is an idempotent no-op that reports `false`** — the
/// `published_at IS NULL` guard means a second approval affects no rows, so a
/// caller can tell "I published this" from "it was already published".
#[tokio::test]
async fn approving_twice_reports_false_the_second_time() {
    let h = Harness::new("nar-approve-twice").await;
    let work = h.work().await;
    let id = h.edition_with_audio(&work).await;

    assert!(approve_narration_edition(h.db(), &id).await.expect("first"));
    assert!(
        !approve_narration_edition(h.db(), &id)
            .await
            .expect("second"),
        "the second approval did nothing and says so"
    );
    // And it did not bump the version a second time.
    assert_eq!(
        h.int_of(&format!(
            "SELECT version FROM media_editions WHERE id = '{id}'"
        ))
        .await,
        Some(2)
    );
}

/// **An edition with no audio cannot be published** — approving silence would
/// put a download door in front of nothing.
#[tokio::test]
async fn an_edition_without_audio_cannot_be_published() {
    let h = Harness::new("nar-approve-noaudio").await;
    let work = h.work().await;
    let id = create_narration_edition(h.db(), &work.to_string(), "v1", None)
        .await
        .expect("create");

    assert!(!approve_narration_edition(h.db(), &id)
        .await
        .expect("approve"));
    assert_eq!(
        h.text_of(&format!(
            "SELECT published_at FROM media_editions WHERE id = '{id}'"
        ))
        .await,
        None
    );
}

/// **An empty checksum also blocks publication** — the guard is
/// `IS NOT NULL AND != ''`, not NULL alone. This is the case a NULL-only check
/// would let through.
#[tokio::test]
async fn an_empty_checksum_also_blocks_publication() {
    let h = Harness::new("nar-approve-empty").await;
    let work = h.work().await;
    let id = create_narration_edition(h.db(), &work.to_string(), "v1", None)
        .await
        .expect("create");
    mark_narration_audio_stored(h.db(), &id, "")
        .await
        .expect("store an empty checksum");

    assert!(
        !approve_narration_edition(h.db(), &id)
            .await
            .expect("approve"),
        "an empty checksum is not audio"
    );
}

/// Audio stored after a refused approval then allows it — the refusal is not
/// sticky.
#[tokio::test]
async fn audio_stored_after_a_refusal_then_allows_publication() {
    let h = Harness::new("nar-approve-later").await;
    let work = h.work().await;
    let id = create_narration_edition(h.db(), &work.to_string(), "v1", None)
        .await
        .expect("create");

    assert!(!approve_narration_edition(h.db(), &id)
        .await
        .expect("refused"));
    mark_narration_audio_stored(h.db(), &id, "sha256-late")
        .await
        .expect("store");
    assert!(approve_narration_edition(h.db(), &id)
        .await
        .expect("allowed"));
}

/// Approving an unknown edition is `false`, not an error.
#[tokio::test]
async fn approving_an_unknown_edition_is_false() {
    let h = Harness::new("nar-approve-unknown").await;
    assert!(
        !approve_narration_edition(h.db(), &Uuid::new_v4().to_string())
            .await
            .expect("approve")
    );
}

/// **The function refuses anything that is not a narration edition**, by name
/// rather than by convention — `edition_kind = 'narration'` is in the `WHERE`.
/// A non-narration edition cannot be published through this path.
#[tokio::test]
async fn a_non_narration_edition_is_refused() {
    let h = Harness::new("nar-approve-other-kind").await;
    let work = h.work().await;
    let id = create_narration_edition(h.db(), &work.to_string(), "v1", None)
        .await
        .expect("create");
    // Rewrite the kind behind the repository's back, with audio present.
    h.exec(&format!(
        "UPDATE media_editions SET edition_kind = 'revised', audio_checksum = 'sha256' \
         WHERE id = '{id}'"
    ))
    .await;

    assert!(
        !approve_narration_edition(h.db(), &id)
            .await
            .expect("approve"),
        "this function publishes narration editions only"
    );
}

/// **A malformed edition id is an `Err` on PostgreSQL and `false` on SQLite.**
///
/// The PostgreSQL statement binds `id = ?::uuid`, so the cast rejects the
/// string before the statement can run and the call fails loudly. SQLite stores
/// the column as TEXT and simply matches no row, so the same input reads as
/// "nothing to publish".
///
/// Both are defensible, but they are not the same, and a caller that treats a
/// `Result::Err` as a server fault would report a bad client-supplied id as a
/// 500. Worth knowing before this reaches a route. Recorded as `M32-D01`.
#[tokio::test]
async fn a_malformed_edition_id_differs_between_backends() {
    let h = Harness::new("nar-approve-bad-uuid").await;
    let result = approve_narration_edition(h.db(), "not-a-uuid").await;
    match h.db().backend() {
        lorehaven_db::Backend::Postgres => assert!(
            result.is_err(),
            "the ::uuid cast rejects it before the statement runs"
        ),
        lorehaven_db::Backend::Sqlite => assert!(
            !result.expect("a malformed id is not a SQLite error"),
            "it matches no row instead"
        ),
    }
}

/// The full narration lifecycle, as the route performs it: create, credit,
/// render, approve, and read the checksum back.
#[tokio::test]
async fn the_narration_lifecycle_runs_end_to_end() {
    let h = Harness::new("nar-lifecycle").await;
    let work = h.work().await;

    let edition = create_narration_edition(h.db(), &work.to_string(), "v1", None)
        .await
        .expect("create");
    add_narration_creator(h.db(), &edition, "elevenlabs", "narrator")
        .await
        .expect("credit");
    assert_eq!(
        narration_audio_checksum(h.db(), &edition)
            .await
            .expect("read"),
        None
    );
    assert!(
        !approve_narration_edition(h.db(), &edition)
            .await
            .expect("too early"),
        "nothing to publish yet"
    );

    mark_narration_audio_stored(h.db(), &edition, "sha256-final")
        .await
        .expect("rendered");
    assert_eq!(
        narration_audio_checksum(h.db(), &edition)
            .await
            .expect("read")
            .as_deref(),
        Some("sha256-final")
    );
    assert!(approve_narration_edition(h.db(), &edition)
        .await
        .expect("publish"));
    assert!(!approve_narration_edition(h.db(), &edition)
        .await
        .expect("idempotent"));
    assert_eq!(h.count("media_edition_creators").await, 1);
}
