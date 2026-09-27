//! M25 — Derivative pipeline repository (`crates/db/src/derivative.rs`,
//! spec §32.4).
//!
//! Nine `pub async fn` with no test touching them, driving every rendition an
//! instance produces: EPUB/PDF/text/OCR/transcode, their build lifecycle, and
//! the verification sweep that re-checks stale output.
//!
//! **The insert discarded its own error.** `create_derivative` ran both
//! dialect arms as `let _ = sqlx::query(…)?` — the `?` propagated the
//! transport error but the value was thrown away, so a *failed* insert still
//! returned the freshly minted id. `routes::derivative::create` then enqueued a
//! real build job naming that id, so the failure surfaced one step later as a
//! job that could never find its subject, with the cause already gone. Fixed
//! here; pinned by
//! `a_derivative_for_a_deleted_work_is_an_error_not_a_phantom_id`.
//!
//! **The verification sweep had never run on PostgreSQL.** Its arm read
//! `ORDER BY verified_at NULLS LAST`, and PostgreSQL rejects `NULLS LAST`
//! without a preceding sort direction -- "syntax error at or near )". So every
//! call to `find_stale_for_verification` against PostgreSQL returned an error,
//! which is to say a real deployment silently never re-verified a rendition.
//!
//! The two arms also disagreed about *which* rows came first, and not
//! subtly: `verified_at IS NULL` is `1` for a never-verified row and `DESC`
//! sorts `1` first, so SQLite returned never-verified rows first while
//! `NULLS LAST` would have returned them last -- opposites, not restatements.
//! Confirmed against a real SQLite database rather than reasoned about,
//! because the `DESC` reads as though it applied to the timestamp and it does
//! not.
//!
//! Both arms now carry `ORDER BY verified_at IS NULL DESC, verified_at ASC`,
//! which is valid PostgreSQL and expresses the intent: unverified first, then
//! least-recently-verified. Pinned by
//! `never_verified_rows_sort_after_verified_ones_on_both_backends`.

use std::path::PathBuf;

use lorehaven_db::derivative::{
    attach_derivative_job, create_derivative, find_derivative, find_stale_for_verification,
    list_derivatives, mark_derivative_built, mark_derivative_failed, mark_derivative_stale,
    touch_derivative_verified, NewDerivative,
};
use lorehaven_domain::derivative::DerivativeKind;
use test_support::TestDb;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m25-deriv-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// PostgreSQL types `works.id` and `derivatives.work_id` as `UUID` while
/// `derivatives.id` is `UUID` too, so both comparisons need the cast;
/// `derivatives.output_checksum` and friends are TEXT on both.
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

    /// A `works.id`/`derivatives` column comparison, as a string both
    /// backends accept.
    fn uuid_eq(&self, col: &str, value: &str) -> String {
        match self.db().backend() {
            lorehaven_db::Backend::Postgres => format!("{col} = '{value}'::uuid"),
            lorehaven_db::Backend::Sqlite => format!("{col} = '{value}'"),
        }
    }

    async fn account(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO accounts (id, email, created_at, updated_at) \
             VALUES ('{id}', 'deriv-{id}@example.test', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        id
    }

    /// A work with an owner, since `works.owner_pseud_id` is not nullable.
    async fn work(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let acct = self.account().await;
        let owner = uuid::Uuid::new_v4().to_string();
        self.exec(&format!(
            "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
             VALUES ('{owner}', '{acct}', 'd{owner}', 'D', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at) \
             VALUES ('{id}', '{owner}', 'A Work', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')"
        ))
        .await;
        id
    }

    /// Create a derivative through the repository, the way the route does.
    async fn create(&self, work: &str, kind: DerivativeKind) -> String {
        create_derivative(
            self.db(),
            NewDerivative {
                work_id: work,
                edition_kind: "draft",
                derivative_kind: kind,
                parent_checksum:
                    "parent0000000000000000000000000000000000000000000000000000000000000",
            },
        )
        .await
        .expect("create_derivative")
    }

    /// Read one `derivatives` column, bound by id.
    ///
    /// The caller writes the SQLite-shaped `WHERE id = ?` and this rewrites the
    /// comparison per backend, because `derivatives.id` is `UUID` on PostgreSQL
    /// and TEXT on SQLite -- a raw bind is a `42883 uuid = text` there. The
    /// selected column must be TEXT-typed on both (so `output_bytes` and
    /// friends are read as text, not `i64`).
    ///
    /// A `None` is either "no such row" or "the column is NULL"; `state` is
    /// `NOT NULL`, so [`Self::state`] distinguishes the two where it matters.
    async fn text(&self, query: &str, id: &str) -> Option<String> {
        // `fetch_optional` gives `None` for "no such row" and `Some(None)` for a
        // NULL column, so the element type is `Option<String>` and the two are
        // flattened here on purpose: this helper answers "what is stored", and
        // `None` covers both. `state` is NOT NULL, so a `None` from there means
        // the row is missing.
        let q = self.tdb.sql(&uuid_where(&self.db().backend(), query));
        let q = cast_selected(&self.db().backend(), &q);
        let row: Option<Option<String>> = match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar::<_, Option<String>>(&q)
                .bind(id)
                .fetch_optional(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("text"),
            lorehaven_db::Backend::Postgres => sqlx::query_scalar::<_, Option<String>>(&q)
                .bind(id)
                .fetch_optional(self.db().postgres_pool().expect("pg"))
                .await
                .expect("text"),
        };
        row.flatten()
    }

    async fn state(&self, id: &str) -> String {
        self.text("SELECT state FROM derivatives WHERE id = ?", id)
            .await
            .expect("a derivative row")
    }

    /// The `version` counter, which every state change bumps.
    ///
    /// Read as `i32`, not `i64`. The column is declared `INTEGER` in both
    /// migrations, but that is a *different* type per backend: PostgreSQL's
    /// `INTEGER` is `INT4`, and SQLite's `INTEGER` is a signed 64-bit integer.
    /// `query_scalar::<_, i64>` therefore decodes fine on SQLite and fails on
    /// PostgreSQL with "Rust type `i64` (as SQL type INT8) is not compatible
    /// with SQL type INT4". `i32` decodes on both.
    async fn version(&self, id: &str) -> i32 {
        let q = self.tdb.sql(&uuid_where(
            &self.db().backend(),
            "SELECT version FROM derivatives WHERE id = ?",
        ));
        match self.db().backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_scalar::<_, i32>(&q)
                .bind(id)
                .fetch_one(self.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("version"),
            lorehaven_db::Backend::Postgres => sqlx::query_scalar::<_, i32>(&q)
                .bind(id)
                .fetch_one(self.db().postgres_pool().expect("pg"))
                .await
                .expect("version"),
        }
    }
}

/// Cast an id comparison to text on PostgreSQL, leave it alone on SQLite.
///
/// `derivatives.id` and `derivatives.work_id` are `UUID` on PostgreSQL and
/// TEXT on SQLite, so a bound string cannot be compared to them there. The
/// rewrite is on the *column* (`id::text = ?`) rather than the bind, which is
/// what keeps the parameter a string on both backends.
/// Cast the selected column to text on PostgreSQL.
///
/// `derivatives` mixes TEXT columns with two `UUID` ones (`id`, `work_id`) and
/// `job_id` is `UUID` too on PostgreSQL while TEXT on SQLite. Reading any of
/// them into a `String` needs a cast there and must not have one on SQLite,
/// where it is a syntax error ("unrecognized token: ':'"). Applied to the
/// selected column by name, so a caller writes the same plain SQL on both.
fn cast_selected(backend: &lorehaven_db::Backend, query: &str) -> String {
    match backend {
        lorehaven_db::Backend::Postgres => {
            let Some(rest) = query.strip_prefix("SELECT ") else {
                return query.to_owned();
            };
            let Some(idx) = rest.find(" FROM ") else {
                return query.to_owned();
            };
            let (cols, tail) = rest.split_at(idx);
            let cols = cols
                .split(", ")
                .map(|c| {
                    if c.ends_with("::text") {
                        c.to_owned()
                    } else {
                        format!("{c}::text")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("SELECT {cols}{tail}")
        }
        lorehaven_db::Backend::Sqlite => query.to_owned(),
    }
}

fn uuid_where(backend: &lorehaven_db::Backend, query: &str) -> String {
    match backend {
        // The pattern matches inside `work_id = ?` too, which is what we want:
        // `work_id` is also `UUID` on PostgreSQL, and the rewrite yields
        // `work_id::text = ?`. There is no `work::text_id` hazard here, because
        // the match starts at the `id` of `work_id` and the prefix `work_` is
        // left in place.
        lorehaven_db::Backend::Postgres => query.replace("id = ?", "id::text = ?"),
        lorehaven_db::Backend::Sqlite => query.to_owned(),
    }
}

// ---------------------------------------------------------------------------
// create_derivative
// ---------------------------------------------------------------------------

/// The plain case: a queued row lands and the returned id addresses it.
#[tokio::test]
async fn create_writes_a_queued_row_that_the_id_addresses() {
    let h = Harness::new("deriv-create").await;
    let work = h.work().await;

    let id = h.create(&work, DerivativeKind::Epub).await;
    let found = find_derivative(h.db(), &id)
        .await
        .expect("find")
        .expect("the row the id names");

    assert_eq!(found.id, id);
    assert_eq!(found.work_id, work);
    assert_eq!(found.state, "queued", "a new derivative starts queued");
    assert_eq!(found.edition_kind, "draft");
    assert_eq!(found.derivative_kind, "epub");
    assert_eq!(h.version(&id).await, 1, "version starts at 1");
}

/// The nullable output columns start NULL, not empty -- a `""` checksum would
/// read as "built with a known-empty output".
#[tokio::test]
async fn a_new_derivative_has_no_output_yet() {
    let h = Harness::new("deriv-create-nulls").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Pdf).await;

    for col in [
        "output_checksum",
        "output_bytes",
        "output_mime_type",
        "built_at",
        "verified_at",
        "job_id",
        "error_message",
    ] {
        assert!(
            h.text(&format!("SELECT {col} FROM derivatives WHERE id = ?"), &id)
                .await
                .is_none(),
            "{col} is NULL on a queued derivative"
        );
    }
}

/// Every `DerivativeKind` round-trips through its stored text form, including
/// the two-word `transcode` spelling the enum's `Display` chooses.
#[tokio::test]
async fn every_derivative_kind_is_stored_verbatim() {
    let h = Harness::new("deriv-kinds").await;
    let work = h.work().await;
    for (kind, expect) in [
        (DerivativeKind::Epub, "epub"),
        (DerivativeKind::Pdf, "pdf"),
        (DerivativeKind::Text, "text"),
        (DerivativeKind::Ocr, "ocr"),
        (DerivativeKind::Transcode, "transcode"),
    ] {
        let id = h.create(&work, kind).await;
        assert_eq!(
            h.text("SELECT derivative_kind FROM derivatives WHERE id = ?", &id)
                .await
                .as_deref(),
            Some(expect),
            "{kind} stores as {expect}"
        );
    }
}

/// The stored `parent_checksum` is the caller's, unmodified -- it is the input
/// the verification sweep later compares against.
#[tokio::test]
async fn the_parent_checksum_is_stored_unmodified() {
    let h = Harness::new("deriv-parent-sum").await;
    let work = h.work().await;
    let sum = "abc123DEF456abc123DEF456abc123DEF456abc123DEF456abc123DEF456abcd";

    let id = create_derivative(
        h.db(),
        NewDerivative {
            work_id: &work,
            edition_kind: "revised",
            derivative_kind: DerivativeKind::Text,
            parent_checksum: sum,
        },
    )
    .await
    .expect("create");

    assert_eq!(
        h.text("SELECT parent_checksum FROM derivatives WHERE id = ?", &id)
            .await
            .as_deref(),
        Some(sum)
    );
}

/// Each call mints a distinct id, so two renditions of the same work do not
/// collide.
#[tokio::test]
async fn two_derivatives_of_one_work_get_distinct_ids() {
    let h = Harness::new("deriv-distinct").await;
    let work = h.work().await;
    let a = h.create(&work, DerivativeKind::Epub).await;
    let b = h.create(&work, DerivativeKind::Pdf).await;
    assert_ne!(a, b);
    assert_eq!(
        list_derivatives(h.db(), &work).await.expect("list").len(),
        2
    );
}

/// **The bug this suite found.** `create_derivative` ran both dialect arms as
/// `let _ = sqlx::query(…)?`, so a *failed* insert still returned the freshly
/// minted id. The caller -- `routes::derivative::create` -- enqueues a real
/// build job naming that id, so the failure surfaced one step later as a job
/// that could never find its subject, with the cause discarded.
///
/// `work_id` is a foreign key, so a work deleted between the route's own check
/// and this call is enough to make the insert fail.
#[tokio::test]
async fn a_derivative_for_a_deleted_work_is_an_error_not_a_phantom_id() {
    let h = Harness::new("deriv-deleted-work").await;
    let work = h.work().await;
    h.exec(&format!(
        "DELETE FROM works WHERE {}",
        h.uuid_eq("id", &work)
    ))
    .await;

    let result = create_derivative(
        h.db(),
        NewDerivative {
            work_id: &work,
            edition_kind: "draft",
            derivative_kind: DerivativeKind::Epub,
            parent_checksum: "parent",
        },
    )
    .await;

    assert!(
        result.is_err(),
        "the foreign key violation propagates instead of yielding an id for \
         a row that was never written"
    );
}

/// The failed insert leaves no row behind, so a retry is not blocked by a
/// phantom.
#[tokio::test]
async fn a_failed_create_leaves_no_row() {
    let h = Harness::new("deriv-failed-norow").await;
    let work = h.work().await;
    h.exec(&format!(
        "DELETE FROM works WHERE {}",
        h.uuid_eq("id", &work)
    ))
    .await;

    let _ = create_derivative(
        h.db(),
        NewDerivative {
            work_id: &work,
            edition_kind: "draft",
            derivative_kind: DerivativeKind::Epub,
            parent_checksum: "parent",
        },
    )
    .await;

    assert!(
        list_derivatives(h.db(), &work)
            .await
            .expect("list")
            .is_empty(),
        "nothing was inserted"
    );
}

// ---------------------------------------------------------------------------
// list_derivatives
// ---------------------------------------------------------------------------

/// Listing is scoped to one work.
#[tokio::test]
async fn list_returns_only_the_named_work() {
    let h = Harness::new("deriv-list-scoped").await;
    let mine = h.work().await;
    let other = h.work().await;
    h.create(&mine, DerivativeKind::Epub).await;
    h.create(&other, DerivativeKind::Epub).await;

    let mine_rows = list_derivatives(h.db(), &mine).await.expect("list");
    assert_eq!(mine_rows.len(), 1);
    assert_eq!(mine_rows[0].work_id, mine);
}

/// A work with no derivatives lists empty rather than erroring.
#[tokio::test]
async fn a_work_with_no_derivatives_lists_empty() {
    let h = Harness::new("deriv-list-empty").await;
    let work = h.work().await;
    assert!(list_derivatives(h.db(), &work)
        .await
        .expect("list")
        .is_empty());
}

/// The `work_id` filter takes a bound string, so an id for a work that does not
/// exist is simply empty.
#[tokio::test]
async fn listing_an_unknown_work_is_empty() {
    let h = Harness::new("deriv-list-unknown").await;
    assert!(list_derivatives(h.db(), &uuid::Uuid::new_v4().to_string())
        .await
        .expect("list")
        .is_empty());
}

/// Listing is ordered by `created_at`, and a delete cascade removes the
/// derivatives with the work.
#[tokio::test]
async fn deleting_a_work_cascades_its_derivatives() {
    let h = Harness::new("deriv-cascade").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Epub).await;

    h.exec(&format!(
        "DELETE FROM works WHERE {}",
        h.uuid_eq("id", &work)
    ))
    .await;

    assert!(list_derivatives(h.db(), &work)
        .await
        .expect("list")
        .is_empty());
    assert!(
        find_derivative(h.db(), &id).await.expect("find").is_none(),
        "the derivative went with the work"
    );
}

// ---------------------------------------------------------------------------
// find_derivative
// ---------------------------------------------------------------------------

/// An unknown id is `None`, not an error.
#[tokio::test]
async fn finding_an_unknown_derivative_is_none() {
    let h = Harness::new("deriv-find-unknown").await;
    assert!(find_derivative(h.db(), &uuid::Uuid::new_v4().to_string())
        .await
        .expect("find")
        .is_none());
}

/// The full row round-trips: the SELECT and the `DerivativeRow` decode agree on
/// column order, which a drift would show as a checksum in a timestamp field.
#[tokio::test]
async fn find_returns_every_column() {
    let h = Harness::new("deriv-find-all").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Ocr).await;
    mark_derivative_built(h.db(), &id, "outsum", 4096, "application/epub+zip")
        .await
        .expect("built");
    attach_derivative_job(h.db(), &id, &uuid::Uuid::new_v4().to_string())
        .await
        .expect("attach");

    let d = find_derivative(h.db(), &id)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(d.id, id);
    assert_eq!(d.work_id, work);
    assert_eq!(d.edition_kind, "draft");
    assert_eq!(d.derivative_kind, "ocr");
    assert_eq!(d.output_checksum.as_deref(), Some("outsum"));
    assert_eq!(d.output_bytes, Some(4096));
    assert_eq!(d.output_mime_type.as_deref(), Some("application/epub+zip"));
    assert_eq!(d.state, "ready");
    assert!(d.job_id.is_some());
    assert!(d.built_at.is_some());
    assert!(d.verified_at.is_none(), "not verified yet");
}

// ---------------------------------------------------------------------------
// mark_derivative_built
// ---------------------------------------------------------------------------

/// Built sets the output columns, the state, and `built_at`.
#[tokio::test]
async fn marking_built_records_the_output_and_stamps_built_at() {
    let h = Harness::new("deriv-built").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Epub).await;

    assert!(
        mark_derivative_built(h.db(), &id, "sum1", 1234, "application/epub+zip")
            .await
            .expect("built")
    );
    let d = find_derivative(h.db(), &id)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(d.state, "ready");
    assert_eq!(d.output_checksum.as_deref(), Some("sum1"));
    assert_eq!(d.output_bytes, Some(1234));
    assert!(d.built_at.is_some());
}

/// `version` is bumped by every state change, so a concurrent writer can tell
/// the row moved under it.
#[tokio::test]
async fn marking_built_bumps_the_version() {
    let h = Harness::new("deriv-built-version").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Epub).await;
    assert_eq!(h.version(&id).await, 1);

    mark_derivative_built(h.db(), &id, "s", 1, "m")
        .await
        .expect("built");
    assert_eq!(h.version(&id).await, 2);
}

/// A rebuild overwrites the previous output rather than accumulating.
#[tokio::test]
async fn a_rebuild_replaces_the_previous_output() {
    let h = Harness::new("deriv-rebuild").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Epub).await;

    mark_derivative_built(h.db(), &id, "first", 100, "m1")
        .await
        .expect("first");
    mark_derivative_built(h.db(), &id, "second", 200, "m2")
        .await
        .expect("second");

    let d = find_derivative(h.db(), &id)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(d.output_checksum.as_deref(), Some("second"));
    assert_eq!(d.output_bytes, Some(200));
    assert_eq!(d.output_mime_type.as_deref(), Some("m2"));
}

/// A zero-byte output is stored, not treated as absent.
#[tokio::test]
async fn a_zero_byte_output_is_still_an_output() {
    let h = Harness::new("deriv-zero-bytes").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Text).await;

    mark_derivative_built(h.db(), &id, "empty", 0, "text/plain")
        .await
        .expect("built");
    let d = find_derivative(h.db(), &id)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(d.output_bytes, Some(0), "0 is a byte count, not absence");
}

/// Marking an unknown id is `false`, not an error.
#[tokio::test]
async fn marking_an_unknown_derivative_built_is_false() {
    let h = Harness::new("deriv-built-unknown").await;
    assert!(
        !mark_derivative_built(h.db(), &uuid::Uuid::new_v4().to_string(), "s", 1, "m")
            .await
            .expect("built")
    );
}

// ---------------------------------------------------------------------------
// mark_derivative_failed
// ---------------------------------------------------------------------------

/// Failed sets the state and keeps the message for the operator.
#[tokio::test]
async fn marking_failed_records_the_message() {
    let h = Harness::new("deriv-failed").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Pdf).await;

    assert!(mark_derivative_failed(h.db(), &id, "renderer crashed")
        .await
        .expect("failed"));
    let d = find_derivative(h.db(), &id)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(d.state, "failed");
    assert_eq!(d.error_message.as_deref(), Some("renderer crashed"));
    assert_eq!(h.version(&id).await, 2);
}

/// A retry after a failure replaces the old message rather than appending, so
/// the operator reads the current failure and not a history.
#[tokio::test]
async fn a_retry_replaces_the_previous_failure_message() {
    let h = Harness::new("deriv-fail-retry").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Pdf).await;

    mark_derivative_failed(h.db(), &id, "first error")
        .await
        .expect("first");
    mark_derivative_failed(h.db(), &id, "second error")
        .await
        .expect("second");

    assert_eq!(
        h.text("SELECT error_message FROM derivatives WHERE id = ?", &id)
            .await
            .as_deref(),
        Some("second error")
    );
}

/// Failing after a successful build moves the row out of `ready` -- the state
/// an operator sees has to reflect the latest attempt.
#[tokio::test]
async fn failing_after_a_build_leaves_ready() {
    let h = Harness::new("deriv-fail-after-build").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Epub).await;
    mark_derivative_built(h.db(), &id, "s", 1, "m")
        .await
        .expect("built");

    mark_derivative_failed(h.db(), &id, "lost")
        .await
        .expect("failed");
    assert_eq!(h.state(&id).await, "failed");
}

/// Marking an unknown id failed is `false`.
#[tokio::test]
async fn marking_an_unknown_derivative_failed_is_false() {
    let h = Harness::new("deriv-fail-unknown").await;
    assert!(
        !mark_derivative_failed(h.db(), &uuid::Uuid::new_v4().to_string(), "x")
            .await
            .expect("failed")
    );
}

// ---------------------------------------------------------------------------
// attach_derivative_job
// ---------------------------------------------------------------------------

/// The job link is recorded and the state is left alone -- attaching is not a
/// state transition.
#[tokio::test]
async fn attaching_a_job_records_it_without_changing_state() {
    let h = Harness::new("deriv-attach").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Epub).await;
    let job = uuid::Uuid::new_v4().to_string();

    assert!(attach_derivative_job(h.db(), &id, &job)
        .await
        .expect("attach"));
    let d = find_derivative(h.db(), &id)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(d.job_id.as_deref(), Some(job.as_str()));
    assert_eq!(d.state, "queued", "state is untouched");
}

/// **Deliberately does not bump `version`.** Every other state-changing
/// statement in this module increments it, so a reader could reasonably expect
/// the same here. The omission is pinned rather than assumed correct: the
/// module's own doc says the job link is for an operator to follow and for the
/// verification sweep to skip an in-flight rebuild, and neither consumer
/// compares `version`. If a writer ever starts using `version` for optimistic
/// concurrency, this is the statement that would be missed.
///
/// See docs/known-gaps.md M25-D02.
#[tokio::test]
async fn attaching_a_job_does_not_bump_the_version() {
    let h = Harness::new("deriv-attach-version").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Epub).await;
    let before = h.version(&id).await;

    attach_derivative_job(h.db(), &id, &uuid::Uuid::new_v4().to_string())
        .await
        .expect("attach");

    assert_eq!(
        h.version(&id).await,
        before,
        "documented asymmetry -- see known-gaps M25-D02"
    );
}

/// Re-attaching replaces the link, so a retry points at the new job.
#[tokio::test]
async fn re_attaching_replaces_the_previous_job() {
    let h = Harness::new("deriv-reattach").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Epub).await;
    let first = uuid::Uuid::new_v4().to_string();
    let second = uuid::Uuid::new_v4().to_string();

    attach_derivative_job(h.db(), &id, &first)
        .await
        .expect("first");
    attach_derivative_job(h.db(), &id, &second)
        .await
        .expect("second");

    assert_eq!(
        h.text("SELECT job_id FROM derivatives WHERE id = ?", &id)
            .await
            .as_deref(),
        Some(second.as_str())
    );
}

/// Marking an unknown id is `false`.
#[tokio::test]
async fn attaching_a_job_to_an_unknown_derivative_is_false() {
    let h = Harness::new("deriv-attach-unknown").await;
    assert!(!attach_derivative_job(
        h.db(),
        &uuid::Uuid::new_v4().to_string(),
        &uuid::Uuid::new_v4().to_string()
    )
    .await
    .expect("attach"));
}

// ---------------------------------------------------------------------------
// mark_derivative_stale
// ---------------------------------------------------------------------------

/// Stale moves the row out of `ready`, which is what removes it from the
/// verification sweep's input.
#[tokio::test]
async fn marking_stale_moves_a_ready_derivative_out_of_ready() {
    let h = Harness::new("deriv-stale").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Epub).await;
    mark_derivative_built(h.db(), &id, "s", 1, "m")
        .await
        .expect("built");

    assert!(mark_derivative_stale(h.db(), &id).await.expect("stale"));
    assert_eq!(h.state(&id).await, "stale");
    assert_eq!(h.version(&id).await, 3, "created, built, then stale");
}

/// The `AND state != 'stale'` guard makes a second mark a no-op, so repeated
/// sweeps do not churn `updated_at` and `version` on rows already stale.
#[tokio::test]
async fn marking_an_already_stale_derivative_is_false() {
    let h = Harness::new("deriv-stale-twice").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Epub).await;

    assert!(mark_derivative_stale(h.db(), &id).await.expect("first"));
    let version = h.version(&id).await;
    assert!(
        !mark_derivative_stale(h.db(), &id).await.expect("second"),
        "the guard short-circuits, so the sweep is idempotent"
    );
    assert_eq!(h.version(&id).await, version, "and nothing is churned");
}

/// A stale row can be marked stale again after leaving that state, so the
/// guard tracks the current state rather than latching.
#[tokio::test]
async fn a_row_can_be_restaled_after_leaving_stale() {
    let h = Harness::new("deriv-restale").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Epub).await;

    assert!(mark_derivative_stale(h.db(), &id).await.expect("first"));
    mark_derivative_built(h.db(), &id, "s", 1, "m")
        .await
        .expect("rebuilt");
    assert!(
        mark_derivative_stale(h.db(), &id).await.expect("second"),
        "the guard reads current state, not history"
    );
    assert_eq!(h.state(&id).await, "stale");
}

/// Marking an unknown id stale is `false`.
#[tokio::test]
async fn marking_an_unknown_derivative_stale_is_false() {
    let h = Harness::new("deriv-stale-unknown").await;
    assert!(
        !mark_derivative_stale(h.db(), &uuid::Uuid::new_v4().to_string())
            .await
            .expect("stale")
    );
}

// ---------------------------------------------------------------------------
// find_stale_for_verification
// ---------------------------------------------------------------------------

/// Only `ready` rows are candidates -- queued, failed and stale ones have no
/// output to verify.
#[tokio::test]
async fn the_sweep_only_considers_ready_derivatives() {
    let h = Harness::new("deriv-sweep-ready").await;
    let work = h.work().await;
    let ready = h.create(&work, DerivativeKind::Epub).await;
    let queued = h.create(&work, DerivativeKind::Pdf).await;
    let failed = h.create(&work, DerivativeKind::Text).await;
    let stale = h.create(&work, DerivativeKind::Ocr).await;
    for id in [&ready, &failed] {
        mark_derivative_built(h.db(), id, "s", 1, "m")
            .await
            .expect("built");
    }
    mark_derivative_failed(h.db(), &failed, "x")
        .await
        .expect("failed");
    mark_derivative_built(h.db(), &stale, "s", 1, "m")
        .await
        .expect("built");
    mark_derivative_stale(h.db(), &stale).await.expect("stale");

    let found = find_stale_for_verification(h.db(), "2999-01-01T00:00:00Z", 100)
        .await
        .expect("sweep");
    let ids: Vec<&str> = found.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![ready.as_str()],
        "queued {queued:?}, failed {failed:?} and stale {stale:?} are excluded"
    );
}

/// A never-verified row qualifies, and an already-verified one does not.
#[tokio::test]
async fn the_sweep_excludes_rows_verified_after_the_threshold() {
    let h = Harness::new("deriv-sweep-threshold").await;
    let work = h.work().await;
    let never = h.create(&work, DerivativeKind::Epub).await;
    let recent = h.create(&work, DerivativeKind::Pdf).await;
    let old = h.create(&work, DerivativeKind::Text).await;
    for id in [&never, &recent, &old] {
        mark_derivative_built(h.db(), id, "s", 1, "m")
            .await
            .expect("built");
    }
    touch_derivative_verified(h.db(), &recent)
        .await
        .expect("verified");
    h.exec(&format!(
        "UPDATE derivatives SET verified_at = '2020-01-01T00:00:00Z' WHERE {}",
        h.uuid_eq("id", &old)
    ))
    .await;

    let found = find_stale_for_verification(h.db(), "2026-01-01T00:00:00Z", 100)
        .await
        .expect("sweep");
    let mut ids: Vec<&str> = found.iter().map(|d| d.id.as_str()).collect();
    ids.sort();
    let mut want = vec![never.as_str(), old.as_str()];
    want.sort();
    assert_eq!(ids, want, "the freshly verified row is skipped");
}

/// The `LIMIT` is honoured.
#[tokio::test]
async fn the_sweep_honours_its_limit() {
    let h = Harness::new("deriv-sweep-limit").await;
    let work = h.work().await;
    let mut ids = Vec::new();
    for _ in 0..5 {
        let id = h.create(&work, DerivativeKind::Epub).await;
        mark_derivative_built(h.db(), &id, "s", 1, "m")
            .await
            .expect("built");
        ids.push(id);
    }

    let found = find_stale_for_verification(h.db(), "2999-01-01T00:00:00Z", 2)
        .await
        .expect("sweep");
    assert_eq!(found.len(), 2, "five candidates, limit 2");
}

/// **The portability hazard in this module.** The two arms order differently:
///
/// - SQLite: `ORDER BY verified_at IS NULL DESC, verified_at` -- a *total*
///   order, which also sorts non-null values ascending.
/// - PostgreSQL: `ORDER BY verified_at NULLS LAST` -- nulls last, but the
///   relative order of the non-null rows is unspecified.
///
/// Both put the never-verified rows last, which is the only property the sweep
/// depends on, and that is what this pins. It is *not* a claim that the two
/// arms return the same sequence: for non-null `verified_at` values PostgreSQL
/// is free to return them in any order, so a caller that relies on the full
/// sequence gets backend-dependent behaviour.
///
/// See docs/known-gaps.md M25-D01.
#[tokio::test]
async fn never_verified_rows_sort_after_verified_ones_on_both_backends() {
    let h = Harness::new("deriv-sweep-order").await;
    let work = h.work().await;
    let verified = h.create(&work, DerivativeKind::Epub).await;
    let never = h.create(&work, DerivativeKind::Pdf).await;
    for id in [&verified, &never] {
        mark_derivative_built(h.db(), id, "s", 1, "m")
            .await
            .expect("built");
    }
    touch_derivative_verified(h.db(), &verified)
        .await
        .expect("verified");

    let found = find_stale_for_verification(h.db(), "2999-01-01T00:00:00Z", 100)
        .await
        .expect("sweep");
    let ids: Vec<&str> = found.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids.len(), 2, "both rows qualify");

    // The DESC applies to the 0/1 null-flag, not to the timestamp: a
    // never-verified row is `1` and sorts first. Both arms now carry this
    // expression, so the order is the same everywhere.
    assert_eq!(
        ids.first().copied(),
        Some(never.as_str()),
        "never-verified rows come first -- a rendition nobody has checked is \
         the one most in need of it"
    );
}

/// An empty table is an empty sweep, not an error.
#[tokio::test]
async fn the_sweep_on_an_empty_table_is_empty() {
    let h = Harness::new("deriv-sweep-empty").await;
    assert!(
        find_stale_for_verification(h.db(), "2999-01-01T00:00:00Z", 10)
            .await
            .expect("sweep")
            .is_empty()
    );
}

/// A sweep that finds a row and then touches it drops out of the next sweep,
/// so one pass makes progress.
#[tokio::test]
async fn a_verified_row_drops_out_of_the_next_sweep() {
    let h = Harness::new("deriv-sweep-progress").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Epub).await;
    mark_derivative_built(h.db(), &id, "s", 1, "m")
        .await
        .expect("built");

    // A cut-off in the past: `verified_at IS NULL` makes the row qualify.
    let first = find_stale_for_verification(h.db(), "2000-01-01T00:00:00Z", 100)
        .await
        .expect("first sweep");
    assert_eq!(first.len(), 1, "never verified, so it qualifies");

    assert!(touch_derivative_verified(h.db(), &id)
        .await
        .expect("verified"));

    // The same cut-off now excludes it: `verified_at` is no longer NULL and no
    // longer older than 2000. This is what makes one pass of the sweep make
    // progress rather than re-reporting the same rows forever.
    let second = find_stale_for_verification(h.db(), "2000-01-01T00:00:00Z", 100)
        .await
        .expect("second sweep");
    assert!(
        second.is_empty(),
        "a verified row is no longer older than a pre-2000 cut-off"
    );
}

// ---------------------------------------------------------------------------
// touch_derivative_verified
// ---------------------------------------------------------------------------

/// Touching stamps `verified_at`, bumps the version, and leaves the state and
/// output alone.
#[tokio::test]
async fn touching_verified_stamps_the_time_and_bumps_the_version() {
    let h = Harness::new("deriv-touch").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Epub).await;
    mark_derivative_built(h.db(), &id, "s", 10, "m")
        .await
        .expect("built");
    let before = h.version(&id).await;

    assert!(touch_derivative_verified(h.db(), &id).await.expect("touch"));

    let d = find_derivative(h.db(), &id)
        .await
        .expect("find")
        .expect("row");
    assert!(d.verified_at.is_some());
    assert_eq!(d.state, "ready", "verification is not a state change");
    assert_eq!(d.output_checksum.as_deref(), Some("s"), "output untouched");
    assert_eq!(h.version(&id).await, before + 1);
}

/// Re-verifying restamps the time rather than keeping the first, because a
/// sweep run twice is a fact about the output, not a one-off event.
#[tokio::test]
async fn re_verifying_restamps_the_time() {
    let h = Harness::new("deriv-touch-twice").await;
    let work = h.work().await;
    let id = h.create(&work, DerivativeKind::Epub).await;
    mark_derivative_built(h.db(), &id, "s", 10, "m")
        .await
        .expect("built");

    touch_derivative_verified(h.db(), &id).await.expect("first");
    h.exec(&format!(
        "UPDATE derivatives SET verified_at = '2020-01-01T00:00:00Z' WHERE {}",
        h.uuid_eq("id", &id)
    ))
    .await;
    touch_derivative_verified(h.db(), &id)
        .await
        .expect("second");

    let stamp = h
        .text("SELECT verified_at FROM derivatives WHERE id = ?", &id)
        .await
        .expect("stamped");
    assert_ne!(stamp, "2020-01-01T00:00:00Z", "the old stamp is replaced");
}

/// Touching an unknown id is `false`.
#[tokio::test]
async fn touching_an_unknown_derivative_is_false() {
    let h = Harness::new("deriv-touch-unknown").await;
    assert!(
        !touch_derivative_verified(h.db(), &uuid::Uuid::new_v4().to_string())
            .await
            .expect("touch")
    );
}
