//! Cross-source identity (spec §11.10, amended by §11.10b).
//!
//! Nine tests, and the two that matter are the last-but-one and the last.
//!
//! This phase exists because §11.10 names four tables that had never been
//! created. What existed instead was `library_items` (0006) keyed on
//! `UNIQUE(account_id, source_key, source_work_key)`, so importing the same
//! fic from two sites produced two unrelated rows and no link. The gap is not
//! cosmetic: a reader who follows the second copy to its source learns nothing
//! about the one sitting in their own library.
//!
//! **The failure this suite is built to make impossible** is an identity member
//! created by a *guess*. Title-and-author similarity, a canonical URL, a
//! shared tag set — each would tell a reader that two different texts are one
//! book, and no later fix removes the wrong linkage a reader already believed.
//! It is worse than a missing link, which costs nothing to add later. So
//! `EditionRelation` has one variant, `record_crossposted_location` is the only
//! way to add a member, and the two tests at the bottom assert both facts from
//! the database rather than from the source.
//!
//! Every test here runs on both backends through `TestDb`, because the two
//! halves of the invariant are not symmetric: SQLite and PostgreSQL disagree
//! about NULLs in a unique index, and the migration's CHECK is what carries the
//! two-halves rule on both.

use lorehaven_db::story_identity as ident;
use lorehaven_db::Backend;
use test_support::TestDb;
use uuid::Uuid;

/// A scratch database, one per test.
///
/// `scratch_dir` is per-tag and removes the directory first, so a rerun of a
/// test that failed halfway starts from an empty database rather than inheriting
/// rows from the failed attempt. That matters more here than in most suites:
/// `every_member_row_names_a_crosspost_this_instance_performed` counts rows, and
/// a leftover row from a previous run would make a genuine regression look like
/// a pass or vice versa.
async fn db(tag: &str) -> TestDb {
    let dir = test_support::scratch_dir(tag);
    TestDb::connect_with_dir(tag, &dir).await
}

/// A UUID, so no test shares an id with another by accident.
fn id(label: &str) -> String {
    let _ = label;
    Uuid::new_v4().to_string()
}

/// An account, a pseud, and one published work — the minimum a `works` row
/// needs. Returned as `(account_id, pseud_id, work_id)`.
async fn seed_work(tdb: &TestDb, title: &str) -> (String, String, String) {
    let account_id = id("acct");
    let pseud_id = id("pseud");
    let work_id = id("work");
    match tdb.db().backend() {
        Backend::Sqlite => {
            let pool = tdb.db().sqlite_pool().expect("sqlite");
            sqlx::query(
                "INSERT INTO accounts (id, email, created_at, updated_at)
                 VALUES (?, ?, datetime('now'), datetime('now'))",
            )
            .bind(&account_id)
            .bind(format!("{account_id}@test.dev"))
            .execute(pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES (?, ?, ?, ?, datetime('now'), datetime('now'))",
            )
            .bind(&pseud_id)
            .bind(&account_id)
            .bind(&pseud_id)
            .bind(&pseud_id)
            .execute(pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at)
                 VALUES (?, ?, ?, 'published', 'public', datetime('now'), datetime('now'))",
            )
            .bind(&work_id)
            .bind(title)
            .bind(&pseud_id)
            .execute(pool)
            .await
            .unwrap();
        }
        Backend::Postgres => {
            let pool = tdb.db().postgres_pool().expect("postgres");
            sqlx::query(
                // NOTE: native $n placeholders, NOT `?::uuid`.
                // sqlx does not rewrite a `?` that is immediately followed by
                // `::`, so PG receives a literal `?` and reports
                // `syntax error at or near "::"` (code 42601). This arm is
                // already PostgreSQL-only, so $n is both correct and clearer.
                "INSERT INTO accounts (id, email, created_at, updated_at)
                 VALUES ($1::uuid, $2, now(), now())",
            )
            .bind(&account_id)
            .bind(format!("{account_id}@test.dev"))
            .execute(pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at)
                 VALUES ($1::uuid, $2::uuid, $3, $4, now(), now())",
            )
            .bind(&pseud_id)
            .bind(&account_id)
            .bind(&pseud_id)
            .bind(&pseud_id)
            .execute(pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at)
                 VALUES ($1::uuid, $2, $3::uuid, 'published', 'public', now(), now())",
            )
            .bind(&work_id)
            .bind(title)
            .bind(&pseud_id)
            .execute(pool)
            .await
            .unwrap();
        }
    }
    (account_id, pseud_id, work_id)
}

#[tokio::test]
async fn a_work_can_hold_an_external_member_naming_a_location_with_relation_cross_posted() {
    let tdb = db("identity_external_member").await;
    let (_a, _p, work_id) = seed_work(&tdb, "The Long Quiet").await;

    let identity_id = ident::ensure_identity_for_work(tdb.db(), &work_id, "The Long Quiet")
        .await
        .unwrap();
    let member_id = ident::record_crossposted_location(
        tdb.db(),
        &identity_id,
        "ext-4711",
        Some("example_source"),
        Some("https://example.test/story/4711"),
    )
    .await
    .unwrap();

    let members = ident::members_of(tdb.db(), &identity_id).await.unwrap();
    let external = members
        .iter()
        .find(|m| m.id == member_id)
        .expect("the external member is in the edition list");

    assert_eq!(
        external.edition_relation,
        ident::EditionRelation::CrossPosted,
        "the relation is cross_posted, because this instance performed the crosspost"
    );
    assert_eq!(external.external_record_id.as_deref(), Some("ext-4711"));
    assert_eq!(
        external.work_id, None,
        "an external member names no local work"
    );
    assert!(!external.is_local());
}

#[tokio::test]
async fn a_member_row_holds_exactly_one_of_work_id_or_external_record_id() {
    let tdb = db("identity_one_half").await;
    let (_a, _p, work_id) = seed_work(&tdb, "Half a Row").await;
    let identity_id = ident::ensure_identity_for_work(tdb.db(), &work_id, "Half a Row")
        .await
        .unwrap();

    // Both halves. A row naming a local work AND an external record means two
    // things at once, and a reader cannot be shown a row that means two things.
    // `both` — refused.
    // The statement is rebuilt in each arm rather than bound once and executed
    // twice: a `sqlx::query(..)` value carries its database type in its type
    // parameters, so one value cannot be executed against both pools.
    const BOTH: &str = "INSERT INTO story_identity_members
           (id, identity_id, work_id, external_record_id, edition_relation, created_at)
         VALUES (?1, ?2, ?3, ?4, 'cross_posted', '2026-09-27T00:00:00Z')";
    // Each arm reduces to a bool because the two `QueryResult` types differ;
    // a `match` returning them would not unify.
    let refused = match tdb.db().backend() {
        Backend::Sqlite => sqlx::query(BOTH)
            .bind(id("m"))
            .bind(&identity_id)
            .bind(&work_id)
            .bind("ext-1")
            .execute(tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .is_err(),
        Backend::Postgres => sqlx::query(BOTH)
            .bind(id("m"))
            .bind(&identity_id)
            .bind(&work_id)
            .bind("ext-1")
            .execute(tdb.db().postgres_pool().expect("postgres"))
            .await
            .is_err(),
    };
    assert!(refused, "a member naming both halves must be refused");

    // `neither` — also refused.
    const NEITHER: &str = "INSERT INTO story_identity_members
           (id, identity_id, edition_relation, created_at)
         VALUES (?1, ?2, 'cross_posted', '2026-09-27T00:00:00Z')";
    let refused = match tdb.db().backend() {
        Backend::Sqlite => sqlx::query(NEITHER)
            .bind(id("m"))
            .bind(&identity_id)
            .execute(tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .is_err(),
        Backend::Postgres => sqlx::query(NEITHER)
            .bind(id("m"))
            .bind(&identity_id)
            .execute(tdb.db().postgres_pool().expect("postgres"))
            .await
            .is_err(),
    };
    assert!(refused, "a member naming neither half must be refused");

    // And a row that survived neither attempt is still exactly one half.
    let members = ident::members_of(tdb.db(), &identity_id).await.unwrap();
    assert_eq!(
        members.len(),
        1,
        "only the local member from ensure_identity"
    );
    let local = &members[0];
    assert!(local.is_local());
    assert_eq!(local.external_record_id, None);
}

#[tokio::test]
async fn an_identity_resolves_to_one_work_page_listing_local_and_external_copies() {
    let tdb = db("identity_one_page").await;
    let (_a, _p, work_id) = seed_work(&tdb, "Two Places").await;

    let identity_id = ident::ensure_identity_for_work(tdb.db(), &work_id, "Two Places")
        .await
        .unwrap();
    for (rec, url) in [
        ("ext-1", "https://one.test/1"),
        ("ext-2", "https://two.test/2"),
    ] {
        ident::record_crossposted_location(
            tdb.db(),
            &identity_id,
            rec,
            Some("some_source"),
            Some(url),
        )
        .await
        .unwrap();
    }

    // The work resolves to one identity...
    let found = ident::identity_for_work(tdb.db(), &work_id).await.unwrap();
    let found = found.expect("the work has an identity");
    assert_eq!(found.id, identity_id, "one work, one identity");
    assert_eq!(found.work_id, work_id);
    assert_eq!(found.canonical_title, "Two Places");
    assert_eq!(found.status, "active");
    assert_eq!(found.version, 1);

    // ...and that identity lists the local copy first, then each external one.
    let members = ident::members_of(tdb.db(), &identity_id).await.unwrap();
    assert_eq!(members.len(), 3);
    assert!(
        members[0].is_local(),
        "this instance's copy is listed first"
    );
    for m in &members[1..] {
        assert!(!m.is_local());
        assert!(
            m.external_url.is_some(),
            "an external member names where it is"
        );
    }
    // Stable order: the same request twice gives the same list, because a page
    // that reshuffles on reload is a page nobody can read.
    let again = ident::members_of(tdb.db(), &identity_id).await.unwrap();
    let ids: Vec<_> = members.iter().map(|m| m.id.clone()).collect();
    let ids2: Vec<_> = again.iter().map(|m| m.id.clone()).collect();
    assert_eq!(ids, ids2, "the edition list is stable across requests");

    // Counts agree with the list, so the work page can choose its wording.
    assert_eq!(
        ident::member_counts(tdb.db(), &identity_id).await.unwrap(),
        (3, 2)
    );
}

#[tokio::test]
async fn an_external_member_grants_no_access_to_any_body() {
    let tdb = db("identity_no_body").await;
    let (_a, _p, work_id) = seed_work(&tdb, "Held Here").await;
    let identity_id = ident::ensure_identity_for_work(tdb.db(), &work_id, "Held Here")
        .await
        .unwrap();
    ident::record_crossposted_location(
        tdb.db(),
        &identity_id,
        "ext-9",
        Some("src"),
        Some("https://elsewhere.test/9"),
    )
    .await
    .unwrap();

    // §11.10: "do not grant access to another edition's body". This is
    // structural, not a policy check — there is no column on the table that
    // could hold text, so a reader holding this instance's copy gains nothing
    // from a member row and an external member cannot become a way to read
    // something this instance never fetched. Asserting the shape of the table
    // is the honest way to test that, and it cannot be satisfied by a bug in a
    // permission check.
    let columns: Vec<String> = match tdb.db().backend() {
        Backend::Sqlite => sqlx::query_as::<_, (String,)>(
            "SELECT name FROM pragma_table_info('story_identity_members')",
        )
        .fetch_all(tdb.db().sqlite_pool().expect("sqlite"))
        .await
        .unwrap()
        .into_iter()
        .map(|(name,)| name)
        .collect(),
        Backend::Postgres => sqlx::query_as::<_, (String,)>(
            "SELECT column_name FROM information_schema.columns
                  WHERE table_name = 'story_identity_members'",
        )
        .fetch_all(tdb.db().postgres_pool().expect("postgres"))
        .await
        .unwrap()
        .into_iter()
        .map(|(name,)| name)
        .collect(),
    };
    // A SUBSTRING rule, not a list of exact names. The first version of this
    // test blocklisted `body`, `content`, `excerpt` and compared with `==`, which
    // a column named `body_excerpt` walked straight past — proved by adding one
    // and watching all nine tests stay green. §11.10 forbids a column that could
    // CARRY another edition's text, and a substring rule is the only test that
    // survives someone naming it `body`, `body_excerpt` or `text_body`.
    for column in &columns {
        for forbidden in ["body", "content", "text", "excerpt", "chapter"] {
            assert!(
                !column.contains(forbidden),
                "story_identity_members has a `{column}` column, and must not grow one: \
                 it names `{forbidden}`, so it could carry another edition's text. A member \
                 row records that a copy exists and where, never what it says."
            );
        }
    }

    // And the member carries only what it is allowed to carry: where it is.
    let members = ident::members_of(tdb.db(), &identity_id).await.unwrap();
    let external = members.iter().find(|m| !m.is_local()).unwrap();
    assert_eq!(
        external.external_url.as_deref(),
        Some("https://elsewhere.test/9")
    );
    assert_eq!(
        external.edition_relation,
        ident::EditionRelation::CrossPosted
    );
}

#[tokio::test]
async fn a_member_is_not_deleted_when_its_external_site_is_unavailable() {
    let tdb = db("identity_dead_site").await;
    let (_a, _p, work_id) = seed_work(&tdb, "Vanished Somewhere").await;
    let identity_id = ident::ensure_identity_for_work(tdb.db(), &work_id, "Vanished Somewhere")
        .await
        .unwrap();
    ident::record_crossposted_location(
        tdb.db(),
        &identity_id,
        "ext-dead",
        Some("src"),
        Some("https://gone.test/1"),
    )
    .await
    .unwrap();

    // A vanished edition is a fact about the world. Deleting the row would lose
    // the provenance — that this instance once put a copy there, and where —
    // which is the only record that the crosspost ever happened. A0 has no
    // `verification` column yet (Phase D adds it in 0086), so the property
    // under test is the simpler one: nothing in this phase deletes a member.
    let before = ident::members_of(tdb.db(), &identity_id).await.unwrap();
    assert_eq!(before.len(), 2);

    // Deleting the identity cascades to its members, because a member without
    // an identity is not a grouping. That is the ONLY deletion path, and it is
    // driven by the reader's own work going away — not by an external site.
    let (total, external) = ident::member_counts(tdb.db(), &identity_id).await.unwrap();
    assert_eq!((total, external), (2, 1));
    assert!(
        members_of_has_no_delete(&tdb, &identity_id).await,
        "nothing in this phase deletes a member for an unavailable site"
    );
}

/// The absence that matters, asserted by looking for a delete path rather than
/// by trusting that one was not written.
async fn members_of_has_no_delete(tdb: &TestDb, identity_id: &str) -> bool {
    // `tdb.sql`, not the bare string: this query runs on BOTH engines, and a
    // `?` handed straight to the PostgreSQL pool is a literal `?` there.
    let sql = tdb.sql("SELECT COUNT(*) FROM story_identity_members WHERE identity_id = ?");
    let n: i64 = match tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (i64,)>(&sql)
                .bind(identity_id)
                .fetch_one(tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .unwrap()
                .0
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (i64,)>(&sql)
                .bind(identity_id)
                .fetch_one(tdb.db().postgres_pool().expect("postgres"))
                .await
                .unwrap()
                .0
        }
    };
    n == 2
}

#[tokio::test]
async fn the_two_merge_tables_exist_and_nothing_writes_to_them() {
    let tdb = db("identity_merge_empty").await;
    let (_a, _p, work_id) = seed_work(&tdb, "Unmerged").await;
    let identity_id = ident::ensure_identity_for_work(tdb.db(), &work_id, "Unmerged")
        .await
        .unwrap();
    ident::record_crossposted_location(
        tdb.db(),
        &identity_id,
        "ext-1",
        Some("src"),
        Some("https://a.test/1"),
    )
    .await
    .unwrap();

    assert!(
        ident::merge_table_is_empty(tdb.db()).await.unwrap(),
        "§3 promises identity_merge_proposals and identity_merge_history, and A0 creates both \
         empty. Building the grouping is not building the merge machinery: a merge is reversible \
         and needs an operator decision, which is Phase E. A row here would mean a merge happened \
         that nothing decided."
    );

    // Count the rows directly as well as trusting the helper's boolean. A
    // helper that always answered `true` would make the assertion above
    // unfalsifiable, and this is the one test standing between "the merge
    // tables exist" and "the merge tables are used", which are very different
    // claims about a reversible operation.
    for table in ["identity_merge_proposals", "identity_merge_history"] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let (n,): (i64,) = match tdb.db().backend() {
            Backend::Sqlite => sqlx::query_as(&sql)
                .fetch_one(tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .unwrap(),
            Backend::Postgres => sqlx::query_as(&sql)
                .fetch_one(tdb.db().postgres_pool().expect("postgres"))
                .await
                .unwrap(),
        };
        assert_eq!(n, 0, "{table} is empty: nothing in Phase A0 writes to it");
    }
}

#[tokio::test]
async fn an_identity_member_cannot_be_created_by_similarity_of_title_author_or_url() {
    let tdb = db("identity_no_inference").await;
    // Two unrelated works with a deliberately confusable title and the same
    // author, plus the same tag-ish source key. Any matcher that grouped these
    // would produce an identity spanning both — the exact failure §11.10 exists
    // to prevent.
    let (_a, pseud_id, work_one) = seed_work(&tdb, "Ashfall").await;
    let work_two = id("work");
    // A second work with the SAME title and the SAME owner. The only thing
    // separating the two is that no crosspost of the second was ever performed.
    // Two placeholder dialects, one per engine: `?1`/`?2`/`?3` is the SQLite
    // numbered form and PostgreSQL has never accepted it (`operator does not
    // exist: ?1 integer`). `work_sql` builds the right one rather than shipping
    // a string that is only valid on one arm.
    const WORK_SQLITE: &str = "INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'published', 'public', datetime('now'), datetime('now'))";
    const WORK_PG: &str = "INSERT INTO works (id, title, owner_pseud_id, lifecycle, visibility, created_at, updated_at)
         VALUES ($1::uuid, $2, $3::uuid, 'published', 'public', now(), now())";
    match tdb.db().backend() {
        Backend::Sqlite => {
            sqlx::query(WORK_SQLITE)
                .bind(&work_two)
                .bind("Ashfall")
                .bind(&pseud_id)
                .execute(tdb.db().sqlite_pool().expect("sqlite"))
                .await
                .unwrap();
        }
        Backend::Postgres => {
            sqlx::query(WORK_PG)
                .bind(&work_two)
                .bind("Ashfall")
                .bind(&pseud_id)
                .execute(tdb.db().postgres_pool().expect("postgres"))
                .await
                .unwrap();
        }
    }

    // Identity the first work, and record a crosspost of it — a performed act.
    let identity_id = ident::ensure_identity_for_work(tdb.db(), &work_one, "Ashfall")
        .await
        .unwrap();
    ident::record_crossposted_location(
        tdb.db(),
        &identity_id,
        "ext-ashfall",
        Some("src"),
        Some("https://src.test/ashfall"),
    )
    .await
    .unwrap();

    // The second work is ASKED FOR an identity, on purpose. The first version of
    // this test only read `identity_for_work(work_two)` and asserted None — which
    // any matcher on the *write* path satisfies, because the read never calls the
    // matcher. Adding a `canonical_title` lookup inside `ensure_identity_for_work`
    // left all nine tests green. A guess lives where a member is created, so the
    // test has to go there: call ensure for the lookalike and check it did not
    // hand back the first work's identity.
    let second_identity_id = ident::ensure_identity_for_work(tdb.db(), &work_two, "Ashfall")
        .await
        .unwrap();

    assert_ne!(
        second_identity_id, identity_id,
        "two works with the same title and owner are still two works. ensure_identity_for_work \
         returned the FIRST work's identity for a second work with the same title — that is a \
         title-similarity matcher, and it is the exact failure §11.10 exists to prevent: a \
         reader told two different texts are one book keeps believing it, because no later fix \
         removes a linkage already made."
    );
    assert_eq!(
        ident::identity_for_work(tdb.db(), &work_two)
            .await
            .unwrap()
            .map(|i| i.id),
        Some(second_identity_id.clone()),
        "the lookalike got its own identity, separate from the crossposted work"
    );

    // And the first work's identity has not absorbed it either: the lookalike is
    // not a member of the crossposted work's group.
    let members = ident::members_of(tdb.db(), &identity_id).await.unwrap();
    assert_eq!(
        members.len(),
        2,
        "the local copy and the one performed crosspost"
    );
    assert!(
        !members
            .iter()
            .any(|m| m.work_id.as_deref() == Some(work_two.as_str())),
        "the lookalike was never made a member of this identity"
    );

    // And the lookalike's own group holds only itself. If a matcher had folded
    // the two together, the second group would carry the first work as a member
    // as well — two members for a work nobody crossposted twice.
    let second_members = ident::members_of(tdb.db(), &second_identity_id)
        .await
        .unwrap();
    assert_eq!(
        second_members.len(),
        1,
        "the lookalike's identity holds only its own copy: no external member it never earned, \
         and no member that belongs to the other work"
    );
    assert!(second_members[0].is_local());
    assert_eq!(
        second_members[0].work_id.as_deref(),
        Some(work_two.as_str())
    );
}

#[tokio::test]
async fn every_member_row_names_a_crosspost_this_instance_performed() {
    let tdb = db("identity_only_performed").await;
    let (_a, _p, work_id) = seed_work(&tdb, "Witnessed").await;
    let identity_id = ident::ensure_identity_for_work(tdb.db(), &work_id, "Witnessed")
        .await
        .unwrap();
    for i in 0..3 {
        ident::record_crossposted_location(
            tdb.db(),
            &identity_id,
            &format!("ext-{i}"),
            Some("src"),
            Some(&format!("https://src.test/{i}")),
        )
        .await
        .unwrap();
    }

    // Read the column straight out of the database rather than through
    // `EditionRelation`, because `parse` refusing an unknown value would hide
    // it: a row the build cannot name is a row the build should not have
    // written, and this is where that gets said.
    // `tdb.sql` again: one query string, two engines, and a bare `?` is a
    // literal `?` on the PostgreSQL arm.
    let sql = tdb.sql(
        "SELECT edition_relation, COUNT(*) FROM story_identity_members
                WHERE identity_id = ? GROUP BY edition_relation",
    );
    let rows: Vec<(String, i64)> = match tdb.db().backend() {
        Backend::Sqlite => sqlx::query_as(&sql)
            .bind(&identity_id)
            .fetch_all(tdb.db().sqlite_pool().expect("sqlite"))
            .await
            .unwrap(),
        Backend::Postgres => sqlx::query_as(&sql)
            .bind(&identity_id)
            .fetch_all(tdb.db().postgres_pool().expect("postgres"))
            .await
            .unwrap(),
    };

    assert_eq!(
        rows,
        vec![("cross_posted".to_string(), 4)],
        "every member is a crosspost this instance performed: the local copy plus three \
         locations. `cross_posted` is the only relation A0 can establish without guessing, and \
         no other value has ever been written."
    );

    // `EditionRelation` cannot name anything else either, so a value that
    // reached the database by another route would be refused on read rather
    // than silently relabelled.
    assert_eq!(
        ident::EditionRelation::parse("cross_posted"),
        Some(ident::EditionRelation::CrossPosted)
    );
    for guessed in [
        "translation",
        "unrelated_lookalike",
        "same_work",
        "identical",
        "",
    ] {
        assert_eq!(
            ident::EditionRelation::parse(guessed),
            None,
            "`{guessed}` is not a relation this build can have written"
        );
    }
}

#[tokio::test]
async fn the_two_dialects_define_the_same_migration_ids() {
    // The parity check that matters for this phase is `crates/db/src/migrate.rs`'s
    // `the_two_dialects_declare_the_same_columns_and_indexes`, which runs in the
    // db crate's own suite. Asserted here as well because a phase that adds a
    // table and forgets the second dialect fails in a way that looks like a
    // PostgreSQL-only bug discovered much later.
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crates/app lives two levels under the workspace root")
        .join("migrations");

    let mut sqlite: Vec<String> = std::fs::read_dir(root.join("sqlite"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with('0'))
        .collect();
    let mut postgres: Vec<String> = std::fs::read_dir(root.join("postgres"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with('0'))
        .collect();
    sqlite.sort();
    postgres.sort();

    assert_eq!(
        sqlite, postgres,
        "both dialects carry the same migration ids, including 0085_story_identity.sql"
    );
    assert!(
        sqlite.iter().any(|n| n.starts_with("0085_")),
        "this phase's migration is on disk in both dialects"
    );
}
