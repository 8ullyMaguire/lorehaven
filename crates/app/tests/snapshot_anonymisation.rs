//! M60-02 — the end-to-end leak test (spec §11.16.7).
//!
//! **It reads the DUMP BYTES, not a restored database, and that is the whole
//! design.** A test that queries the restored database cannot see a canary in
//! a `pg_dump` comment, and a comment is exactly where a real leak survives
//! review: `pg_dump` emits `-- Name: accounts; Type: TABLE` headers, setval
//! statements naming sequences, and `COPY` blocks. Anything the mask missed in
//! the *data* is caught by either method; anything missed in the *file* is
//! caught only by the byte scan.
//!
//! The second design point, learned by building this: **a leak test needs a
//! control.** `no canary in the dump` is satisfied just as well by a dump that
//! contains no data at all, by a pipeline that failed before it wrote anything,
//! and by a mask that dropped every table. So the first test here asserts the
//! UNMASKED dump *does* contain every canary, and only then asserts the masked
//! one contains none. Without the control, "0 found" is unfalsifiable.
//!
//! PostgreSQL only, by design: §11.16 is a `pg_dump`/`psql` publication feature
//! and SQLite has no sha256 (see `migrations/sqlite/0092_snapshot_rekey.sql`).

use lorehaven_db::Backend;
use test_support::TestDb;

/// Known-plaintext PII, one per identifying shape the spec names.
///
/// Deliberately greppable and deliberately *not* realistic. A canary shaped like
/// a real email is a canary whose absence a reader might doubt; `leak-canary@`
/// is a string nobody would write by accident, so a single occurrence anywhere
/// in the file is unambiguous.
const CANARY_EMAIL: &str = "leak-canary@example.invalid";
const CANARY_HANDLE: &str = "leak_canary_handle";
const CANARY_DISPLAY: &str = "leak_canary_display";
const CANARY_PASSWORD: &str = "leak-canary-passwordhash";
const CANARY_TOKEN: &str = "leak-canary-sessiontoken";

const CANARIES: &[&str] = &[
    CANARY_EMAIL,
    CANARY_HANDLE,
    CANARY_DISPLAY,
    CANARY_PASSWORD,
    CANARY_TOKEN,
];

fn pg_url() -> Option<String> {
    std::env::var("LOREHAVEN_TEST_PG_URL")
        .ok()
        .filter(|s| !s.is_empty())
}

async fn seed_canaries(tdb: &TestDb) {
    let pool = tdb.db().postgres_pool().expect("postgres handle");
    // Fixed uuids, so the re-key of each is predictable and the "the dump
    // contains the ORIGINAL id" check below is meaningful.
    let account = "11111111-1111-1111-1111-111111111111";
    let pseud = "22222222-2222-2222-2222-222222222222";

    sqlx::query(
        "INSERT INTO accounts (id, email, created_at, updated_at)
         VALUES ($1::uuid, $2, now(), now())",
    )
    .bind(account)
    .bind(CANARY_EMAIL)
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, $3, $4, now(), now())",
    )
    .bind(pseud)
    .bind(account)
    .bind(CANARY_HANDLE)
    .bind(CANARY_DISPLAY)
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO password_credentials (account_id, password_hash, algorithm, created_at, updated_at)
         VALUES ($1::uuid, $2, 'argon2id', now(), now())",
    )
    .bind(account)
    .bind(CANARY_PASSWORD)
    .execute(pool)
    .await
    .unwrap();

    // Column names read off information_schema, not assumed: the first version of
    // this inserted `updated_at` into `sessions`, which has `last_seen_at`, and
    // wrapped `id` in `::text` when it is a uuid. Both were caught by the seed
    // failing loudly rather than by the assertion passing on an empty table.
    sqlx::query(
        "INSERT INTO sessions (id, account_id, token_hash, csrf_token_hash,
                               user_agent, created_at, last_seen_at, expires_at)
         VALUES (gen_random_uuid(), $1::uuid, $2, $2, $2, now(), now(), now())",
    )
    .bind(account)
    .bind(CANARY_TOKEN)
    .execute(pool)
    .await
    .unwrap();
}

/// Build the masked schema IN this test's database, from the real generator.
///
/// The test runs the pipeline rather than a hand-written mask, on purpose. A
/// hand-written mask in the test and a generated mask in the script are two
/// implementations of the same spec, and the leak test would only be measuring
/// the hand-written one — so a regression in `build-snapshot-sql.py` would ship
/// a leak with a green suite. This is the difference between testing the
/// component and testing the thing a user would actually run.
async fn apply_mask(tdb: &TestDb) {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf();
    let script = repo.join("scripts").join("build-snapshot-sql.py");

    let out_dir = test_support::scratch_dir("snapshot_mask_sql");
    std::fs::create_dir_all(&out_dir).expect("scratch dir");
    let sql_path = out_dir.join("mask.sql");

    // The generator reads the live schema via psql, so it needs the URL.
    let status = std::process::Command::new("python3")
        .arg(&script)
        .arg("--out")
        .arg(&sql_path)
        .env(
            "LOREHAVEN_PG_URL",
            // The scratch database, not the template — same reason as in `dump`.
            format!(
                "postgres://postgres:{}@127.0.0.1:5432/{}",
                pg_password(),
                tdb.pg_database_name().expect("pg database name")
            ),
        )
        .current_dir(&repo)
        .output()
        .expect("run the generator");
    assert!(
        status.status.success(),
        "build-snapshot-sql.py failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );

    let sql = std::fs::read_to_string(&sql_path).expect("generated sql");
    assert!(
        sql.contains("CREATE SCHEMA IF NOT EXISTS snapshot_masked"),
        "the generator produced no masked schema"
    );

    sqlx::raw_sql(&sql)
        .execute(tdb.db().postgres_pool().expect("pool"))
        .await
        .expect("apply the generated mask");
}

fn dump(tdb: &TestDb, extra: &[&str], out: &std::path::Path) -> String {
    // The SCRATCH database, not the one named in the env var. `TestDb` creates a
    // uniquely-named database per test and the env URL's database name is only
    // the template; dumping the template name silently produced an empty dump of
    // a database that does not contain the canaries, which is precisely the
    // false pass the control test exists to prevent.
    let name = tdb
        .pg_database_name()
        .expect("a postgres scratch database name")
        .to_owned();
    let mut cmd = std::process::Command::new("pg_dump");
    cmd.args(["-h", "127.0.0.1", "-U", "postgres", "-d"])
        .arg(&name)
        .args(extra)
        .arg("-f")
        .arg(out);
    cmd.env("PGPASSWORD", pg_password());
    let status = cmd.output().expect("run pg_dump");
    assert!(
        status.status.success(),
        "pg_dump failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    std::fs::read_to_string(out).expect("read the dump")
}

fn pg_password() -> String {
    // The admin URL carries the password; psql/pg_dump need it separately.
    pg_url()
        .and_then(|u| {
            let (creds, _) = u.rsplit_once('@')?;
            creds.rsplit_once(':').map(|(_, pw)| pw.to_owned())
        })
        .unwrap_or_default()
}

/// The control: an UNMASKED dump contains every canary.
///
/// This is what makes the masked-dump assertion mean something. It is the
/// difference between "the mask works" and "the file was empty".
#[tokio::test]
async fn an_unmasked_dump_contains_every_canary_so_the_masked_one_means_something() {
    let Some(_) = pg_url() else { return };
    let tdb = TestDb::connect_with_dir(
        "snapshot_leak_control",
        &test_support::scratch_dir("snapshot_leak_control"),
    )
    .await;
    if tdb.db().backend() != Backend::Postgres {
        tdb.cleanup().await;
        return;
    }
    seed_canaries(&tdb).await;

    let dir = test_support::scratch_dir("snapshot_leak_control_out");
    let text = dump(&tdb, &[], &dir.join("full.sql"));

    for canary in CANARIES {
        assert!(
            text.contains(canary),
            "the control failed: {canary} is not in an UNMASKED dump of a \
             database that contains it. The leak test would then assert 'no \
             canary present' against a pipeline that never wrote any data, and \
             pass for the wrong reason."
        );
    }
    tdb.cleanup().await;
}

/// The property: the masked dump contains NO canary, anywhere in its bytes.
///
/// Includes the original `pseud_id` and `account_id` values, which are not PII
/// *as strings* but ARE the join to the re-keyed tables — §11.16.3b's failure,
/// where a dump has re-keyed `account_id` foreign keys and a raw `accounts.id`
/// for them to resolve against.
#[tokio::test]
async fn a_masked_dump_contains_no_canary_in_any_of_its_bytes() {
    let Some(_) = pg_url() else { return };
    let tdb = TestDb::connect_with_dir(
        "snapshot_leak_masked",
        &test_support::scratch_dir("snapshot_leak_masked"),
    )
    .await;
    if tdb.db().backend() != Backend::Postgres {
        tdb.cleanup().await;
        return;
    }
    seed_canaries(&tdb).await;
    apply_mask(&tdb).await;

    let dir = test_support::scratch_dir("snapshot_leak_masked_out");
    let text = dump(&tdb, &["-n", "snapshot_masked"], &dir.join("masked.sql"));

    for canary in CANARIES {
        assert!(!text.contains(canary), "the masked dump leaked {canary}");
    }
    for original_id in [
        "11111111-1111-1111-1111-111111111111",
        "22222222-2222-2222-2222-222222222222",
    ] {
        assert!(
            !text.contains(original_id),
            "the masked dump contains the ORIGINAL {original_id}. Either the \
             re-key was not applied, or it was applied to the foreign key and \
             not to the key it points at — the 11.16.3b failure, where every \
             behavioural column reads as pseudonymous and the identifying key is \
             still in the file."
        );
    }
    tdb.cleanup().await;
}

/// The masked dump is not empty. A dump with no rows satisfies "no canary" too.
#[tokio::test]
async fn a_masked_dump_still_carries_the_dataset() {
    let Some(_) = pg_url() else { return };
    let tdb = TestDb::connect_with_dir(
        "snapshot_leak_not_empty",
        &test_support::scratch_dir("snapshot_leak_not_empty"),
    )
    .await;
    if tdb.db().backend() != Backend::Postgres {
        tdb.cleanup().await;
        return;
    }
    seed_canaries(&tdb).await;
    apply_mask(&tdb).await;

    let dir = test_support::scratch_dir("snapshot_leak_not_empty_out");
    let text = dump(&tdb, &["-n", "snapshot_masked"], &dir.join("masked.sql"));

    assert!(
        text.contains("COPY snapshot_masked.pseuds"),
        "the masked dump has no pseudos COPY block. A dump that dropped every \
         table also contains no canaries, and this suite's other two tests would \
         pass for the wrong reason."
    );
    assert!(
        text.contains("CREATE TABLE snapshot_masked.pseuds"),
        "the masked dump has no pseuds TABLE. pg_dump emits a VIEW or a \
         MATERIALIZED VIEW as a definition with no rows, so a mask built from \
         either produces a file that contains no data -- and therefore no \
         canaries. The snapshot has to ship as TABLES."
    );
    assert!(
        text.len() > 2000,
        "the masked dump is suspiciously small ({} bytes) — check that the          generator emitted real projections rather than empty views",
        text.len()
    );
    tdb.cleanup().await;
}
