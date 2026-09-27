//! M10 — Encrypted secrets (`crates/db/src/secrets.rs`, spec §10), the store
//! behind import-source credentials.
//!
//! Six `pub async fn` and **no tests at all** — neither in-module nor in an
//! integration suite. That matters more here than the raw count: `secrets` holds
//! encrypted OAuth tokens, and `crates/app/src/routes/imports.rs` reads and
//! writes them on a live request path.
//!
//! The schema is deliberately kind to the repository layer. `migrations/*/0005`
//! stores timestamps as RFC 3339 **TEXT** on both engines and identifiers as
//! canonical UUID text (ADR 0004), so unlike `rating_anomaly_events` there is no
//! native-type decode to get wrong here — `created_at` really is a `String`
//! column in PostgreSQL. What the suite checks instead is the behaviour the
//! callers depend on, and what a plausible edit could silently break:
//!
//! - **A write replaces by `(owner_type, owner_id, name)` and keeps the id.**
//!   `imports.rs` stores a `previous` id and deletes it after a re-seal; if a
//!   replacement reissued the id, that delete would remove the row it had just
//!   been superseded by. The id must be stable across a rewrite.
//! - **The version increments on replacement.** A caller caching ciphertext can
//!   tell it is stale from the version alone.
//! - **A key must be registered before it can encrypt.** `secrets.key_id`
//   references `encryption_keys`, so an unregistered key is refused by the
//!   database rather than by a convention.
//! - **Retiring a key keeps the rows.** A rotation must not make existing
//!   ciphertext unreadable.

use std::path::PathBuf;

use lorehaven_db::secrets::{
    delete_secret, ensure_encryption_key, get_secret, get_secret_by_id, put_secret,
    retire_encryption_key,
};
use test_support::TestDb;
use uuid::Uuid;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-secrets-{tag}-{}-{:?}",
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

    /// Register a key so a secret can reference it.
    async fn key(&self, key_id: &str) {
        ensure_encryption_key(self.db(), key_id, "xchacha20poly1305")
            .await
            .expect("ensure_encryption_key");
    }

    /// Write a secret, registering `key_id` first.
    async fn put(
        &self,
        owner_type: &str,
        owner_id: &str,
        name: &str,
        key_id: &str,
        ciphertext: &str,
    ) -> String {
        self.key(key_id).await;
        put_secret(
            self.db(),
            owner_type,
            owner_id,
            name,
            key_id,
            "nonce-aabb",
            ciphertext,
        )
        .await
        .expect("put_secret")
    }
}

// ---------------------------------------------------------------------------
// ensure_encryption_key
// ---------------------------------------------------------------------------

/// Registering a key records it with its algorithm and no retirement.
#[tokio::test]
async fn registering_a_key_records_it_unretired() {
    let h = Harness::new("sec-key").await;
    h.key("k1").await;

    assert_eq!(h.count("encryption_keys").await, 1);
    assert_eq!(
        h.text_of("SELECT algorithm::text FROM encryption_keys WHERE key_id = 'k1'")
            .await
            .as_deref(),
        Some("xchacha20poly1305")
    );
    assert!(
        h.text_of("SELECT retired_at FROM encryption_keys WHERE key_id = 'k1'")
            .await
            .is_none(),
        "a fresh key is not retired"
    );
}

/// **Registering the same key twice is a no-op, including `created_at`.** The
/// caller invokes this on every seal, so a re-registration that rewrote the
/// timestamp would falsify the key's age on the next secret written.
#[tokio::test]
async fn registering_a_key_twice_changes_nothing() {
    let h = Harness::new("sec-key-twice").await;
    h.key("k1").await;
    let first_created = h
        .text_of("SELECT created_at FROM encryption_keys WHERE key_id = 'k1'")
        .await
        .expect("created_at");

    // A different algorithm must not be adopted either — the first write wins.
    ensure_encryption_key(h.db(), "k1", "aes-256-gcm")
        .await
        .expect("second registration is a no-op");

    assert_eq!(h.count("encryption_keys").await, 1, "still one key");
    assert_eq!(
        h.text_of("SELECT created_at FROM encryption_keys WHERE key_id = 'k1'")
            .await
            .as_deref(),
        Some(first_created.as_str()),
        "created_at is not rewritten"
    );
    assert_eq!(
        h.text_of("SELECT algorithm FROM encryption_keys WHERE key_id = 'k1'")
            .await
            .as_deref(),
        Some("xchacha20poly1305"),
        "and the first algorithm stands"
    );
}

#[tokio::test]
async fn registering_a_retired_key_does_not_unretire_it() {
    let h = Harness::new("sec-key-retired").await;
    h.key("k1").await;
    retire_encryption_key(h.db(), "k1").await.expect("retire");

    h.key("k1").await;
    assert!(
        h.text_of("SELECT retired_at FROM encryption_keys WHERE key_id = 'k1'")
            .await
            .is_some(),
        "re-registering must not bring a retired key back"
    );
}

/// Distinct key ids are distinct rows.
#[tokio::test]
async fn distinct_keys_are_distinct_rows() {
    let h = Harness::new("sec-key-distinct").await;
    h.key("k1").await;
    h.key("k2").await;
    assert_eq!(h.count("encryption_keys").await, 2);
}

// ---------------------------------------------------------------------------
// retire_encryption_key
// ---------------------------------------------------------------------------

/// Retiring stamps the key and keeps it listed — a rotation must still be able
/// to say which key encrypted which row.
#[tokio::test]
async fn retiring_a_key_stamps_it_and_keeps_the_row() {
    let h = Harness::new("sec-retire").await;
    h.key("k1").await;
    retire_encryption_key(h.db(), "k1").await.expect("retire");

    assert_eq!(
        h.count("encryption_keys").await,
        1,
        "a retired key stays listed"
    );
    assert!(h
        .text_of("SELECT retired_at FROM encryption_keys WHERE key_id = 'k1'")
        .await
        .is_some());
}

/// **Retiring a key does not delete the secrets encrypted with it.** This is
/// the whole point of a retired key, and it is the failure a cascade here would
/// cause silently.
#[tokio::test]
async fn retiring_a_key_leaves_its_secrets_readable() {
    let h = Harness::new("sec-retire-keeps").await;
    h.put("import_source", "src-1", "token", "k1", "ct-original")
        .await;
    retire_encryption_key(h.db(), "k1").await.expect("retire");

    let row = get_secret(h.db(), "import_source", "src-1", "token")
        .await
        .expect("read")
        .expect("still there");
    assert_eq!(row.ciphertext, "ct-original", "still readable");
    assert_eq!(row.key_id, "k1");
}

/// Retiring a key that was never registered is a silent no-op.
#[tokio::test]
async fn retiring_an_unknown_key_is_a_no_op() {
    let h = Harness::new("sec-retire-unknown").await;
    retire_encryption_key(h.db(), "nope")
        .await
        .expect("a missing key is not an error to retire");
    assert_eq!(h.count("encryption_keys").await, 0);
}

/// **Retiring twice re-stamps the key.** The `UPDATE` has no
/// `AND retired_at IS NULL`, so the second retirement overwrites the first
/// time.
///
/// That is a smaller problem than the equivalent in `rating_integrity` — a key
/// has one retirement and the second call is almost certainly the same operator
/// repeating themselves — but it means the recorded time is the *last* attempt
/// rather than the first, and nothing warns that the key was already retired.
/// Recorded as `M10-D01` in `docs/known-gaps.md`.
#[tokio::test]
async fn retiring_twice_re_stamps_the_key() {
    let h = Harness::new("sec-retire-twice").await;
    h.key("k1").await;
    retire_encryption_key(h.db(), "k1").await.expect("first");
    let first = h
        .text_of("SELECT retired_at FROM encryption_keys WHERE key_id = 'k1'")
        .await
        .expect("stamped");
    retire_encryption_key(h.db(), "k1").await.expect("second");
    let second = h
        .text_of("SELECT retired_at FROM encryption_keys WHERE key_id = 'k1'")
        .await
        .expect("stamped again");

    assert_ne!(
        second, first,
        "DIAG: the second retirement overwrites the first stamp"
    );
}

// ---------------------------------------------------------------------------
// put_secret
// ---------------------------------------------------------------------------

/// A write stores the secret and returns its id.
#[tokio::test]
async fn writing_a_secret_stores_it() {
    let h = Harness::new("sec-put").await;
    let id = h.put("import_source", "src-1", "token", "k1", "ct-1").await;
    assert!(!id.is_empty(), "an id is returned");
    assert_eq!(h.count("secrets").await, 1);

    let row = get_secret(h.db(), "import_source", "src-1", "token")
        .await
        .expect("read")
        .expect("present");
    assert_eq!(row.id, id);
    assert_eq!(row.owner_type, "import_source");
    assert_eq!(row.owner_id, "src-1");
    assert_eq!(row.name, "token");
    assert_eq!(row.key_id, "k1");
    assert_eq!(row.nonce, "nonce-aabb");
    assert_eq!(row.ciphertext, "ct-1");
    assert_eq!(row.version, 1, "a first write is version 1");
}

/// The unique constraint is `(owner_type, owner_id, name)`, so the same name
/// under a different owner is a different secret.
#[tokio::test]
async fn the_same_name_under_a_different_owner_is_a_different_secret() {
    let h = Harness::new("sec-owner").await;
    h.put("import_source", "src-1", "token", "k1", "ct-a").await;
    h.put("import_source", "src-2", "token", "k1", "ct-b").await;
    assert_eq!(h.count("secrets").await, 2);
}

/// A different `owner_type` is also a different namespace.
#[tokio::test]
async fn a_different_owner_type_is_a_different_secret() {
    let h = Harness::new("sec-ownertype").await;
    h.put("import_source", "src-1", "token", "k1", "ct-a").await;
    h.put("account", "src-1", "token", "k1", "ct-b").await;
    assert_eq!(h.count("secrets").await, 2);
}

/// **The id is stable across a replacement.** This is what lets `imports.rs`
/// delete the `previous` id after a re-seal without deleting the row it just
/// superseded.
#[tokio::test]
async fn a_replacement_keeps_the_id_and_bumps_the_version() {
    let h = Harness::new("sec-replace").await;
    let first = h
        .put("import_source", "src-1", "token", "k1", "ct-old")
        .await;
    let second = h
        .put("import_source", "src-1", "token", "k1", "ct-new")
        .await;

    assert_eq!(first, second, "the id is reused, not reissued");
    assert_eq!(h.count("secrets").await, 1, "replaced, not added");
    let row = get_secret(h.db(), "import_source", "src-1", "token")
        .await
        .expect("read")
        .expect("present");
    assert_eq!(row.ciphertext, "ct-new", "the new ciphertext wins");
    assert_eq!(row.nonce, "nonce-aabb");
    assert_eq!(row.version, 2, "the version is incremented");
}

/// The version keeps climbing across repeated rewrites, so a cached ciphertext
/// from any earlier write is detectable as stale.
#[tokio::test]
async fn the_version_climbs_across_repeated_rewrites() {
    let h = Harness::new("sec-version").await;
    h.put("import_source", "src-1", "token", "k1", "ct-1").await;
    h.put("import_source", "src-1", "token", "k1", "ct-2").await;
    h.put("import_source", "src-1", "token", "k1", "ct-3").await;

    let row = get_secret(h.db(), "import_source", "src-1", "token")
        .await
        .expect("read")
        .expect("present");
    assert_eq!(row.version, 3);
    assert_eq!(row.ciphertext, "ct-3");
}

/// A replacement may move the secret to a different key — that is a rotation.
#[tokio::test]
async fn a_replacement_can_change_the_key() {
    let h = Harness::new("sec-rotate").await;
    h.put("import_source", "src-1", "token", "k1", "ct-1").await;
    h.put("import_source", "src-1", "token", "k2", "ct-2").await;

    let row = get_secret(h.db(), "import_source", "src-1", "token")
        .await
        .expect("read")
        .expect("present");
    assert_eq!(row.key_id, "k2", "now under the new key");
    assert_eq!(h.count("secrets").await, 1);
}

/// **A key that was never registered cannot encrypt anything** — the write is
/// refused by the foreign key.
#[tokio::test]
async fn writing_with_an_unregistered_key_is_refused() {
    let h = Harness::new("sec-fk").await;
    let result = put_secret(
        h.db(),
        "import_source",
        "src-1",
        "token",
        "never-registered",
        "nonce",
        "ct",
    )
    .await;
    assert!(result.is_err(), "key_id references encryption_keys");
    assert_eq!(h.count("secrets").await, 0, "and nothing is written");
}

/// Two secrets under one owner coexist under different names.
#[tokio::test]
async fn two_names_coexist_under_one_owner() {
    let h = Harness::new("sec-two-names").await;
    h.put("import_source", "src-1", "token", "k1", "ct-token")
        .await;
    h.put(
        "import_source",
        "src-1",
        "refresh_token",
        "k1",
        "ct-refresh",
    )
    .await;
    assert_eq!(h.count("secrets").await, 2);
}

/// `put_secret` reads the row back after writing, so a write it cannot see is an
/// error rather than a silent no-op.
#[tokio::test]
async fn the_returned_id_names_a_row_that_can_be_fetched() {
    let h = Harness::new("sec-roundtrip").await;
    let id = h.put("import_source", "src-1", "token", "k1", "ct").await;
    let by_id = get_secret_by_id(h.db(), &id)
        .await
        .expect("read by id")
        .expect("the returned id resolves");
    assert_eq!(by_id.ciphertext, "ct");
    assert_eq!(by_id.id, id);
}

/// An empty ciphertext is stored as given — this layer encrypts, it does not
/// validate, and deciding that an empty secret is meaningless belongs to the
/// caller.
#[tokio::test]
async fn an_empty_ciphertext_is_stored_verbatim() {
    let h = Harness::new("sec-empty-ct").await;
    h.put("import_source", "src-1", "token", "k1", "").await;
    let row = get_secret(h.db(), "import_source", "src-1", "token")
        .await
        .expect("read")
        .expect("present");
    assert_eq!(row.ciphertext, "");
}

// ---------------------------------------------------------------------------
// get_secret
// ---------------------------------------------------------------------------

/// Reading an absent secret is `None`, not an error.
#[tokio::test]
async fn reading_an_absent_secret_is_none() {
    let h = Harness::new("sec-get-absent").await;
    assert!(get_secret(h.db(), "import_source", "src-1", "token")
        .await
        .expect("read")
        .is_none());
}

/// The lookup is by the full triple, so a wrong owner does not find it.
#[tokio::test]
async fn the_lookup_requires_the_whole_triple() {
    let h = Harness::new("sec-get-triple").await;
    h.put("import_source", "src-1", "token", "k1", "ct").await;

    for (owner_type, owner_id, name) in [
        ("import_source", "src-2", "token"),
        ("import_source", "src-1", "other"),
        ("account", "src-1", "token"),
    ] {
        assert!(
            get_secret(h.db(), owner_type, owner_id, name)
                .await
                .expect("read")
                .is_none(),
            "{owner_type}/{owner_id}/{name} must not match"
        );
    }
}

/// A read does not mutate the row.
#[tokio::test]
async fn a_read_does_not_bump_the_version() {
    let h = Harness::new("sec-get-no-bump").await;
    h.put("import_source", "src-1", "token", "k1", "ct").await;
    for _ in 0..3 {
        get_secret(h.db(), "import_source", "src-1", "token")
            .await
            .expect("read");
    }
    let row = get_secret(h.db(), "import_source", "src-1", "token")
        .await
        .expect("read")
        .expect("present");
    assert_eq!(row.version, 1);
}

// ---------------------------------------------------------------------------
// get_secret_by_id
// ---------------------------------------------------------------------------

/// Fetching by id finds the same row as fetching by triple.
#[tokio::test]
async fn fetching_by_id_matches_the_triple_lookup() {
    let h = Harness::new("sec-by-id").await;
    let id = h.put("import_source", "src-1", "token", "k1", "ct").await;

    let by_id = get_secret_by_id(h.db(), &id)
        .await
        .expect("read")
        .expect("found");
    let by_triple = get_secret(h.db(), "import_source", "src-1", "token")
        .await
        .expect("read")
        .expect("found");
    assert_eq!(by_id.id, by_triple.id);
    assert_eq!(by_id.ciphertext, by_triple.ciphertext);
    assert_eq!(by_id.version, by_triple.version);
    assert_eq!(by_id.created_at, by_triple.created_at);
}

/// An unknown id is `None`.
#[tokio::test]
async fn fetching_an_unknown_id_is_none() {
    let h = Harness::new("sec-by-id-unknown").await;
    assert!(get_secret_by_id(h.db(), &Uuid::new_v4().to_string())
        .await
        .expect("read")
        .is_none());
}

/// A malformed id is `None` rather than an error: the query compares
/// `id::text`, so a non-UUID string simply matches no row.
///
/// This is the asymmetry with `rating_integrity`, where the same shape of
/// malformed id is *rejected* — there the column is a native `UUID` and the
/// repository parses before binding; here it compares text.
#[tokio::test]
async fn a_malformed_id_is_none_rather_than_an_error() {
    let h = Harness::new("sec-bad-id").await;
    assert!(get_secret_by_id(h.db(), "not-a-uuid")
        .await
        .expect("a malformed id is not a database error")
        .is_none());
}

/// The id is unique across secrets, so a by-id fetch is unambiguous even when
/// several secrets exist.
#[tokio::test]
async fn the_id_is_unique_across_secrets() {
    let h = Harness::new("sec-id-unique").await;
    let a = h.put("import_source", "src-1", "token", "k1", "ct-a").await;
    let b = h.put("import_source", "src-2", "token", "k1", "ct-b").await;
    assert_ne!(a, b, "two writes to different owners get different ids");

    assert_eq!(
        get_secret_by_id(h.db(), &a)
            .await
            .expect("read")
            .expect("a")
            .ciphertext,
        "ct-a"
    );
    assert_eq!(
        get_secret_by_id(h.db(), &b)
            .await
            .expect("read")
            .expect("b")
            .ciphertext,
        "ct-b"
    );
}

// ---------------------------------------------------------------------------
// delete_secret
// ---------------------------------------------------------------------------

/// Deleting reports whether it removed anything.
#[tokio::test]
async fn deleting_reports_whether_a_row_went_away() {
    let h = Harness::new("sec-delete").await;
    let id = h.put("import_source", "src-1", "token", "k1", "ct").await;

    assert!(
        delete_secret(h.db(), &id).await.expect("delete"),
        "removed one"
    );
    assert_eq!(h.count("secrets").await, 0);
    assert!(
        !delete_secret(h.db(), &id).await.expect("delete"),
        "and deleting it again reports false"
    );
}

/// Deleting an unknown id is `false`, not an error.
#[tokio::test]
async fn deleting_an_unknown_id_is_false() {
    let h = Harness::new("sec-delete-unknown").await;
    assert!(!delete_secret(h.db(), &Uuid::new_v4().to_string())
        .await
        .expect("delete"));
}

/// Deleting one secret leaves its siblings alone.
#[tokio::test]
async fn deleting_one_secret_leaves_the_others() {
    let h = Harness::new("sec-delete-sibling").await;
    let keep = h
        .put("import_source", "src-1", "refresh_token", "k1", "ct-keep")
        .await;
    let drop = h
        .put("import_source", "src-1", "token", "k1", "ct-drop")
        .await;

    assert!(delete_secret(h.db(), &drop).await.expect("delete"));
    assert!(get_secret_by_id(h.db(), &keep)
        .await
        .expect("read")
        .is_some());
    assert!(get_secret_by_id(h.db(), &drop)
        .await
        .expect("read")
        .is_none());
}

/// **Deleting a secret does not delete the key it used**, and the key stays
/// usable for a later write.
#[tokio::test]
async fn deleting_a_secret_keeps_the_key() {
    let h = Harness::new("sec-delete-keeps-key").await;
    let id = h.put("import_source", "src-1", "token", "k1", "ct").await;
    delete_secret(h.db(), &id).await.expect("delete");

    assert_eq!(h.count("encryption_keys").await, 1, "the key survives");
    let again = h.put("import_source", "src-2", "token", "k1", "ct-2").await;
    assert!(!again.is_empty(), "and still encrypts");
}

/// After a delete, the freed `(owner, name)` can be written again — and that
/// write is a fresh row at version 1, not a continuation.
#[tokio::test]
async fn a_name_freed_by_a_delete_starts_a_fresh_version_one() {
    let h = Harness::new("sec-delete-fresh").await;
    let first = h.put("import_source", "src-1", "token", "k1", "ct-1").await;
    h.put("import_source", "src-1", "token", "k1", "ct-2").await; // version 2
    delete_secret(h.db(), &first).await.expect("delete");

    let again = h.put("import_source", "src-1", "token", "k1", "ct-3").await;
    let row = get_secret_by_id(h.db(), &again)
        .await
        .expect("read")
        .expect("present");
    assert_eq!(row.version, 1, "a new row, not the old one resumed");
    assert_ne!(row.id, first, "with a fresh id");
}

/// The lifecycle `imports.rs` performs end to end: seal, re-seal under a new
/// key, and confirm the caller can still read the current value.
///
/// Because the id is stable across a replacement, the `previous` id can name
/// the row just written. `imports.rs` guards exactly that with
/// `if previous != secret_id` before deleting, so a same-label rotation keeps
/// its row; the `previous != secret_id` check is what makes the stable id safe,
/// and this test pins the property that check depends on.
#[tokio::test]
async fn the_rotate_and_read_lifecycle_works() {
    let h = Harness::new("sec-lifecycle").await;
    let previous = h
        .put("import_source", "src-1", "token", "k1", "ct-old")
        .await;

    h.put("import_source", "src-1", "token", "k2", "ct-new")
        .await;
    let current = get_secret(h.db(), "import_source", "src-1", "token")
        .await
        .expect("read")
        .expect("present");
    assert_eq!(current.id, previous, "the id survived the rotation");
    assert_eq!(current.key_id, "k2", "under the new key");
    assert_eq!(current.ciphertext, "ct-new");
    assert_eq!(current.version, 2);
    assert_eq!(h.count("secrets").await, 1, "rotation replaced, not added");

    // A reader holding the retired key's ciphertext can still tell it is stale.
    assert!(current.version > 1);
    assert_eq!(
        h.text_of("SELECT retired_at FROM encryption_keys WHERE key_id = 'k1'")
            .await,
        None,
        "and the old key can be retired afterwards"
    );
    retire_encryption_key(h.db(), "k1").await.expect("retire");
    assert!(get_secret(h.db(), "import_source", "src-1", "token")
        .await
        .expect("still readable")
        .is_some());
}
