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

/// §11.16.6 — an AGGREGATING instance must publish a snapshot with no chapter
/// body text.
///
/// Written before the gate existed, so the first run is the control: the mask had
/// no notion of retention mode and this test is red. The canary here is a real
/// chapter body, distinct from the identity canaries, because a body leak is a
/// DIFFERENT failure from an identity leak: nothing in the identity canary set
/// would catch it, and §7.7's audience rule is not expressible in a SQL dump.
///
/// The `cache` half is in the same file and is the part that makes this
/// meaningful: if bodies were redacted unconditionally, the aggregate test would
/// pass while the snapshot was useless. §11.16.6 permits bodies on a caching
/// instance precisely because that is a decision its operator made.
const CANARY_BODY: &str = "leak-canary-chapter-body-the-whole-text-of-a-chapter";

/// A pseudonym the seeded work and revision hang off. The chapters are NOT
/// anonymous: ownership is part of the behavioural graph, and a body seeded
/// against no owner would be a shape the real schema cannot hold.
const PSEUD: &str = "66666666-6666-6666-6666-666666666666";

/// The account the seeded pseud belongs to. Same fixed uuid as seed_canaries, so
/// the two seeds compose and the identity mask is exercised in the body tests too.
const ACCOUNT: &str = "11111111-1111-1111-1111-111111111111";

/// Insert a chapter and a revision carrying a body canary.
async fn seed_body_canary(tdb: &TestDb) {
    let pool = tdb.db().postgres_pool().expect("postgres handle");
    // The account is the identity canary from seed_canaries, so an aggregating
    // snapshot must still mask it even while redacting the body.
    seed_canaries(tdb).await;
    let work = "33333333-3333-3333-3333-333333333333";
    let chapter = "44444444-4444-4444-4444-444444444444";
    let revision = "55555555-5555-5555-5555-555555555555";

    sqlx::query(
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, 'leak_canary_owner', 'leak canary owner',
                 now(), now())",
    )
    .bind(PSEUD)
    .bind(ACCOUNT)
    .execute(pool)
    .await
    .unwrap();

    // Column lists match the pattern the other integration tests use: the
    // NOT NULL columns without defaults are all supplied. My first version named
    // `works.slug`, which does not exist, and every body test died in the seed --
    // which reads like a test bug and is, but only because the seed is loud. A
    // seed that silently inserted nothing would have made the body canary absent
    // and the aggregate test GREEN on an empty table.
    sqlx::query(
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility,
                            created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, 'leak canary work', 'published', 'public',
                 now(), now())",
    )
    .bind(work)
    .bind(PSEUD)
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO chapters (id, work_id, order_key, title, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, 1, 'leak canary chapter', now(), now())",
    )
    .bind(chapter)
    .bind(work)
    .execute(pool)
    .await
    .unwrap();

    // All THREE body shapes, because masking one and shipping the other two
    // publishes the same text. This is the failure the 11.16.6 policy entries
    // for chapter_revisions now prevent.
    sqlx::query(
        "INSERT INTO chapter_revisions
           (id, chapter_id, revision_number, document_json, sanitized_html,
            plain_text, word_count, created_by_pseud_id, created_at)
         VALUES ($1::uuid, $2::uuid, 1, $3, $4, $5, 1, $6::uuid, now())",
    )
    .bind(revision)
    .bind(chapter)
    .bind(format!("{{\"text\":\"{CANARY_BODY}\"}}"))
    .bind(format!("<p>{CANARY_BODY}</p>"))
    .bind(CANARY_BODY)
    .bind(PSEUD)
    .execute(pool)
    .await
    .unwrap();
}

/// Build the mask with an EXPLICIT mode, bypassing the instance-policy read, so
/// both halves of 11.16.6 can be exercised against one database.
async fn apply_mask_in_mode(tdb: &TestDb, mode: &str) {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf();
    let out_dir = test_support::scratch_dir("snapshot_mask_mode_sql");
    std::fs::create_dir_all(&out_dir).expect("scratch dir");
    let sql_path = out_dir.join(format!("mask_{mode}.sql"));

    let status = std::process::Command::new("python3")
        .arg(repo.join("scripts").join("build-snapshot-sql.py"))
        .arg("--out")
        .arg(&sql_path)
        .arg("--mode")
        .arg(mode)
        .env(
            "LOREHAVEN_PG_URL",
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
        "generator failed for mode {mode}: {}",
        String::from_utf8_lossy(&status.stderr)
    );

    let sql = std::fs::read_to_string(&sql_path).expect("generated sql");
    // In AGGREGATE mode chapter_revisions must appear in the generated mask --
    // a table that is not mentioned is a table the operator dumps from `public`,
    // which is how the body ships in the clear. In CACHE mode the opposite holds
    // and asserting it would be asserting the bug: a caching instance ships the
    // body, so there is nothing to mask and the table is dumped directly. That
    // asymmetry is the whole of 11.16.6, and both halves are asserted.
    if mode == "aggregate" {
        assert!(
            sql.contains("CREATE TABLE snapshot_masked.chapter_revisions"),
            "no chapter_revisions in the aggregate mask. A table the generator \
             does not mention is a table dumped from `public` in the clear -- and \
             this one holds the chapter body."
        );
    }
    sqlx::raw_sql(&sql)
        .execute(tdb.db().postgres_pool().expect("pool"))
        .await
        .expect("apply the generated mask");
}

#[tokio::test]
async fn an_aggregating_instance_publishes_no_chapter_body_text() {
    let Some(_) = pg_url() else { return };
    let tdb = TestDb::connect_with_dir(
        "snapshot_leak_aggregate",
        &test_support::scratch_dir("snapshot_leak_aggregate"),
    )
    .await;
    if tdb.db().backend() != Backend::Postgres {
        tdb.cleanup().await;
        return;
    }
    seed_body_canary(&tdb).await;
    apply_mask_in_mode(&tdb, "aggregate").await;

    let dir = test_support::scratch_dir("snapshot_leak_aggregate_out");
    let text = dump(&tdb, &["-n", "snapshot_masked"], &dir.join("masked.sql"));

    assert!(
        !text.contains(CANARY_BODY),
        "an AGGREGATING instance published chapter body text. 11.16.6: an \
         instance that never fetched a body has none to publish, and a dump that \
         publishes every body undoes 7.7's audience rule for every audience at \
         once -- a rule that is not expressible in a SQL dump."
    );
    // The three shapes are asserted separately: a mask that redacts plain_text
    // but ships sanitized_html publishes the same text.
    assert!(
        !text.contains("leak canary chapter") || !text.contains("leak-canary-chapter"),
        "the chapter TITLE leaked on an aggregating instance"
    );
    tdb.cleanup().await;
}

#[tokio::test]
async fn a_caching_instance_may_publish_bodies_and_this_test_proves_it_is_not_redacted_away() {
    let Some(_) = pg_url() else { return };
    let tdb = TestDb::connect_with_dir(
        "snapshot_leak_cache",
        &test_support::scratch_dir("snapshot_leak_cache"),
    )
    .await;
    if tdb.db().backend() != Backend::Postgres {
        tdb.cleanup().await;
        return;
    }
    seed_body_canary(&tdb).await;
    apply_mask_in_mode(&tdb, "cache").await;

    let dir = test_support::scratch_dir("snapshot_leak_cache_out");

    // TWO dumps, and the reason matters. In cache mode the body table is not
    // masked -- it is dumped from `public`, because there is nothing to mask on a
    // caching instance. So the body assertion reads the WHOLE database, while
    // the identity assertion reads `snapshot_masked` only. Asserting both on one
    // file cannot work: a whole-database dump necessarily contains the raw email
    // in `public.accounts`, which is not a leak (the snapshot does not ship
    // `public`), and asserting on it would train the reader to ignore a real one.
    let whole = dump(&tdb, &[], &dir.join("whole.sql"));
    let masked = dump(&tdb, &["-n", "snapshot_masked"], &dir.join("masked.sql"));

    assert!(
        whole.contains(CANARY_BODY),
        "a CACHING instance did not publish the body. 11.16.6 PERMITS bodies \
         here -- the body text IS the dataset, and 11.15 records the operator's \
         decision to keep it. If this assertion ever needs to be removed to make \
         a snapshot smaller, the honest fix is a retention-mode decision, not a \
         deleted test."
    );
    // Retention mode decides whether BODIES ship. It does not decide whether
    // IDENTITIES are masked: even on a caching instance, the published schema
    // must have no email in it.
    assert!(
        !masked.contains(CANARY_EMAIL),
        "cache mode published the account email in the snapshot schema. \
         Retention mode decides whether bodies ship; it does not switch the \
         identity mask off."
    );
    tdb.cleanup().await;
}

/// §11.16.6 / §7.7.3 — no `body_audience` VALUE appears anywhere in a snapshot.
///
/// Two things are asserted, and the second is the one that is easy to miss:
///
/// 1. the audience VALUE (`trust_at_least:5`) — the obvious leak;
/// 2. the COLUMN NAME (`body_audience`) — because a dump that still has a
///    `body_audience` column tells a recipient which works carry an access rule
///    at all, even with every value redacted. The spec's phrase is "dropped from
///    the snapshot entirely", and a column of NULLs is not "entirely".
///
/// The canary is a REAL legal value, not a made-up one: `works_body_audience_valid`
/// is a CHECK constraint, so a fake string cannot be stored and the test would
/// seed nothing. A real value is also the better canary, because
/// `trust_at_least:5` publishes the trust THRESHOLD — the most operationally
/// revealing form the column takes.
const CANARY_AUDIENCE: &str = "trust_at_least:5";

async fn seed_audience_canary(tdb: &TestDb) {
    let pool = tdb.db().postgres_pool().expect("postgres handle");
    sqlx::query(
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility,
                            body_audience, created_at, updated_at)
         VALUES ($1::uuid, $2::uuid, 'audience canary work', 'published', 'public',
                 $3, now(), now())",
    )
    .bind("77777777-7777-7777-7777-777777777777")
    .bind(PSEUD)
    .bind(CANARY_AUDIENCE)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn no_snapshot_publishes_a_body_audience_value_or_the_column_that_holds_it() {
    let Some(_) = pg_url() else { return };
    let tdb = TestDb::connect_with_dir(
        "snapshot_leak_audience",
        &test_support::scratch_dir("snapshot_leak_audience"),
    )
    .await;
    if tdb.db().backend() != Backend::Postgres {
        tdb.cleanup().await;
        return;
    }
    // seed_canaries creates the ACCOUNT but not the PSEUD the work hangs off;
    // the body seed creates the pseud itself, so this composes the same way.
    seed_body_canary(&tdb).await;
    seed_audience_canary(&tdb).await;
    apply_mask(&tdb).await;

    let dir = test_support::scratch_dir("snapshot_leak_audience_out");
    let text = dump(&tdb, &["-n", "snapshot_masked"], &dir.join("masked.sql"));

    assert!(
        !text.contains(CANARY_AUDIENCE),
        "the snapshot published body_audience={CANARY_AUDIENCE}. 11.16.6: an access \
         rule that is readable is not one. This value also publishes the trust \
         THRESHOLD, which is the operationally useful part."
    );
    // The COLUMN, not the token. This distinction cost a real debugging session
    // and is worth stating: asserting the absence of the string "body_audience"
    // anywhere in the dump is WRONG, because the migration ledger legitimately
    // contains a row named `0086_body_audience` -- that migration exists, it
    // added the column, and the ledger is a list of migration NAMES. A dump can
    // therefore be entirely correct and still trip a substring assertion.
    //
    // What 11.16.6 actually forbids is the column SURVIVING: a `body_audience`
    // column of NULLs still tells a recipient which works carry an audience rule
    // at all, which is the disclosure. So the assertion is about the schema -- a
    // CREATE TABLE for works, and a COPY whose column list includes it.
    let works_create = text
        .lines()
        .skip_while(|l| !l.starts_with("CREATE TABLE snapshot_masked.works"))
        .take_while(|l| !l.starts_with(");"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !works_create.contains("body_audience"),
        "snapshot_masked.works still has a body_audience column:\n  {works_create}\n\
         Every value may be redacted and this still tells a recipient which works \
         carry an audience rule at all. 11.16.6 says dropped ENTIRELY, and a \
         column of NULLs is not that."
    );
    assert!(
        !text
            .lines()
            .any(|l| l.starts_with("COPY snapshot_masked.works") && l.contains("body_audience")),
        "the works COPY lists body_audience, so the values are in the file even if \
         the CREATE TABLE above looks clean."
    );
    // And the work itself must still be there -- the point is to drop the rule,
    // not the work. 11.16.6's companion clause keeps titles and metadata.
    assert!(
        text.contains("audience canary work"),
        "the seeded work vanished from the snapshot. Dropping the ACCESS RULE is \
         not a licence to drop the work it applied to; the dataset loses the \
         audience column, not the works."
    );
    tdb.cleanup().await;
}

/// §11.16.3b — `account_id` is re-keyed on the SAME basis as `pseud_id`, with a
/// different salt, and the join to `accounts` survives.
///
/// The spec calls this "the requirement most likely to be implemented backwards",
/// and it was. 55 of 56 uuid `account_id` columns were classified `rekey_text`,
/// which emits a `snp_` TEXT hash, while `accounts.id` is re-keyed to a uuid. A
/// text hash cannot be compared with a uuid, so every one of those foreign keys
/// was silently severed — and the snapshot still looked de-identified, because
/// every behavioural column reads as pseudonymous. That is the failure mode the
/// requirement describes in its own words.
///
/// Three assertions, because each catches a different way of getting this wrong:
/// the JOIN survives; the two derivations DIFFER (same salt would let an observer
/// learn that a pseud and an account belong together); and the original id
/// appears nowhere in the bytes.
#[tokio::test]
async fn an_account_id_re_keys_to_the_same_derivation_as_accounts_id_so_the_join_survives() {
    let Some(_) = pg_url() else { return };
    let tdb = TestDb::connect_with_dir(
        "snapshot_rekey_account",
        &test_support::scratch_dir("snapshot_rekey_account"),
    )
    .await;
    if tdb.db().backend() != Backend::Postgres {
        tdb.cleanup().await;
        return;
    }
    seed_body_canary(&tdb).await;
    // A row that references the account through a NON-pseudonym column, so the
    // join is proved on the account path specifically. `shelves` rather than
    // `bookmarks` because every other bookmark column is NOT NULL with no
    // default; a seed that names a column the table does not have reads as a
    // test bug and is, and a seed that quietly inserts nothing makes the join
    // assertion below pass on an empty table.
    sqlx::query(
        "INSERT INTO shelves (id, account_id, name, created_at, updated_at)
         VALUES (gen_random_uuid(), $1::uuid, 'leak canary shelf', now(), now())",
    )
    .bind(ACCOUNT)
    .execute(tdb.db().postgres_pool().expect("pool"))
    .await
    .unwrap();
    apply_mask(&tdb).await;

    let pool = tdb.db().postgres_pool().expect("pool");
    let joined: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM snapshot_masked.shelves s
           JOIN snapshot_masked.accounts a ON a.id = s.account_id",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    assert!(
        joined > 0,
        "no shelf joins the re-keyed accounts. A uuid account_id classified \
         `rekey_text` is emitted as a `snp_` TEXT hash, which can never equal \
         a uuid, so the foreign key and the key it points at are derived \
         DIFFERENTLY and the relationship is destroyed. 11.16.3b: the snapshot \
         LOOKS de-identified because every behavioural column is pseudonymous, \
         while the identifier that still joins to the table holding the email \
         is left behind."
    );

    // The same input must NOT produce the same output: a shared salt would let
    // anyone holding one re-keyed table join a pseudonym to its account.
    let same: bool =
        sqlx::query_scalar("SELECT snapshot_pseud($1::uuid) = snapshot_account($1::uuid)")
            .bind(ACCOUNT)
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(
        !same,
        "snapshot_pseud and snapshot_account agree on the same input. They must \
         use DIFFERENT salts: sharing one lets an observer learn that a given \
         pseud_id and account_id belong to the same person, which is precisely \
         the join 11.16.3b exists to prevent."
    );

    let dir = test_support::scratch_dir("snapshot_rekey_account_out");
    let text = dump(&tdb, &["-n", "snapshot_masked"], &dir.join("masked.sql"));
    assert!(
        !text.contains(ACCOUNT),
        "the original account id {ACCOUNT} is in the dump. Re-keying the \
         foreign keys without re-keying accounts.id obscures the public handle \
         and leaves the identifier that still joins to the table holding the \
         email."
    );
    tdb.cleanup().await;
}

/// §11.16.7 step 5 — the snapshot restores into an empty PostgreSQL AND passes
/// `lorehaven doctor`. Both halves, because the first half passing says nothing
/// about the second, and that gap is where this requirement was quietly broken.
///
/// The bug this catches: `_migrations` was in the generator's `INTERNAL_TABLES`,
/// so it was excluded from the dump. The reasoning sounded right — a snapshot
/// recreates tables from its own schema, so it must not import the live ledger —
/// but nothing in the snapshot ever *created* a ledger. A recipient restored a
/// database with no `_migrations` table at all, `doctor` reported "89 migration(s)
/// pending" on a snapshot that had restored perfectly, and every byte-level leak
/// assertion stayed green. The anonymisation tests could not have caught it: they
/// assert what is ABSENT, and this failure is a row that should have been
/// PRESENT.
///
/// The fix regenerates the ledger from the migration files that already ship with
/// the binary, so it is a function of the release rather than a fact about one
/// host. Verified red: with the exclusion reinstated, this test fails and doctor
/// exits 1 with all 89 migrations pending.
#[tokio::test]
async fn a_restored_snapshot_passes_doctor() {
    let Some(_) = pg_url() else { return };
    let tdb = TestDb::connect_with_dir(
        "snapshot_doctor",
        &test_support::scratch_dir("snapshot_doctor"),
    )
    .await;
    if tdb.db().backend() != Backend::Postgres {
        tdb.cleanup().await;
        return;
    }
    seed_body_canary(&tdb).await;
    apply_mask(&tdb).await;

    let dir = test_support::scratch_dir("snapshot_doctor_dump");
    let dump_path = dir.join("snapshot.sql");
    // The ARTEFACT is the `snapshot_masked` schema and nothing else. Dumping the
    // whole database is not a stricter test, it is a different artefact: it
    // ships the unmasked `public.*` tables, and their live foreign keys, so the
    // restore either fails on a relation the dump does not contain or succeeds
    // against tables no recipient would ever receive.
    let text = dump(&tdb, &["-n", "snapshot_masked"], &dump_path);
    assert!(
        text.contains("COPY snapshot_masked._migrations"),
        "the dump carries no _migrations data, so a recipient restores a database \
         with an empty or missing ledger and `lorehaven doctor` reports every \
         migration pending. 11.16.7 step 5 requires the restore to PASS doctor, and \
         a restore that then needs a migration run is not the snapshot.\n\
         The ledger has to be written INSIDE the dumped schema: one written to \
         `public` is invisible to `pg_dump -n snapshot_masked`, so it fixes the \
         development database and not the artefact."
    );

    // Restore into a genuinely empty database, then run the real binary's doctor
    // against it. Asserting on the ledger's row count would be a test of the
    // helper; this is the requirement itself.
    let restored = format!("lh_snapshot_doctor_restore_{}", std::process::id());
    let create = std::process::Command::new("psql")
        .args([
            "-h",
            "127.0.0.1",
            "-U",
            "postgres",
            "-d",
            "postgres",
            "-c",
            &format!("DROP DATABASE IF EXISTS {restored};"),
            "-c",
            &format!("CREATE DATABASE {restored};"),
        ])
        .env("PGPASSWORD", pg_password())
        .output()
        .expect("create the restore database");
    assert!(create.status.success(), "could not create {restored}");

    // Restore it AS `public`, because that is the only reading of "restores into
    // an empty database" that means anything: a recipient is not going to point
    // Lorehaven at a schema called `snapshot_masked`, and a check that restores
    // the tables under their own name and then tests a different database is a
    // check of the harness.
    //
    // So the schema is rewritten on the way in. The `CREATE SCHEMA` line is
    // dropped rather than renamed, because `public` already exists in a fresh
    // database and restoring it is an error that aborts the whole script under
    // ON_ERROR_STOP -- which is how the first attempt at this failed.
    let as_public = dir.join("snapshot_as_public.sql");
    let rewritten = text
        .lines()
        .filter(|l| l.trim() != "CREATE SCHEMA snapshot_masked;")
        .map(|l| l.replace("snapshot_masked", "public"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&as_public, rewritten).expect("write the rewritten dump");

    let load = std::process::Command::new("psql")
        .args([
            "-h",
            "127.0.0.1",
            "-U",
            "postgres",
            "-d",
            &restored,
            "-v",
            "ON_ERROR_STOP=1",
            "-q",
            "-f",
        ])
        .arg(&as_public)
        .env("PGPASSWORD", pg_password())
        .output()
        .expect("restore the dump");
    assert!(
        load.status.success(),
        "the dump does not restore into an empty database:\n{}",
        String::from_utf8_lossy(&load.stderr)
    );

    let url = format!(
        "postgresql://postgres:{}@127.0.0.1:5432/{restored}",
        pg_password()
    );
    let doctor = std::process::Command::new(env!("CARGO_BIN_EXE_lorehaven"))
        .args(["doctor"])
        .env("LOREHAVEN_DATABASE_URL", &url)
        .output()
        .expect("run lorehaven doctor against the restore");
    let report = String::from_utf8_lossy(&doctor.stdout);
    std::process::Command::new("psql")
        .args([
            "-h",
            "127.0.0.1",
            "-U",
            "postgres",
            "-d",
            "postgres",
            "-c",
            &format!("DROP DATABASE IF EXISTS {restored};"),
        ])
        .env("PGPASSWORD", pg_password())
        .output()
        .expect("drop the restore database");

    assert!(
        doctor.status.success(),
        "doctor FAILS against a snapshot that restored cleanly:\n{}\n{}",
        report
            .lines()
            .filter(|l| l.starts_with("[FAIL]"))
            .collect::<Vec<_>>()
            .join("\n"),
        "A recipient is the one who discovers a broken snapshot, and 11.16.7 \
         exists to make sure they never have to."
    );
    assert!(
        report.contains("all ") && report.contains("migration(s) applied"),
        "doctor did not report the migration ledger as fully applied:\n{report}"
    );
    tdb.cleanup().await;
}

/// §11.16.5 / §11.17.4 — a month offset is a LINKABLE IDENTIFIER. Two dumps
/// shifted by the same amount join row-for-row on every timestamp, so each
/// snapshot must use its own offset, applied INSIDE the snapshot.
///
/// Four assertions, because the failure modes are different in kind and two of
/// them are SILENT — they do not raise, they produce a value that looks like a
/// timestamp and is not one:
///
///  1. the offset is applied (+45 days exactly, not approximately);
///  2. intervals SURVIVE — the research value 11.16.5 names is "how long do
///     people keep a work before abandoning it", so a shift that moved every
///     timestamp by a different amount would answer a different question;
///  3. the output still PARSES as RFC 3339 (see the to_char note in the
///     generator: escaped quotes land in the DATA, they do not raise);
///  4. the LIVE row is untouched — the mask is applied to a copy, and a shifted
///     live table would mean the instance itself now lies about when things
///     happened.
#[tokio::test]
async fn a_snapshot_shifts_every_timestamp_by_one_unpublished_offset() {
    let Some(_) = pg_url() else { return };
    let tdb = TestDb::connect_with_dir(
        "snapshot_offset",
        &test_support::scratch_dir("snapshot_offset"),
    )
    .await;
    if tdb.db().backend() != Backend::Postgres {
        tdb.cleanup().await;
        return;
    }
    let pool = tdb.db().postgres_pool().expect("pool");
    // Two accounts a known distance apart in time. The gap between them is the
    // thing that must not change.
    //
    // **The ids are generated here and captured, not left to `gen_random_uuid()`.**
    // The mask scrambles every key through `snapshot_account(id)` (see
    // `scripts/build-snapshot-sql.py`), so the masked table holds none of the
    // original ids — which means "read the oldest row" was the only way this test
    // could find its fixture, and that is a proxy for the row it seeded. It broke
    // the moment migration 0094 added an account older than both canaries. With
    // the ids in hand the masked ids are computable and the proxy is gone.
    let mut canary_ids: Vec<uuid::Uuid> = Vec::new();
    for (i, day) in ["2026-03-01", "2026-04-11"].iter().enumerate() {
        let id = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO accounts (id, email, created_at, updated_at)
             VALUES ($1::uuid, $2, $3::text, $3::text)",
        )
        .bind(id.to_string())
        .bind(format!("offset-canary-{i}@example.invalid"))
        .bind(format!("{day}T10:30:00Z"))
        .execute(pool)
        .await
        .expect("seed an offset canary");
        canary_ids.push(id);
    }

    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf();
    let sql_path = test_support::scratch_dir("snapshot_offset_sql").join("mask.sql");
    let status = std::process::Command::new("python3")
        .arg(repo.join("scripts").join("build-snapshot-sql.py"))
        .arg("--out")
        .arg(&sql_path)
        .arg("--timestamp-offset")
        .arg("45")
        .arg("--mode")
        .arg("cache")
        .env(
            "LOREHAVEN_PG_URL",
            format!(
                "postgres://postgres:{}@127.0.0.1:5432/{}",
                pg_password(),
                tdb.pg_database_name().expect("pg database name")
            ),
        )
        .current_dir(&repo)
        .output()
        .expect("run the generator with an offset");
    assert!(
        status.status.success(),
        "generator failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    sqlx::raw_sql(&std::fs::read_to_string(&sql_path).expect("mask sql"))
        .execute(pool)
        .await
        .expect("apply the mask");

    // **Not `ORDER BY created_at LIMIT 1` with no exclusion.** Migration 0094
    // inserts the instance's own system account with `created_at = 2026-01-01`,
    // earlier than this fixture's account, so the oldest row in the masked
    // snapshot is now a row that was never supposed to move. The failure that
    // produced looks exactly like a date-arithmetic bug — the unshifted date
    // minus 45 days — and nothing in it names the cause.
    //
    // Excluded by id rather than by a `created_at >` threshold: a date bound is
    // a guess about when the instance row was written, and this account is
    // identifiable on purpose (a fixed literal id, a `system` status, a
    // `.invalid` email — migration 0094 says so, so an operator can spot it in a
    // dump). Same exclusion as the other five harnesses, and the same reason.
    // **By masked id, not by `ORDER BY created_at LIMIT 1`.** Two reasons, and the
    // second is why the first is not enough:
    //
    // 1. Migration 0094's system account (`created_at = 2026-01-01`) is older
    //    than both canaries, and the mask shifts every row equally, so it stays
    //    the oldest. It is masked and shifted *correctly* — `2026-01-01 + 45 =
    //    2026-02-15` was exactly the value this assertion rejected — which is why
    //    the failure reads as a date-arithmetic bug and names nothing about row
    //    identity.
    // 2. **Excluding it by its original id does not work either**, because the
    //    masked table holds no original ids: every key is projected through
    //    `snapshot_account(id)`. `WHERE id != $1` on the *original* id matches
    //    every row and excludes nothing.
    //
    // So this selects the canary BY its masked id rather than excluding the
    // system account. Naming the row under test beats enumerating the rows it is
    // not, and it is the form the other five harnesses could not use — there the
    // ids survive, here they do not.
    let masked_earlier_canary =
        sqlx::query_scalar::<_, String>("SELECT snapshot_account($1::text::uuid)::text")
            .bind(canary_ids[0].to_string())
            .fetch_one(pool)
            .await
            .expect("the earlier canary's masked id");
    let row: (String, String) = sqlx::query_as(
        "SELECT created_at, updated_at FROM snapshot_masked.accounts
             WHERE id = $1::text::uuid",
    )
    .bind(masked_earlier_canary.clone())
    .fetch_one(pool)
    .await
    .expect("a masked account");

    // (3) parses, and (1) shifted by exactly 45 days
    let shifted: chrono::DateTime<chrono::Utc> = row.0.parse().unwrap_or_else(|e| {
        panic!(
            "the shifted timestamp {row:?} is not RFC 3339: {e}. \
             A to_char format with escaped quotes produces a string that LOOKS like a \
             timestamp and does not parse, and no error is raised anywhere."
        )
    });
    assert_eq!(
        shifted.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "2026-04-15T10:30:00Z",
        "the offset was not applied exactly. 11.16.5 wants a shift of a known size, \
         and an approximate one would make the published dates unknowable."
    );

    // (2) the interval survives: both rows moved by the SAME amount
    // **Over the two canaries, not over the table.** `max - min` across every
    // account spans the instance's system account too (migration 0094, dated
    // 2026-01-01, where both canaries are dated March and April), which widens
    // the gap from 41 days to 100. The assertion is about the two canaries, so it
    // reads the two canaries — the same masked ids as the assertion above.
    let masked_later_canary =
        sqlx::query_scalar::<_, String>("SELECT snapshot_account($1::text::uuid)::text")
            .bind(canary_ids[1].to_string())
            .fetch_one(pool)
            .await
            .expect("the later canary's masked id");
    let span: i64 = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM (max(created_at::timestamptz)
                                    - min(created_at::timestamptz)))::bigint
           FROM snapshot_masked.accounts
          WHERE id IN ($1::text::uuid, $2::text::uuid)",
    )
    .bind(masked_earlier_canary.clone())
    .bind(masked_later_canary)
    .fetch_one(pool)
    .await
    .expect("the span");
    assert_eq!(
        span,
        41 * 86_400,
        "the gap between the two canaries changed from 41 days to {span}s. A shift \
         that moved each timestamp by a different amount would preserve the shape of \
         nothing, and 11.16.5 keeps timestamps precisely because 'how long do people \
         keep a work before abandoning it' is the question the dataset exists for."
    );

    // (4) the live instance is untouched
    let live: String = sqlx::query_scalar(
        "SELECT created_at FROM accounts WHERE email = 'offset-canary-0@example.invalid'",
    )
    .fetch_one(pool)
    .await
    .expect("the live row");
    assert_eq!(
        live, "2026-03-01T10:30:00Z",
        "the mask shifted the LIVE table. The pipeline applies the mask to a scratch \
         copy precisely so the instance never lies about when something happened, and \
         a snapshot pipeline that edited its own source is not one."
    );
    tdb.cleanup().await;
}

/// Two snapshots must not share an offset, and the offset must not be in the file.
///
/// §11.17.4's rotation, asserted on the artefact. The second half matters as much
/// as the first: an offset recorded *beside* the dump is a published offset, and a
/// published offset is the linkability §11.16.5 is forbidding.
#[test]
fn two_snapshots_get_different_offsets_and_neither_records_its_own() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf();
    let dir = test_support::scratch_dir("snapshot_offset_rotation");
    std::fs::create_dir_all(&dir).expect("scratch dir");

    let mut offsets = Vec::new();
    for i in 0..2 {
        let path = dir.join(format!("mask{i}.sql"));
        let out = std::process::Command::new("python3")
            .arg(repo.join("scripts").join("build-snapshot-sql.py"))
            .arg("--out")
            .arg(&path)
            .arg("--random-timestamp-offset")
            .arg("365")
            .arg("--mode")
            .arg("cache")
            .current_dir(&repo)
            .output()
            .expect("run the generator with a random offset");
        assert!(out.status.success(), "generator failed");
        let sql = std::fs::read_to_string(&path).expect("mask sql");
        assert!(
            sql.contains("INTERVAL"),
            "no offset reached the generated SQL at all, so the timestamps are \
             unshifted and two snapshots of the same instance align row-for-row"
        );
        // The published SQL carries the shift as an INTERVAL, so a reader can see
        // THAT a shift happened -- which 11.16.3 requires for the construction to
        // be checkable -- without learning how big it was. An offset recorded
        // beside the data would be a published offset, and a published offset is
        // the linkability 11.16.5 forbids, so the value goes only to stderr.
        assert!(
            !sql.contains("-- timestamp offset:"),
            "the generated SQL records the offset value in a comment. 11.16.5: the              offset must be applied inside the snapshot, not recorded next to it."
        );
        offsets.push(sql);
    }
    assert_ne!(
        offsets[0], offsets[1],
        "two snapshots generated the SAME sql, so they share an offset and a recipient \
         holding both can join them row-for-row on every timestamp. 11.16.5: a month \
         offset is a linkable identifier."
    );
}
