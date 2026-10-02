//! §52.2 and §52.3 — acceptance tests for the taste-leakage store.
//!
//! These exist to test MIGRATION 0110's enforcement as much as the store's, so
//! several of them deliberately bypass the store and write raw SQL. That is the
//! point: `record_payout` has no `attribution` parameter and always writes the
//! window inside which it was called, so nothing reachable through the store's
//! API can express §52.2's forbidden payout. The only way to know the rule holds
//! is to try to write the forbidden row.
//
//! Green on SQLite and PostgreSQL.

use lorehaven_db::taste_leakage as tl;
use lorehaven_domain::ids::{PseudId, WorkId};
use lorehaven_domain::leakage::{
    prose_without_precision, BatchWindow, OwnerResonance, OwnerResonanceLabel,
};
use test_support::{scratch_dir, TestDb};
use uuid::Uuid;

const T0: i64 = 1_767_225_600; // 2026-01-01
const T1: i64 = T0 + 7 * 86_400; // one week later: a §52.3 weekly round closes
const T2: i64 = T1 + 86_400;
const T3: i64 = T1 + 7 * 86_400;
const MID: i64 = T0 + 3 * 86_400; // squarely inside the first window

async fn scratch(tag: &str) -> TestDb {
    let dir = scratch_dir(tag);
    TestDb::connect_with_dir(tag, &dir).await
}

fn pseud(n: u8) -> PseudId {
    PseudId::from_uuid(Uuid::from_bytes([n; 16]))
}

fn work(n: u8) -> WorkId {
    WorkId::from_uuid(Uuid::from_bytes([n; 16]))
}

/// The account → pseud → work chain, so the payout and label FKs have targets.
///
/// Raw SQL rather than a content store, because §52.2's rules are about the payout
/// path and a fixture that drags in the whole work-creation flow would make a
/// failure here hard to read.
///
/// The chain is three deep because `works.owner_pseud_id` references `pseuds`,
/// which references `accounts` — so a fixture that inserts a work and invents a
/// pseud id fails on the FK, which is what the first version of this did.
async fn fixture_work(db: &TestDb, id: &WorkId, owner: PseudId) {
    let account = Uuid::from_bytes([7u8; 16]);
    let email = format!("leak-{}@example.invalid", account);
    match db.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            let pool = db.db().sqlite_pool().expect("sqlite pool");
            sqlx::query("INSERT INTO accounts (id, email, created_at, updated_at)                  VALUES (?1, ?2, ?3, ?3)")
                .bind(account.to_string())
                .bind(&email)
                .bind(ts(T0))
                .execute(pool)
                .await
                .expect("fixture account");
            sqlx::query("INSERT INTO pseuds (id, account_id, handle, display_name,                  created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)")
                .bind(owner.to_canonical_string())
                .bind(account.to_string())
                .bind(format!("leak{}", u8::from_be_bytes([owner.as_uuid().as_bytes()[0]])))
                .bind("Leak Fixture")
                .bind(ts(T0))
                .execute(pool)
                .await
                .expect("fixture pseud");
            // `generated_content_posture` is named explicitly rather than left to
            // a default. 0109's backfill filled the rows that existed when it ran,
            // but a fixture INSERT is a NEW row and gets no backfill -- and
            // PostgreSQL narrows the column to NOT NULL where SQLite cannot, so a
            // bare INSERT is portable only by SQLite's leniency. Naming it is what
            // makes this fixture honest on both engines.
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at, \
                 generated_content_posture) VALUES (?1, ?2, ?3, ?4, ?4, 'forbid')",
            )
            .bind(id.to_canonical_string())
            .bind(owner.to_canonical_string())
            .bind("A Work")
            .bind(ts(T0))
            .execute(pool)
            .await
            .expect("fixture work");
        }
        lorehaven_db::Backend::Postgres => {
            let pool = db.db().postgres_pool().expect("postgres pool");
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at) \
                 VALUES ($1::uuid, $2, $3, $3)",
            )
            .bind(account)
            .bind(&email)
            .bind(ts(T0))
            .execute(pool)
            .await
            .expect("fixture account");
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, \
                 created_at, updated_at) VALUES ($1::uuid, $2::uuid, $3, $4, $5, $5)",
            )
            .bind(owner.as_uuid())
            .bind(account)
            .bind(format!(
                "leak{}",
                u8::from_be_bytes([owner.as_uuid().as_bytes()[0]])
            ))
            .bind("Leak Fixture")
            .bind(ts(T0))
            .execute(pool)
            .await
            .expect("fixture pseud");
            sqlx::query(
                "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at, \
                 generated_content_posture) VALUES ($1::uuid, $2::uuid, $3, $4, $4, 'forbid')",
            )
            .bind(id.as_uuid())
            .bind(owner.as_uuid())
            .bind("A Work")
            .bind(ts(T0))
            .execute(pool)
            .await
            .expect("fixture work");
        }
    }
}

/// A timestamp in the format the schema actually stores.
///
/// The unix-seconds constants above are for the store API, which takes `i64` and
/// formats on the way in. Raw SQL has to do that formatting itself, and getting it
/// wrong is silent: the column is TEXT, so a wrong format is not a type error, it
/// is a string that compares differently.
fn ts(at: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(at)
        .expect("a timestamp in range")
        .format(&time::format_description::well_known::Rfc3339)
        .expect("RFC 3339 formats")
}

fn engine(db: &TestDb) -> &'static str {
    match db.db().backend() {
        lorehaven_db::Backend::Sqlite => "SQLite",
        lorehaven_db::Backend::Postgres => "PostgreSQL",
    }
}

// ── §52.2 Batched, instance-attributed payouts ───────────────────────────────

#[tokio::test]
async fn a_payout_inside_a_closed_window_is_accepted() {
    let db = scratch("pay-in-window").await;
    let w = work(1);
    fixture_work(&db, &w, pseud(1)).await;
    let window = tl::open_window(db.db(), T0).await.expect("window opens");
    tl::close_window(db.db(), window, T1)
        .await
        .expect("window closes");

    let id = tl::record_payout(db.db(), pseud(1), w, 50, window, MID)
        .await
        .expect("a payout inside its window is the normal case");

    let read_back = tl::get_payout(db.db(), id)
        .await
        .expect("read")
        .expect("the payout is there");
    assert_eq!(read_back.credits, 50);
    assert_eq!(read_back.window.opened_at, T0);
    assert_eq!(read_back.window.closed_at, Some(T1));
    // §52.2's actual predicate, on a row that came back out of the database.
    assert!(
        lorehaven_domain::leakage::payout_is_safe(&read_back),
        "a payout inside its own window passes §52.2 on {engine}",
        engine = engine(&db)
    );
}

#[tokio::test]
async fn a_payout_after_the_window_closes_is_refused() {
    // §52.2's finding: timing carries the signal even with no payload detail, so
    // the batch interval has to bound WHEN as well as WHETHER. Tested on raw SQL
    // because the store's callers pass a paid_at they computed themselves.
    let db = scratch("pay-after").await;
    let w = work(1);
    fixture_work(&db, &w, pseud(1)).await;
    let window = tl::open_window(db.db(), T0).await.expect("window opens");
    tl::close_window(db.db(), window, T1)
        .await
        .expect("window closes");

    let err = raw_payout(&db, pseud(1), w, 50, window, T2)
        .await
        .expect_err("a payout after its window closed is the correlation §52.2 forbids");
    assert!(
        err.contains("closed window") || err.contains("check_violation"),
        "the refusal should name §52.2's rule, got: {err}"
    );
}

#[tokio::test]
async fn a_payout_into_an_open_window_is_refused() {
    // No closed window to attribute it to, which is the whole failure mode.
    let db = scratch("pay-open").await;
    let w = work(1);
    fixture_work(&db, &w, pseud(1)).await;
    let window = tl::open_window(db.db(), T0).await.expect("window opens");

    let err = raw_payout(&db, pseud(1), w, 50, window, MID)
        .await
        .expect_err("an open window cannot receive a payout");
    assert!(
        err.contains("closed window") || err.contains("check_violation"),
        "got: {err}"
    );
}

#[tokio::test]
async fn the_store_itself_cannot_record_a_payout_into_an_open_window() {
    // The same rule reached through the public API. Worth having separately: it
    // proves the store does not paper over the schema, which is the failure mode
    // where a store catches an error and retries with a plausible timestamp.
    let db = scratch("store-open").await;
    let w = work(1);
    fixture_work(&db, &w, pseud(1)).await;
    let window = tl::open_window(db.db(), T0).await.expect("window opens");

    assert!(
        tl::record_payout(db.db(), pseud(1), w, 50, window, MID)
            .await
            .is_err(),
        "the store must surface the refusal rather than retrying with its own clock"
    );
}

#[tokio::test]
async fn only_the_instance_attribution_is_storable() {
    // §52.2: a payout saying "you were rated highly" makes one rating action at
    // one moment observable, which is a measurement of the lens. The column admits
    // no such value.
    let db = scratch("attrib").await;
    let w = work(1);
    fixture_work(&db, &w, pseud(1)).await;
    let window = tl::open_window(db.db(), T0).await.expect("window opens");
    tl::close_window(db.db(), window, T1)
        .await
        .expect("window closes");

    // Every value that is not 'instance' must be refused, not just the one I
    // happened to think of. Dropping the CHECK entirely left this test GREEN --
    // because the single value it tried ('rating') was the only thing standing
    // between the row and a leak, and the test never noticed the rule was gone.
    //
    // Each row below differs from the legal insert that follows it in ONE column
    // only -- `attribution` -- and shares its id shape, recipient, work, credits,
    // window and timestamp. That is what makes a refusal attributable to the CHECK:
    // an earlier version of this test used a `work_id` the fixture had never
    // created, so every insert failed on the FOREIGN KEY and the test stayed green
    // with the CHECK deleted entirely.
    for forbidden in [
        "rating",
        "rated",
        "curation",
        "bounty",
        "instance_rating",
        "Instance",
        "INSTANCE",
        "",
        " instance",
    ] {
        let bad = format!(
            "INSERT INTO taste_leakage_payouts
                 (id, recipient_pseud_id, work_id, credits, window_id, attribution, paid_at)
             VALUES ('{}', '{}', '{}', 50, '{}', '{forbidden}', '{}')",
            Uuid::new_v4(),
            pseud(1).to_canonical_string(),
            w.to_canonical_string(),
            window,
            // RFC 3339, NOT the unix seconds every other timestamp in this file
            // carries. The columns are TEXT, and the timing trigger compares them
            // as strings -- so a unix-seconds literal sorts after every RFC 3339
            // value and the trigger refuses the row for the wrong reason. That is
            // exactly how this test stayed green with the CHECK deleted: the
            // refusals it counted were the timing trigger's, not the CHECK's.
            ts(MID)
        );
        let outcome = raw_sql(&db, &bad).await;
        assert!(
            outcome.is_err(),
            "attribution {forbidden:?} must not be storable on {engine} (got: {outcome:?})",
            engine = engine(&db)
        );
    }

    // And the one legal value still works, so the assertion above cannot pass
    // because every insert fails.
    let ok = tl::record_payout(db.db(), pseud(1), w, 50, window, MID)
        .await
        .expect("'instance' is the storable attribution");
    assert!(tl::get_payout(db.db(), ok).await.expect("read").is_some());
}

#[tokio::test]
async fn at_most_one_window_is_open_at_a_time() {
    // §52.2: two open windows would let a payout be attributed to whichever the
    // reader picked, which defeats the point of attributing it to one.
    let db = scratch("one-open").await;
    tl::open_window(db.db(), T0)
        .await
        .expect("first window opens");
    assert!(
        tl::open_window(db.db(), T0).await.is_err(),
        "a second open window is refused"
    );
}

#[tokio::test]
async fn a_closed_window_cannot_be_re_closed() {
    // Otherwise every row attributed to it could be re-spanned after the fact.
    let db = scratch("reclose").await;
    let window = tl::open_window(db.db(), T0).await.expect("window opens");
    tl::close_window(db.db(), window, T1)
        .await
        .expect("window closes");
    assert!(
        tl::close_window(db.db(), window, T3).await.is_err(),
        "re-closing widens the interval every row in it claims to represent"
    );
}

#[tokio::test]
async fn a_payout_of_nothing_or_less_is_refused() {
    // Not in §52's prose, and that is the point: a payout of zero or a negative
    // one is not a *disclosure* risk, so no spec clause demanded it. It is here
    // because the column says `credits > 0`, and deleting that CHECK left the
    // suite green -- a constraint nothing tests is a constraint nobody has.
    let db = scratch("credits").await;
    let w = work(1);
    fixture_work(&db, &w, pseud(1)).await;
    let window = tl::open_window(db.db(), T0).await.expect("window opens");
    tl::close_window(db.db(), window, T1)
        .await
        .expect("window closes");

    for bad_credits in [0, -1, -50] {
        let sql = format!(
            "INSERT INTO taste_leakage_payouts
                 (id, recipient_pseud_id, work_id, credits, window_id, attribution, paid_at)
             VALUES ('{}', '{}', '{}', {bad_credits}, '{}', 'instance', '{}')",
            Uuid::new_v4(),
            pseud(1).to_canonical_string(),
            w.to_canonical_string(),
            window,
            ts(MID)
        );
        assert!(
            raw_sql(&db, &sql).await.is_err(),
            "credits = {bad_credits} must not be storable on {}",
            engine(&db)
        );
    }

    // And the smallest legal payout still works.
    tl::record_payout(db.db(), pseud(1), w, 1, window, MID)
        .await
        .expect("one credit is a payout");
}

#[tokio::test]
async fn an_author_can_hold_no_payout_at_all() {
    // §52.2: a payout answers a supply question, so paying an author must not
    // require a payout artifact to exist. Asserted because a NOT NULL or a join
    // that assumed one would quietly make this impossible later.
    let db = scratch("no-payout").await;
    let w = work(1);
    fixture_work(&db, &w, pseud(1)).await;
    assert!(
        tl::get_payout(db.db(), Uuid::new_v4())
            .await
            .expect("read")
            .is_none(),
        "an author with no payout reads as None, which is a legitimate state"
    );
}

// ── §52.3 The owner-visible resonance label ──────────────────────────────────

#[tokio::test]
async fn a_coarse_label_round_trips_and_never_stores_a_number() {
    let db = scratch("label").await;
    let w = work(1);
    fixture_work(&db, &w, pseud(1)).await;
    let window = tl::open_window(db.db(), T0).await.expect("window opens");
    tl::close_window(db.db(), window, T1)
        .await
        .expect("window closes");

    tl::set_resonance_label(db.db(), pseud(1), w, OwnerResonance::Noticed, window, T1)
        .await
        .expect("label written");

    let (label, computed_at, from) = tl::get_resonance_label(db.db(), w)
        .await
        .expect("read")
        .expect("the label is there");
    assert_eq!(label, OwnerResonance::Noticed);
    assert_eq!(computed_at, T1);
    assert_eq!(from, window, "the label records the batch it came from");
}

#[tokio::test]
async fn a_numeric_label_is_refused_by_the_schema() {
    // §52.3: a continuous value with visible precision is a probe however coarse
    // the units. The column stores only the four words, so a score cannot be
    // written at all.
    let db = scratch("numeric-label").await;
    let w = work(1);
    fixture_work(&db, &w, pseud(1)).await;
    let window = tl::open_window(db.db(), T0).await.expect("window opens");
    tl::close_window(db.db(), window, T1)
        .await
        .expect("window closes");

    let bad = format!(
        "INSERT INTO taste_leakage_resonance_labels
             (work_id, owner_pseud_id, label, computed_at, batch_window_id)
         VALUES ('{}', '{}', '0.82', '{}', '{}')",
        w.to_canonical_string(),
        pseud(1).to_canonical_string(),
        ts(T1),
        window
    );
    assert!(
        raw_sql(&db, &bad).await.is_err(),
        "a score cannot be stored as a label on either engine"
    );
}

#[tokio::test]
async fn a_label_written_before_the_batch_closed_reads_as_stale() {
    // §52.3's third clause. Computed inside a window that has not closed, so the
    // reading is provisional — and an author reading it as current would
    // over-read a movement that is three days old.
    let db = scratch("stale").await;
    let w = work(1);
    fixture_work(&db, &w, pseud(1)).await;
    let window = tl::open_window(db.db(), T0).await.expect("window opens");
    tl::set_resonance_label(db.db(), pseud(1), w, OwnerResonance::Steady, window, T0)
        .await
        .expect("label written");

    let (label, computed_at, _from) = tl::get_resonance_label(db.db(), w)
        .await
        .expect("read")
        .expect("the label is there");
    let stored = OwnerResonanceLabel {
        owner: pseud(1),
        work: w,
        label,
        computed_at,
    };
    let open = BatchWindow {
        opened_at: T0,
        closed_at: None,
    };
    assert!(
        stored.is_stale(&open),
        "an open window means the reading is provisional"
    );
    assert!(
        stored.display(&open).contains("last week"),
        "staleness is visible in the text, not a boolean a caller may drop"
    );

    // Closing the window does NOT make that reading current, and this is the part
    // worth pinning: the label was computed at T0 while the window ran to T1, so it
    // still describes a moment before the batch finished. Staleness is
    // `computed_at < closed_at`, not "the window has a close time" -- a batch that
    // has closed but whose label predates it is exactly the three-day-old movement
    // §52.3 says an author would over-read.
    tl::close_window(db.db(), window, T1)
        .await
        .expect("window closes");
    let closed = BatchWindow {
        opened_at: T0,
        closed_at: Some(T1),
    };
    assert!(
        stored.is_stale(&closed),
        "a label computed at T0 stays stale against a window that closed at T1"
    );

    // Re-computed at the close, it is current -- and that is what the weekly batch
    // is for.
    tl::set_resonance_label(db.db(), pseud(1), w, OwnerResonance::Steady, window, T1)
        .await
        .expect("label rewritten at the close");
    let (label, computed_at, from) = tl::get_resonance_label(db.db(), w)
        .await
        .expect("read")
        .expect("the label is there");
    let current = OwnerResonanceLabel {
        owner: pseud(1),
        work: w,
        label,
        computed_at,
    };
    assert!(!current.is_stale(&closed));
    assert_eq!(current.display(&closed), "steady");
    assert_eq!(from, window);
}

#[tokio::test]
async fn the_next_batch_replaces_the_previous_reading() {
    // §52.3's weekly cadence: the label is not per-work-once, it is re-derived
    // each round from that round's window.
    let db = scratch("next-batch").await;
    let w = work(1);
    fixture_work(&db, &w, pseud(1)).await;

    let first = tl::open_window(db.db(), T0).await.expect("window 1 opens");
    tl::close_window(db.db(), first, T1)
        .await
        .expect("window 1 closes");
    tl::set_resonance_label(db.db(), pseud(1), w, OwnerResonance::Quiet, first, T1)
        .await
        .expect("label from batch 1");

    let second = tl::open_window(db.db(), T1).await.expect("window 2 opens");
    tl::close_window(db.db(), second, T3)
        .await
        .expect("window 2 closes");
    tl::set_resonance_label(db.db(), pseud(1), w, OwnerResonance::Landing, second, T3)
        .await
        .expect("label from batch 2");

    let (label, computed_at, from) = tl::get_resonance_label(db.db(), w)
        .await
        .expect("read")
        .expect("one label per work");
    assert_eq!(label, OwnerResonance::Landing, "the newer batch wins");
    assert_eq!(computed_at, T3);
    assert_eq!(from, second, "and it names the batch it came from");

    let (_, latest) = tl::latest_window(db.db())
        .await
        .expect("read")
        .expect("a latest window");
    assert_eq!(latest.closed_at, Some(T3));
}

// ── The two rules, from the shape rather than the values ────────────────────

#[tokio::test]
async fn a_payout_row_cannot_carry_a_rating_reference() {
    // §52.2 enforced by the SCHEMA'S SHAPE. Read the column list rather than
    // trying to insert into a column that does not exist: the guarantee is that
    // there is nowhere to put one, and that is only checkable by looking.
    let db = scratch("no-rating-col").await;
    let cols = columns(&db, "taste_leakage_payouts").await;
    for forbidden in ["rating", "rater", "event_id", "score", "rating_id"] {
        assert!(
            !cols.iter().any(|c| c == forbidden),
            "taste_leakage_payouts must have no `{forbidden}` column, found: {cols:?}"
        );
    }
    // And the ones that must be there, so the assertion above cannot pass on an
    // empty or renamed table.
    for required in ["credits", "window_id", "attribution", "paid_at"] {
        assert!(
            cols.iter().any(|c| c == required),
            "taste_leakage_payouts is missing `{required}`, found: {cols:?}"
        );
    }
}

#[tokio::test]
async fn a_label_row_must_name_the_batch_it_came_from() {
    let db = scratch("label-batch-fk").await;
    let w = work(1);
    fixture_work(&db, &w, pseud(1)).await;
    let window = tl::open_window(db.db(), T0).await.expect("window opens");
    tl::close_window(db.db(), window, T1)
        .await
        .expect("window closes");

    // A label with no batch is a per-event update wearing a batch's name, so the
    // column is NOT NULL with a real foreign key.
    let cols = columns(&db, "taste_leakage_resonance_labels").await;
    assert!(cols.iter().any(|c| c == "batch_window_id"), "{cols:?}");
    assert!(cols.iter().any(|c| c == "computed_at"), "{cols:?}");
}

#[tokio::test]
async fn the_leakage_wording_rules_hold_on_real_rows() {
    // The §52.1 predicate, exercised against the wording §52.2 actually writes
    // into a payout attribution. It is the string a reader sees, so it is the
    // only place a number could enter.
    assert!(prose_without_precision(
        "paid by the instance in the weekly round"
    ));
    assert!(prose_without_precision(
        "standing bounties in this fandom pay promptly"
    ));
    assert!(!prose_without_precision("rated 0.82 by the instance"));
}

// ── helpers for the raw-SQL cases ───────────────────────────────────────────

/// Insert a payout row without going through the store, returning the error text.
async fn raw_payout(
    db: &TestDb,
    recipient: PseudId,
    work: WorkId,
    credits: i64,
    window: uuid::Uuid,
    paid_at: i64,
) -> Result<(), String> {
    let id = Uuid::new_v4();
    let paid_at = ts(paid_at);
    let sql = match db.db().backend() {
        lorehaven_db::Backend::Sqlite => format!(
            "INSERT INTO taste_leakage_payouts
                 (id, recipient_pseud_id, work_id, credits, window_id, attribution, paid_at)
             VALUES ('{id}', '{}', '{}', {credits}, '{window}', 'instance', '{paid_at}')",
            recipient.to_canonical_string(),
            work.to_canonical_string()
        ),
        lorehaven_db::Backend::Postgres => format!(
            "INSERT INTO taste_leakage_payouts
                 (id, recipient_pseud_id, work_id, credits, window_id, attribution, paid_at)
             VALUES ('{id}'::uuid, '{}'::uuid, '{}'::uuid, {credits}, '{window}'::uuid, 'instance', '{paid_at}')",
            recipient.to_canonical_string(),
            work.to_canonical_string()
        ),
    };
    raw_sql(db, &sql).await
}

/// Run arbitrary SQL, returning its error text rather than panicking.
async fn raw_sql(db: &TestDb, sql: &str) -> Result<(), String> {
    match db.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::raw_sql(sql)
            .execute(db.db().sqlite_pool().expect("sqlite pool"))
            .await
            .map(|_| ())
            .map_err(|e| e.to_string()),
        lorehaven_db::Backend::Postgres => sqlx::raw_sql(sql)
            .execute(db.db().postgres_pool().expect("postgres pool"))
            .await
            .map(|_| ())
            .map_err(|e| e.to_string()),
    }
}

/// A table's column names, so a test can assert a column is absent.
async fn columns(db: &TestDb, table: &str) -> Vec<String> {
    let rows: Vec<(String,)> = match db.db().backend() {
        lorehaven_db::Backend::Sqlite => sqlx::query_as("SELECT name FROM pragma_table_info(?1)")
            .bind(table)
            .fetch_all(db.db().sqlite_pool().expect("sqlite pool"))
            .await
            .expect("columns"),
        lorehaven_db::Backend::Postgres => sqlx::query_as(
            "SELECT column_name AS name FROM information_schema.columns \
             WHERE table_name = $1 ORDER BY ordinal_position",
        )
        .bind(table)
        .fetch_all(db.db().postgres_pool().expect("postgres pool"))
        .await
        .expect("columns"),
    };
    rows.into_iter().map(|(c,)| c).collect()
}
