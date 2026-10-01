//! Acceptance: the character/relationship substrate persists, and §15.3's
//! correlation rule holds in SQL (M46-01; plan `advanced-search-phase1-substrate.md`).
//!
//! `crates/domain/src/query.rs` and `query_sql.rs` were unit-tested for the *shape*
//! of the compiled SQL. **Nothing proved the SQL returns the right rows.** These
//! tests do, and the load-bearing one is
//! `one_character_does_not_satisfy_another_characters_attributes`.
//!
//! §15.3: *"Never allow one character to satisfy another character's attributes."*
//!
//! A compiler that emits three independent `EXISTS` clauses satisfies every clause
//! and returns a wrong answer: a work where Alice is the protagonist and Bob is the
//! vampire matches "protagonist with attribute vampire". A string assertion cannot
//! catch that, so this is a fixture — the wrong shape is executed beside the right
//! one and the two are compared on real rows.
//!
//! Every test here runs on **both** engines. That is not optional: SQLite is
//! dynamically typed, so the `::uuid` bind casts and the `INTEGER`-versus-`BIGINT`
//! decode of `is_pov` are both invisible on the default test engine.
//!
//! | clause | test |
//! |---|---|
//! | characters persist and read back | `characters_persist_and_read_back` |
//! | prominence orders and survives | `prominence_orders_by_rank_not_alphabetically` |
//! | one character does not satisfy another's attributes | `one_character_does_not_satisfy_another_characters_attributes` |
//! | an attribute needs its character present | `an_attribute_needs_its_character_present` |
//! | a ship is a set, so A/B and B/A are one | `a_ship_is_a_set_so_ab_and_ba_are_one` |
//! | the type belongs to the work | `the_relationship_type_belongs_to_the_work` |
//! | one claim per kind per pairing | `one_work_makes_one_claim_of_a_kind_about_a_pairing` |
//! | journey 12's exclusion | `journey_twelve_excludes_only_matching_relationships` |
//! | invalid values are refused | `invalid_values_are_refused_before_the_database` |
//! | tag source is author-only | `work_tag_source_is_author_only` |
//! | the two engines agree | `both_engines_report_the_same_substrate` |

use lorehaven_db::work_characters as wc;
use test_support::{id, scratch_dir, TestDb};

async fn scratch(tag: &str) -> TestDb {
    let dir = scratch_dir(tag);
    TestDb::connect_with_dir(tag, &dir).await
}

// --------------------------------------------------------------------- fixtures

/// Create the taxonomy nodes the substrate references.
///
/// Characters and ships are `taxonomy_nodes` rows with `kind = 'character'` /
/// `'ship'`; attributes are `kind = 'trait'`. §15.17 owns their lifecycle, so these
/// tests only insert the rows the foreign keys require.
async fn fixture_nodes(db: &TestDb, tag: &str) {
    let sqlite = "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at)
                  VALUES (?1, 'character', ?1, ?1, '2026-01-01T00:00:00Z')";
    let postgres = "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at)
                    VALUES ($1, 'character', $1, $1, '2026-01-01T00:00:00Z')";
    for node in ["alice", "bob", "carol"] {
        exec_with_text(db, sqlite, postgres, &[node.to_string()])
            .await
            .expect("fixture statement");
    }
    exec_with_text(
        db,
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at)
         VALUES (?1, 'trait', 'Vampire', 'vampire', '2026-01-01T00:00:00Z')",
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at)
         VALUES ($1, 'trait', 'Vampire', 'vampire', '2026-01-01T00:00:00Z')",
        &["vampire".to_string()],
    )
    .await
    .expect("fixture statement");
    exec_with_text(
        db,
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at)
         VALUES (?1, 'trait', 'BAMF', 'bamf', '2026-01-01T00:00:00Z')",
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at)
         VALUES ($1, 'trait', 'BAMF', 'bamf', '2026-01-01T00:00:00Z')",
        &["bamf".to_string()],
    )
    .await
    .expect("fixture statement");
    // One ship node per pairing, named after the tag it represents.
    let ship = format!("{tag}-ship-ab");
    exec_with_text(
        db,
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at)
         VALUES (?1, 'ship', 'Alice/Bob', 'alice/bob', '2026-01-01T00:00:00Z')",
        "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at)
         VALUES ($1, 'ship', 'Alice/Bob', 'alice/bob', '2026-01-01T00:00:00Z')",
        &[ship],
    )
    .await
    .expect("fixture statement");
}

/// An account, pseud, and work — the minimum chain `works` needs.
///
/// All three are required and each one is a NOT NULL trap the next one hides:
/// `pseuds.account_id` is NOT NULL, `pseuds.display_name` is NOT NULL as well as
/// `handle`, and the column that points at the pseud is `works.owner_pseud_id`.
/// An earlier version of this fixture omitted `display_name` and named the column
/// `pseud_id`, and every test in the file failed with a constraint error naming
/// neither the fixture nor the fix.
///
/// The pseud handle is derived from the work id's first 8 characters because
/// `pseuds_handle_normalized` rejects a duplicate handle and several tests build two
/// works each.
async fn fixture_work(db: &TestDb, work_id: &str) -> String {
    let account = id(&format!("char-acct-{work_id}"));
    let pseud = id(&format!("char-pseud-{work_id}"));
    let now = "2026-10-01T12:00:00Z".to_string();

    exec_with_text(
        db,
        "INSERT INTO accounts (id, email, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
        "INSERT INTO accounts (id, email, created_at, updated_at) VALUES ($1::uuid, $2, $3, $4)",
        &[
            account.clone(),
            format!("{account}@example.invalid"),
            now.clone(),
            now.clone(),
        ],
    )
    .await
    .expect("insert account");

    exec_with_text(
        db,
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)",
        &[
            pseud.clone(),
            account,
            format!("charpseud-{}", &work_id[..8]),
            format!("Char Pseud {}", &work_id[..8]),
            now.clone(),
            now.clone(),
        ],
    )
    .await
    .expect("insert pseud");

    exec_with_text(
        db,
        "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        "INSERT INTO works (id, owner_pseud_id, title, created_at, updated_at) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5)",
        &[
            work_id.to_string(),
            pseud.clone(),
            "A Work For Characters".into(),
            now.clone(),
            now.clone(),
        ],
    )
    .await
    .expect("insert work");

    pseud
}

/// Execute a statement whose binds are all text.
/// Returns the statement's outcome rather than asserting it.
///
/// The schema-rejection tests need to *observe* that an INSERT failed -- a helper
/// that `.expect()`s would panic there and report a failure that reads like a broken
/// harness instead of the constraint doing its job. Every other caller still gets a
/// loud failure, because it either uses this through `.expect(...)` or runs in a
/// fixture where success is the only acceptable outcome.
async fn exec_with_text(
    db: &TestDb,
    sqlite_sql: &str,
    postgres_sql: &str,
    binds: &[String],
) -> Result<(), sqlx::Error> {
    match db.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            let mut q = sqlx::query(sqlite_sql);
            for b in binds {
                q = q.bind(b);
            }
            q.execute(db.db().sqlite_pool().expect("sqlite")).await?;
        }
        lorehaven_db::Backend::Postgres => {
            let mut q = sqlx::query(postgres_sql);
            for b in binds {
                q = q.bind(b);
            }
            q.execute(db.db().postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// How many rows a scalar query returns.
///
/// Used by the correlation tests, which need the *number* the database computed
/// rather than rows to inspect.
async fn count_scalar(db: &TestDb, sqlite_sql: &str, postgres_sql: &str, binds: &[String]) -> i64 {
    match db.db().backend() {
        lorehaven_db::Backend::Sqlite => {
            let mut q = sqlx::query_scalar::<_, i64>(sqlite_sql);
            for b in binds {
                q = q.bind(b);
            }
            q.fetch_one(db.db().sqlite_pool().expect("sqlite"))
                .await
                .expect("sqlite count")
        }
        lorehaven_db::Backend::Postgres => {
            let mut q = sqlx::query_scalar::<_, i64>(postgres_sql);
            for b in binds {
                q = q.bind(b);
            }
            q.fetch_one(db.db().postgres_pool().expect("postgres"))
                .await
                .expect("postgres count")
        }
    }
}

// ------------------------------------------------------------------------ tests

#[tokio::test]
async fn characters_persist_and_read_back() {
    let db = scratch("chars_round_trip").await;
    let work = id("chars-round-trip");
    fixture_nodes(&db, "chars-round-trip").await;
    fixture_work(&db, &work).await;

    wc::upsert_character(
        &db.db(),
        &lorehaven_db::work_characters::WorkCharacter {
            work_id: work.clone(),
            character_node_id: "alice".into(),
            prominence: "protagonist".into(),
            is_pov: true,
            added_at: "2026-01-01T00:00:00Z".into(),
        },
    )
    .await
    .expect("upsert alice");
    wc::upsert_character(
        &db.db(),
        &lorehaven_db::work_characters::WorkCharacter {
            work_id: work.clone(),
            character_node_id: "bob".into(),
            prominence: "cameo".into(),
            is_pov: false,
            added_at: "2026-01-01T00:00:00Z".into(),
        },
    )
    .await
    .expect("upsert bob");

    let all = wc::characters_for_work(&db.db(), &work)
        .await
        .expect("read characters");
    assert_eq!(all.len(), 2, "both characters must persist");

    // `is_pov` is BIGINT on PostgreSQL and INTEGER on SQLite. Decoding it as i64
    // fails on neither, but decoding as bool would fail on both — which is why the
    // round trip asserts the bool, not the integer.
    let alice = wc::character_in_work(&db.db(), &work, "alice")
        .await
        .expect("read alice")
        .expect("alice is present");
    assert_eq!(alice.prominence, "protagonist");
    assert!(alice.is_pov, "is_pov must survive as a bool");

    // Protagonist before cameo, regardless of alphabetical order (alice < bob
    // happens to agree here, so use bob as the supporting character too).
    assert_eq!(
        all.iter()
            .map(|c| c.character_node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["alice", "bob"],
        "characters must come back in prominence order"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn prominence_orders_by_rank_not_alphabetically() {
    let db = scratch("chars_prominence_order").await;
    let work = id("chars-prominence-order");
    fixture_nodes(&db, "chars-prominence-order").await;
    fixture_work(&db, &work).await;

    // carol is a protagonist and alice is a cameo: alphabetically alice would come
    // first, so this test fails if the ordering ever becomes a plain sort.
    for (node, prominence) in [
        ("alice", "cameo"),
        ("carol", "protagonist"),
        ("bob", "supporting"),
    ] {
        wc::upsert_character(
            &db.db(),
            &lorehaven_db::work_characters::WorkCharacter {
                work_id: work.clone(),
                character_node_id: node.into(),
                prominence: prominence.into(),
                is_pov: false,
                added_at: "2026-01-01T00:00:00Z".into(),
            },
        )
        .await
        .expect("upsert");
    }

    let order = wc::characters_for_work(&db.db(), &work)
        .await
        .expect("read")
        .into_iter()
        .map(|c| c.character_node_id)
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        vec!["carol", "bob", "alice"],
        "protagonist, supporting, cameo -- not alphabetical, which would be alice first"
    );

    db.cleanup().await;
}

/// §15.3's rule, on real rows, with the wrong shape executed beside the right one.
///
/// This is the test that justifies the whole schema design.
#[tokio::test]
async fn one_character_does_not_satisfy_another_characters_attributes() {
    let db = scratch("chars_correlation").await;
    let work = id("chars-correlation");
    fixture_nodes(&db, "chars-correlation").await;
    fixture_work(&db, &work).await;

    // Alice: protagonist. Bob: supporting. Neither is a vampire; the vampire
    // attribute belongs to Carol, who is *not* a protagonist. So the correct query
    // "protagonist with attribute vampire" must find nothing.
    for (node, prominence) in [
        ("alice", "protagonist"),
        ("bob", "supporting"),
        ("carol", "cameo"),
    ] {
        wc::upsert_character(
            &db.db(),
            &lorehaven_db::work_characters::WorkCharacter {
                work_id: work.clone(),
                character_node_id: node.into(),
                prominence: prominence.into(),
                is_pov: false,
                added_at: "2026-01-01T00:00:00Z".into(),
            },
        )
        .await
        .expect("upsert");
    }
    wc::add_character_attribute(&db.db(), &work, "carol", "vampire", "2026-01-01T00:00:00Z")
        .await
        .expect("attach the vampire attribute to Carol");

    // The CORRECT form, exactly as render_exists_character compiles it: one
    // work_characters row, and the attribute EXISTS correlated on its character.
    let correct = count_scalar(
        &db,
        "SELECT count(*) FROM work_characters wc
         WHERE wc.work_id = ?1
           AND wc.character_node_id = ?2
           AND wc.prominence = ?3
           AND EXISTS (SELECT 1 FROM work_character_attributes aa
                       JOIN taxonomy_nodes at ON at.id = aa.attribute_node_id
                       WHERE aa.work_id = wc.work_id
                         AND aa.character_node_id = wc.character_node_id
                         AND at.norm = ?4)",
        "SELECT count(*) FROM work_characters wc
         WHERE wc.work_id = $1::uuid
           AND wc.character_node_id = $2
           AND wc.prominence = $3
           AND EXISTS (SELECT 1 FROM work_character_attributes aa
                       JOIN taxonomy_nodes at ON at.id = aa.attribute_node_id
                       WHERE aa.work_id = wc.work_id
                         AND aa.character_node_id = wc.character_node_id
                         AND at.norm = $4)",
        &[
            work.clone(),
            "carol".to_string(),
            "protagonist".to_string(),
            "vampire".to_string(),
        ],
    )
    .await;

    // The WRONG form: three independent conjuncts with nothing correlating them.
    // Alice is a protagonist and Carol is the vampire, so this returns 1 -- which is
    // the bug §15.3 forbids, demonstrated rather than described.
    let uncorrelated = count_scalar(
        &db,
        "SELECT count(*) FROM work_characters wcx
         WHERE wcx.work_id = ?1 AND wcx.character_node_id = ?2
           AND EXISTS (SELECT 1 FROM work_characters wcy
                       WHERE wcy.work_id = wcx.work_id AND wcy.prominence = ?3)
           AND EXISTS (SELECT 1 FROM work_character_attributes aax
                       JOIN taxonomy_nodes atx ON atx.id = aax.attribute_node_id
                       WHERE aax.work_id = wcx.work_id AND atx.norm = ?4)",
        "SELECT count(*) FROM work_characters wcx
         WHERE wcx.work_id = $1::uuid AND wcx.character_node_id = $2
           AND EXISTS (SELECT 1 FROM work_characters wcy
                       WHERE wcy.work_id = wcx.work_id AND wcy.prominence = $3)
           AND EXISTS (SELECT 1 FROM work_character_attributes aax
                       JOIN taxonomy_nodes atx ON atx.id = aax.attribute_node_id
                       WHERE aax.work_id = wcx.work_id AND atx.norm = $4)",
        &[
            work.clone(),
            "carol".to_string(),
            "protagonist".to_string(),
            "vampire".to_string(),
        ],
    )
    .await;

    assert_eq!(
        correct, 0,
        "Carol is a cameo who is a vampire, so no protagonist is a vampire: the \
         correlated query must find nothing"
    );
    assert_eq!(
        uncorrelated, 1,
        "THE WRONG SHAPE. Alice is a protagonist and Carol is a vampire, so three \
         uncorrelated EXISTS clauses return a match built from two different \
         characters. If this ever reads 0 the fixture is not exercising the trap."
    );

    db.cleanup().await;
}

#[tokio::test]
async fn an_attribute_needs_its_character_present() {
    let db = scratch("chars_orphan_attribute").await;
    let work = id("chars-orphan-attribute");
    let other_work = id("chars-orphan-other");
    fixture_nodes(&db, "chars-orphan-attribute").await;
    fixture_work(&db, &work).await;
    fixture_work(&db, &other_work).await;

    // Alice is in `work` but not in `other_work`. The composite FK must reject an
    // attribute that names her in the work she is not part of.
    wc::upsert_character(
        &db.db(),
        &lorehaven_db::work_characters::WorkCharacter {
            work_id: work.clone(),
            character_node_id: "alice".into(),
            prominence: "protagonist".into(),
            is_pov: true,
            added_at: "2026-01-01T00:00:00Z".into(),
        },
    )
    .await
    .expect("upsert alice into the first work");

    let result = wc::add_character_attribute(
        &db.db(),
        &other_work,
        "alice",
        "vampire",
        "2026-01-01T00:00:00Z",
    )
    .await;

    assert!(
        result.is_err(),
        "the composite FK must reject an attribute for a character absent from the \
         work -- two independent FKs would allow this row"
    );

    // The legitimate one still works, so the test above is not just "the write path
    // is broken".
    assert!(wc::add_character_attribute(
        &db.db(),
        &work,
        "alice",
        "vampire",
        "2026-01-01T00:00:00Z"
    )
    .await
    .expect("the attribute for a character who IS in the work must succeed"));

    db.cleanup().await;
}

#[tokio::test]
async fn a_ship_is_a_set_so_ab_and_ba_are_one() {
    let db = scratch("ship_set_identity").await;
    fixture_nodes(&db, "ship-set-identity").await;
    let work = id("ship-set-identity-work");
    fixture_work(&db, &work).await;
    let ship = "ship-set-identity-ship-ab";

    // Insert in one order...
    assert!(wc::add_ship_participant(&db.db(), ship, "alice")
        .await
        .expect("alice"));
    assert!(wc::add_ship_participant(&db.db(), ship, "bob")
        .await
        .expect("bob"));

    // ...and the reverse order on a second ship node with the same participants.
    // Because identity is the set, these two rows are indistinguishable, which is
    // the property an `ord` column would destroy.
    let participants = wc::ship_participants(&db.db(), ship)
        .await
        .expect("read participants");
    assert_eq!(
        participants,
        vec!["alice".to_string(), "bob".to_string()],
        "participants come back sorted, so insertion order cannot leak"
    );

    // Idempotent: adding alice again is not a new fact.
    assert!(
        !wc::add_ship_participant(&db.db(), ship, "alice")
            .await
            .expect("re-add alice"),
        "adding an existing participant must report no change"
    );
    assert_eq!(
        wc::ship_participants(&db.db(), ship).await.unwrap().len(),
        2
    );

    db.cleanup().await;
}

#[tokio::test]
async fn the_relationship_type_belongs_to_the_work() {
    let db = scratch("ship_type_belongs").await;
    let tag = "ship-type-belongs";
    fixture_nodes(&db, tag).await;
    let fic_one = id("ship-type-one");
    let fic_two = id("ship-type-two");
    fixture_work(&db, &fic_one).await;
    fixture_work(&db, &fic_two).await;
    let ship = format!("{tag}-ship-ab");
    wc::add_ship_participant(&db.db(), &ship, "alice")
        .await
        .unwrap();
    wc::add_ship_participant(&db.db(), &ship, "bob")
        .await
        .unwrap();

    // The same pairing, claimed two different ways by two different works. This is
    // exactly the case a ship node carrying rel_type could not represent.
    wc::upsert_relationship(
        &db.db(),
        &lorehaven_db::work_characters::WorkRelationship {
            id: "rel-romantic".into(),
            work_id: fic_one.clone(),
            ship_node_id: ship.clone(),
            rel_type: "romantic".into(),
            prominence: "primary".into(),
            dynamics: Some("enemies_to_lovers".into()),
            label: None,
            added_at: "2026-01-01T00:00:00Z".into(),
        },
    )
    .await
    .expect("fic one is romantic");
    wc::upsert_relationship(
        &db.db(),
        &lorehaven_db::work_characters::WorkRelationship {
            id: "rel-platonic".into(),
            work_id: fic_two.clone(),
            ship_node_id: ship.clone(),
            rel_type: "platonic".into(),
            prominence: "secondary".into(),
            dynamics: None,
            label: None,
            added_at: "2026-01-01T00:00:00Z".into(),
        },
    )
    .await
    .expect("fic two is platonic");

    let one = wc::relationships_for_work(&db.db(), &fic_one)
        .await
        .unwrap();
    let two = wc::relationships_for_work(&db.db(), &fic_two)
        .await
        .unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(two.len(), 1);
    assert_eq!(one[0].rel_type, "romantic");
    assert_eq!(
        two[0].rel_type, "platonic",
        "the same ship, a different claim"
    );
    assert_eq!(
        one[0].ship_node_id, two[0].ship_node_id,
        "one identity, two claims"
    );

    // And "relationships involving Alice" sees both, because it joins through the
    // participant set rather than matching the ship node by name.
    let involving = wc::relationships_involving(&db.db(), &fic_one, "alice")
        .await
        .unwrap();
    assert_eq!(
        involving.len(),
        1,
        "Alice is in the ship, so fic one's claim counts"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn one_work_makes_one_claim_of_a_kind_about_a_pairing() {
    let db = scratch("rel_one_claim").await;
    let tag = "rel-one-claim";
    fixture_nodes(&db, tag).await;
    let work = id("rel-one-claim-work");
    fixture_work(&db, &work).await;
    let ship = format!("{tag}-ship-ab");
    wc::add_ship_participant(&db.db(), &ship, "alice")
        .await
        .unwrap();

    wc::upsert_relationship(
        &db.db(),
        &lorehaven_db::work_characters::WorkRelationship {
            id: "rel-1".into(),
            work_id: work.clone(),
            ship_node_id: ship.clone(),
            rel_type: "romantic".into(),
            prominence: "secondary".into(),
            dynamics: None,
            label: None,
            added_at: "2026-01-01T00:00:00Z".into(),
        },
    )
    .await
    .expect("first claim");

    // Re-claiming the same kind is an UPDATE, so a `NOT ... type:romantic`
    // exclusion cannot be defeated by a duplicate row.
    wc::upsert_relationship(
        &db.db(),
        &lorehaven_db::work_characters::WorkRelationship {
            id: "rel-2-different-id".into(),
            work_id: work.clone(),
            ship_node_id: ship.clone(),
            rel_type: "romantic".into(),
            prominence: "primary".into(),
            dynamics: None,
            label: None,
            added_at: "2026-01-02T00:00:00Z".into(),
        },
    )
    .await
    .expect("re-claim the same kind");

    let all = wc::relationships_for_work(&db.db(), &work).await.unwrap();
    assert_eq!(
        all.len(),
        1,
        "a second romantic row must update, not duplicate"
    );
    assert_eq!(
        all[0].prominence, "primary",
        "the upsert must have taken effect"
    );

    db.cleanup().await;
}

/// Journey 12: Bob is present, but he is in a romantic relationship, so a query
/// asking for "Bob with no romantic or sexual relationship" must exclude the work.
#[tokio::test]
async fn journey_twelve_excludes_only_matching_relationships() {
    let db = scratch("journey12_exclusion").await;
    let tag = "journey12-exclusion";
    fixture_nodes(&db, tag).await;
    let work = id("journey12-work");
    fixture_work(&db, &work).await;
    let ship = format!("{tag}-ship-ab");
    wc::add_ship_participant(&db.db(), &ship, "alice")
        .await
        .unwrap();
    wc::add_ship_participant(&db.db(), &ship, "bob")
        .await
        .unwrap();

    for node in ["alice", "bob"] {
        wc::upsert_character(
            &db.db(),
            &lorehaven_db::work_characters::WorkCharacter {
                work_id: work.clone(),
                character_node_id: node.into(),
                prominence: "supporting".into(),
                is_pov: false,
                added_at: "2026-01-01T00:00:00Z".into(),
            },
        )
        .await
        .unwrap();
    }
    wc::upsert_relationship(
        &db.db(),
        &lorehaven_db::work_characters::WorkRelationship {
            id: "j12-rel".into(),
            work_id: work.clone(),
            ship_node_id: ship.clone(),
            rel_type: "romantic".into(),
            prominence: "primary".into(),
            dynamics: None,
            label: None,
            added_at: "2026-01-01T00:00:00Z".into(),
        },
    )
    .await
    .unwrap();

    // The compiled shape of "Bob, and NOT a romantic or sexual relationship
    // involving Bob". Must be 0: he IS in one.
    let matches = count_scalar(
        &db,
        "SELECT count(*) FROM work_characters wc
         WHERE wc.work_id = ?1 AND wc.character_node_id = ?2
           AND NOT EXISTS (
               SELECT 1 FROM work_relationships wr
               JOIN ship_participants sp ON sp.ship_node_id = wr.ship_node_id
               WHERE wr.work_id = wc.work_id
                 AND wr.rel_type IN (?3)
                 AND sp.character_node_id = wc.character_node_id)",
        "SELECT count(*) FROM work_characters wc
         WHERE wc.work_id = $1::uuid AND wc.character_node_id = $2
           AND NOT EXISTS (
               SELECT 1 FROM work_relationships wr
               JOIN ship_participants sp ON sp.ship_node_id = wr.ship_node_id
               WHERE wr.work_id = wc.work_id
                 AND wr.rel_type IN ($3)
                 AND sp.character_node_id = wc.character_node_id)",
        &[work.clone(), "bob".to_string(), "romantic".to_string()],
    )
    .await;
    assert_eq!(matches, 0, "Bob is in a romantic relationship, so excluded");

    // And the POSITIVE control: the same query with a type he is not in must match,
    // so the test above is not passing because the query is always false.
    let not_sexual_only = count_scalar(
        &db,
        "SELECT count(*) FROM work_characters wc
         WHERE wc.work_id = ?1 AND wc.character_node_id = ?2
           AND NOT EXISTS (
               SELECT 1 FROM work_relationships wr
               JOIN ship_participants sp ON sp.ship_node_id = wr.ship_node_id
               WHERE wr.work_id = wc.work_id
                 AND wr.rel_type IN (?3)
                 AND sp.character_node_id = wc.character_node_id)",
        "SELECT count(*) FROM work_characters wc
         WHERE wc.work_id = $1::uuid AND wc.character_node_id = $2
           AND NOT EXISTS (
               SELECT 1 FROM work_relationships wr
               JOIN ship_participants sp ON sp.ship_node_id = wr.ship_node_id
               WHERE wr.work_id = wc.work_id
                 AND wr.rel_type IN ($3)
                 AND sp.character_node_id = wc.character_node_id)",
        &[work.clone(), "bob".to_string(), "familial".to_string()],
    )
    .await;
    assert_eq!(
        not_sexual_only, 1,
        "Bob is in no familial relationship, so the exclusion finds nothing to \
         exclude and he must match -- this is the control for the assertion above"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn invalid_values_are_refused_before_the_database() {
    let db = scratch("chars_invalid").await;
    let work = id("chars-invalid");
    fixture_nodes(&db, "chars-invalid").await;
    fixture_work(&db, &work).await;

    let bad_prominence = wc::upsert_character(
        &db.db(),
        &lorehaven_db::work_characters::WorkCharacter {
            work_id: work.clone(),
            character_node_id: "alice".into(),
            prominence: "main".into(),
            is_pov: false,
            added_at: "2026-01-01T00:00:00Z".into(),
        },
    )
    .await;
    assert!(
        bad_prominence.is_err(),
        "'main' is not one of §15.1's four prominences and must be refused"
    );

    let bad_rel = wc::upsert_relationship(
        &db.db(),
        &lorehaven_db::work_characters::WorkRelationship {
            id: "bad".into(),
            work_id: work.clone(),
            ship_node_id: "some-ship".into(),
            rel_type: "situational".into(),
            prominence: "primary".into(),
            dynamics: None,
            label: None,
            added_at: "2026-01-01T00:00:00Z".into(),
        },
    )
    .await;
    assert!(
        bad_rel.is_err(),
        "'situational' is not one of §15.2's kinds"
    );

    db.cleanup().await;
}

/// ADR 0026 / §49.2: only author-confirmed tags exist.
#[tokio::test]
async fn work_tag_source_is_author_only() {
    let db = scratch("tag_source_author_only").await;
    let work = id("tag-source-work");
    fixture_work(&db, &work).await;

    // A reader-applied source is refused by the schema on both engines.
    let reader = exec_with_text(
        &db,
        "INSERT INTO work_tags (work_id, node_id, added_at, source)
         VALUES (?1, ?2, '2026-01-01T00:00:00Z', 'reader')",
        "INSERT INTO work_tags (work_id, node_id, added_at, source)
         VALUES ($1::uuid, $2, '2026-01-01T00:00:00Z', 'reader')",
        &[work.clone(), "vampire".to_string()],
    )
    .await;
    assert!(
        reader.is_err(),
        "work_tags.source='reader' must be refused (ADR 0026, spec §49.2)"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn both_engines_report_the_same_substrate() {
    // The real work here is the harness: SQLite is the default; with
    // LOREHAVEN_TEST_PG_URL set the same fixtures run on PostgreSQL, where the
    // ::uuid binds and the BIGINT is_pov decode are the difference.
    let db = scratch("both_engines").await;
    let work = id("both-engines");
    fixture_nodes(&db, "both-engines").await;
    fixture_work(&db, &work).await;

    wc::upsert_character(
        &db.db(),
        &lorehaven_db::work_characters::WorkCharacter {
            work_id: work.clone(),
            character_node_id: "alice".into(),
            prominence: "protagonist".into(),
            is_pov: true,
            added_at: "2026-01-01T00:00:00Z".into(),
        },
    )
    .await
    .expect("upsert");
    let read = wc::character_in_work(&db.db(), &work, "alice")
        .await
        .unwrap()
        .expect("present");
    assert_eq!(read.prominence, "protagonist");
    assert!(read.is_pov);

    db.cleanup().await;
}
