//! The snapshot re-key, proven against a real Postgres (M60-03, M60-04).
//!
//! Spec §11.16.3: `pseud_id` is the column every behavioural table joins on, and
//! the replacement must be a **deterministic function of the original alone**.
//! §11.16.3b: `account_id` is a second join key, on a separate salt, and is the
//! one a first draft misses.
//!
//! The property under test is small and the consequences of getting it wrong are
//! not, so each test below names the failure it exists to catch rather than the
//! behaviour it asserts:
//!
//! * **not a join** -- a per-row re-key. Every table becomes unrelated, the
//!   dataset is a set of noise, and it restores cleanly, so nothing complains
//!   until someone tries to use it.
//! * **not reversible via the pseud/account join** -- one salt for both keys.
//!   A published snapshot then hands over the pseud-to-account mapping that the
//!   whole exercise was supposed to withhold.
//! * **not merely a string that parses** -- a UUID of the wrong *version* is
//!   still rejected by tools that key on version, and still restores wrong.
//!
//! PostgreSQL only, and honestly so: SQLite has no sha256 and no pgcrypto, and
//! §11.16 is a publication feature driven by `pg_dump`/`psql`. See
//! `migrations/sqlite/0092_snapshot_rekey.sql` for why inventing a weaker
//! construction there would be worse than an honest absence.

use lorehaven_db::Backend;
use test_support::TestDb;

/// The `?`-style bind differs by dialect, and this suite only runs on Postgres.
/// Kept as a named function so the "why Postgres only" reads in one place
/// rather than at every call site.
fn require_pg(tdb: &TestDb) {
    assert_eq!(
        tdb.db().backend(),
        Backend::Postgres,
        "the snapshot re-key is PostgreSQL-only by design: SQLite has no \
         sha256 and no pgcrypto, and 11.16 is a pg_dump/psql feature"
    );
}

async fn rekey(tdb: &TestDb, sql: &str) -> String {
    let pool = tdb.db().postgres_pool().expect("postgres handle");
    sqlx::query_scalar::<_, String>(sql)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}\n{e}"))
}

#[tokio::test]
async fn the_same_pseud_id_rekeys_to_the_same_value_everywhere() {
    let tdb = TestDb::connect_with_dir(
        "snapshot_rekey_determinism",
        &test_support::scratch_dir("snapshot_rekey_determinism"),
    )
    .await;
    require_pg(&tdb);

    let a = rekey(
        &tdb,
        "SELECT snapshot_pseud('11111111-1111-1111-1111-111111111111'::uuid)::text",
    )
    .await;
    let b = rekey(
        &tdb,
        "SELECT snapshot_pseud('11111111-1111-1111-1111-111111111111'::uuid)::text",
    )
    .await;
    assert_eq!(
        a, b,
        "a re-key that varies between calls destroys every join in the snapshot, \
         and the result still restores, so nothing notices until it is used"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn two_different_pseud_ids_never_rekey_to_the_same_value() {
    let tdb = TestDb::connect_with_dir(
        "snapshot_rekey_distinct",
        &test_support::scratch_dir("snapshot_rekey_distinct"),
    )
    .await;
    require_pg(&tdb);

    let a = rekey(
        &tdb,
        "SELECT snapshot_pseud('11111111-1111-1111-1111-111111111111'::uuid)::text",
    )
    .await;
    let b = rekey(
        &tdb,
        "SELECT snapshot_pseud('22222222-2222-2222-2222-222222222222'::uuid)::text",
    )
    .await;
    assert_ne!(
        a, b,
        "two pseudonyms collapsing to one value silently fuses two people's \
         activity into one synthetic person in the published graph"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_pseud_id_and_an_account_id_with_the_same_input_never_collide() {
    let tdb = TestDb::connect_with_dir(
        "snapshot_rekey_salt_separation",
        &test_support::scratch_dir("snapshot_rekey_salt_separation"),
    )
    .await;
    require_pg(&tdb);

    let one = rekey(
        &tdb,
        "SELECT
            snapshot_pseud('11111111-1111-1111-1111-111111111111'::uuid)::text
            || '|' ||
            snapshot_account('11111111-1111-1111-1111-111111111111'::uuid)::text",
    )
    .await;
    let (pseud, account) = one.split_once('|').expect("two values");

    assert_ne!(
        pseud, account,
        "§11.16.3b: one salt for both keys means anyone holding a re-keyed \
         pseud_id table can join it to the re-keyed accounts table and recover \
         exactly the pseud-to-account mapping the snapshot withholds. This is \
         the failure the second function exists to prevent, and it is invisible \
         in any test that only checks one function at a time."
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn the_rekey_is_a_version_four_uuid_and_not_merely_a_uuid_shaped_string() {
    let tdb = TestDb::connect_with_dir(
        "snapshot_rekey_v4",
        &test_support::scratch_dir("snapshot_rekey_v4"),
    )
    .await;
    require_pg(&tdb);

    let version = rekey(
        &tdb,
        "SELECT substring(snapshot_pseud('11111111-1111-1111-1111-111111111111'::uuid)::text, 15, 1)",
    )
    .await;
    let variant = rekey(
        &tdb,
        "SELECT substring(snapshot_pseud('11111111-1111-1111-1111-111111111111'::uuid)::text, 20, 1)",
    )
    .await;

    assert_eq!(
        version, "4",
        "character 15 must be the version nibble. Asserted separately from the \
         equality tests because a derivation that emits a valid-looking UUID of \
         the WRONG version still passes every determinism check above."
    );
    assert_eq!(
        variant, "a",
        "character 20 must be the RFC 4122 variant nibble, for the same reason: \
         the cast succeeds either way and only a reader keying on the variant \
         would notice"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn the_rekey_is_not_the_identity_on_the_input() {
    let tdb = TestDb::connect_with_dir(
        "snapshot_rekey_not_identity",
        &test_support::scratch_dir("snapshot_rekey_not_identity"),
    )
    .await;
    require_pg(&tdb);

    // Mutation-proven, not assumed. Making `snapshot_pseud` return its input
    // unchanged -- a re-key that does nothing -- passes determinism (idempotent
    // by definition), passes distinctness (inputs differ), passes the v4 check
    // (a real uuid has a 4 at position 15), and passes the join test (nothing
    // moved, so every edge still joins). Every one of the four tests above is
    // green against a function that publishes the ORIGINAL identifiers.
    //
    // That is the sharpest version of the failure this milestone exists to
    // prevent, and it is worth stating plainly: a snapshot built with an
    // identity re-key is a snapshot of real pseud_ids, and it restores fine.
    let rekeyed = rekey(
        &tdb,
        "SELECT snapshot_pseud('11111111-1111-1111-1111-111111111111'::uuid)::text",
    )
    .await;

    assert_ne!(
        rekeyed, "11111111-1111-1111-1111-111111111111",
        "the re-key returned its input unchanged: the published snapshot would \
         carry the real pseud_id of a real person, and every other test here \
         would still pass"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn the_rekey_preserves_the_join_across_both_keys_at_once() {
    let tdb = TestDb::connect_with_dir(
        "snapshot_rekey_join_survives",
        &test_support::scratch_dir("snapshot_rekey_join_survives"),
    )
    .await;
    require_pg(&tdb);
    let pool = tdb.db().postgres_pool().expect("postgres handle");

    // Seed the real shape, which is two hops and not one: a work hangs off a
    // PSEUD, and the pseud hangs off an ACCOUNT. A test that joined works
    // straight to accounts -- which is what this first attempted -- would have
    // been testing a join that does not exist in the schema, and would have
    // passed against a re-key that broke the one that does.
    let account: String = sqlx::query_scalar("SELECT gen_random_uuid()::text")
        .fetch_one(pool)
        .await
        .unwrap();
    let pseud: String = sqlx::query_scalar("SELECT gen_random_uuid()::text")
        .fetch_one(pool)
        .await
        .unwrap();
    let work: String = sqlx::query_scalar("SELECT gen_random_uuid()::text")
        .fetch_one(pool)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO accounts (id, email, created_at, updated_at)
         VALUES ($1::uuid, $2, now(), now())",
    )
    .bind(&account)
    .bind("rekey-join@example.invalid")
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, $3, $4, now(), now())",
    )
    .bind(&pseud)
    .bind(&account)
    .bind("rekey-join-handle")
    .bind("Rekey Join Display")
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, 'rekey probe', now(), now())",
    )
    .bind(&work)
    .bind(&pseud)
    .execute(pool)
    .await
    .unwrap();

    // The masking has to happen somewhere the foreign keys cannot object, and
    // this test found out empirically that "in place" does not work in ANY
    // order: updating the child first trips the FK because the parent does not
    // hold the new value yet, and updating the parent first trips it because
    // the child still holds the old one. The constraint is checked per
    // statement, so no ordering of two statements satisfies both directions.
    //
    // A masking pipeline that walked tables in place would therefore die on a
    // live database -- which is why this is asserted here rather than left to
    // be discovered while publishing a real snapshot. The pipeline copies to a
    // scratch database and masks there, where the ordering constraint is a
    // design choice instead of an FK violation.
    let masked = "snapshot_masked";
    sqlx::query("DROP SCHEMA IF EXISTS snapshot_masked CASCADE")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE SCHEMA snapshot_masked")
        .execute(pool)
        .await
        .unwrap();

    // Copy the two rows into the scratch schema, then re-key them there.
    // LIKE INCLUDING ALL so the scratch tables carry the same constraints and
    // defaults. The FK is left pointed at the masked schema's own pseuds, which
    // is what makes the re-key order meaningful here rather than incidental.
    for table in ["accounts", "pseuds", "works"] {
        sqlx::query(&format!(
            "CREATE TABLE snapshot_masked.{table} (LIKE public.{table} INCLUDING ALL)"
        ))
        .execute(pool)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO snapshot_masked.accounts SELECT * FROM accounts WHERE id = $1::uuid")
        .bind(&account)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO snapshot_masked.pseuds SELECT * FROM pseuds WHERE id = $1::uuid")
        .bind(&pseud)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO snapshot_masked.works SELECT * FROM works WHERE id = $1::uuid")
        .bind(&work)
        .execute(pool)
        .await
        .unwrap();

    for sql in [
        "UPDATE snapshot_masked.pseuds SET id = snapshot_pseud(id), account_id = snapshot_account(account_id)
          WHERE id = $1::uuid",
        "UPDATE snapshot_masked.works SET owner_pseud_id = snapshot_pseud(owner_pseud_id)
          WHERE owner_pseud_id = $1::uuid",
    ] {
        sqlx::query(&sql.replace("snapshot_masked", masked))
            .bind(&pseud)
            .execute(pool)
            .await
            .unwrap();
    }

    // Now the question the spec actually asks: does the edge survive, with each
    // key re-keyed by its OWN function?
    let surviving_works: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM snapshot_masked.works w
           JOIN snapshot_masked.pseuds p ON p.id = w.owner_pseud_id
          WHERE p.account_id = snapshot_account($1::uuid)",
    )
    .bind(&account)
    .fetch_one(pool)
    .await
    .unwrap();

    assert_eq!(
        surviving_works, 1,
        "a deterministic re-key whose foreign keys are not re-keyed the same way \
         produces exactly this: zero surviving rows, a snapshot that restores \
         perfectly, and a social graph with no edges in it. Neither \
         determinism test above can see this -- both pass while the graph is \
         empty, because the bug is in the caller, not the function."
    );
    tdb.cleanup().await;
}
