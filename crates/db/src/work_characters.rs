//! Characters, ships, and relationships: persistence for §15.1–15.3 (M46-01).
//!
//! ## What this is for
//!
//! §15.3 specifies two query nodes, `ExistsCharacter` and `ExistsRelationship`, and
//! journey 12 (§25.1 item 12) is built on them: *"search bound character attribute
//! → exclude ship → filter by mood"*. This module stores the facts those nodes
//! read — which characters are in a work, how prominent they are, what attributes
//! they carry, and which relationships a work claims about which pairings.
//!
//! ## A ship is a participant SET; the type belongs to the work
//!
//! The same pair of characters can be romantic in one fic and platonic in another,
//! sometimes in the same fic. So a `taxonomy_nodes` row with `kind = 'ship'` is
//! the *identity* of a pairing, and `work_relationships.rel_type` is the *claim* a
//! particular work makes about it.
//!
//! `ship_participants` therefore has no surrogate id and no ordinal column. A/B and
//! B/A insert the same two rows, so they produce the same set and therefore the same
//! ship node — a writer cannot split one pairing into two nodes by choosing a
//! different order, which is the exact failure an `ord` column would invite.
//!
//! ## `work_character_attributes` is keyed WITH its character, deliberately
//!
//! §15.3: *"Never allow one character to satisfy another character's attributes."*
//!
//! The rule is about correlation, and the key enforces it structurally. With
//! `PRIMARY KEY (work_id, character_node_id, attribute_node_id)` and a **composite**
//! FK to `work_characters`, an attribute row cannot name a character who is not in
//! the work, and a join from `work_characters` on `(work_id, character_node_id)`
//! cannot reach another character's attributes — another character's attributes have
//! a different key.
//!
//! Two single-column FKs instead of the composite one would let a row name a
//! character absent from the work entirely, and every query would still return a
//! plausible answer.
//!
//! ## Dialect notes, because the columns are not uniform
//!
//! 0103 declares these tables and the two engines disagree:
//!
//! | column                   | SQLite  | PostgreSQL |
//! |--------------------------|---------|------------|
//! | every `work_id`          | TEXT    | **UUID**   |
//! | `work_characters.is_pov` | INTEGER | **BIGINT** |
//!
//! **Every `work_id` bind takes `::uuid` on PostgreSQL and every read projects
//! `work_id::text`.** Both are load-bearing: without the projection, decoding a UUID
//! column into a `String` fails even when the bind was correct.
//!
//! The node columns are **TEXT on both engines** — `taxonomy_nodes.id` is TEXT in
//! both arms of 0011 — so `character_node_id`, `ship_node_id` and
//! `attribute_node_id` take no cast anywhere. Worth stating because the instinct is
//! the opposite: an id that looks like a UUID usually is one, and here it is not.
//! Checked against both migration files.
//!
//! `is_pov` is BIGINT because PostgreSQL's INTEGER is INT4 while this store decodes
//! `i64` — an error even when the bind was right, as 0003 notes for
//! `works.version`.
//!
//! SQLite is dynamically typed, so every one of these mistakes is invisible on the
//! engine the test suite runs by default. The second-dialect gate is not optional
//! for this module.

use crate::{Backend, Database};
use anyhow::{anyhow, Result};

/// A character in a work, with the bounds §15.1 attaches to it.
///
/// `prominence` is an enum in the AST but a constrained `TEXT` column here, and the
/// store re-validates on write rather than trusting the caller: §15.1's four values
/// are load-bearing ("exclude Major Character Death" means something different when
/// the tag is incidental) and a typo would otherwise surface as a work that
/// silently fails to match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkCharacter {
    pub work_id: String,
    pub character_node_id: String,
    pub prominence: String,
    pub is_pov: bool,
    pub added_at: String,
}

/// §15.1's prominence values.
pub const PROMINENCE_VALUES: [&str; 4] = ["protagonist", "supporting", "cameo", "mentioned"];

/// §15.2's relationship kinds.
pub const REL_TYPE_VALUES: [&str; 7] = [
    "romantic",
    "platonic",
    "familial",
    "qpp",
    "sexual",
    "antagonistic",
    "other",
];

/// A relationship a work claims about a pairing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkRelationship {
    pub id: String,
    pub work_id: String,
    pub ship_node_id: String,
    pub rel_type: String,
    pub prominence: String,
    /// Comma-separated node ids. A list because §15.2 wants several dynamics on one
    /// relationship (`[enemies_to_lovers, slow_burn]`) and a single scalar column
    /// would force a second row for the same claim.
    pub dynamics: Option<String>,
    pub label: Option<String>,
    pub added_at: String,
}

/// A stored character row, decoded straight into a tuple.
///
/// `work_id::text` on the PostgreSQL arm is load-bearing (see the header). Tuples
/// rather than `sqlx::Row` getters because `query_as` lets both engines decode the
/// same SQL into the same Rust type, which is the point of the `Database::sql`
/// convention: one statement, two spellings, one result type.
type CharacterRow = (String, String, String, i64, String);

/// A stored relationship row.
type RelationshipRow = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
);

/// Insert or replace a character's presence and bounds in a work.
///
/// Upsert on `(work_id, character_node_id)`: a work has one row per character, and
/// re-tagging a character from `cameo` to `supporting` is an update of the same
/// fact, not a second row.
///
/// Returns `true` when the write happened.
pub async fn upsert_character(db: &Database, character: &WorkCharacter) -> Result<bool> {
    // Validate before the write rather than relying on the CHECK: this is an upsert,
    // and 0103's constraint fires on the INSERT path. An invalid `prominence`
    // arriving on the DO UPDATE path is a quieter failure than a rejected insert,
    // and a bad prominence is invisible in every query result — the work simply
    // stops matching.
    if !PROMINENCE_VALUES.contains(&character.prominence.as_str()) {
        return Err(anyhow!(
            "prominence must be one of {PROMINENCE_VALUES:?}, got {:?}",
            character.prominence
        ));
    }

    let sql = db.sql(
        "INSERT INTO work_characters
             (work_id, character_node_id, prominence, is_pov, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (work_id, character_node_id) DO UPDATE SET
             prominence = excluded.prominence,
             is_pov     = excluded.is_pov,
             added_at   = excluded.added_at",
        "INSERT INTO work_characters
             (work_id, character_node_id, prominence, is_pov, added_at)
         VALUES ($1::uuid, $2, $3, $4, $5)
         ON CONFLICT (work_id, character_node_id) DO UPDATE SET
             prominence = excluded.prominence,
             is_pov     = excluded.is_pov,
             added_at   = excluded.added_at",
    );

    // Executed per arm because a `sqlx::Query` is parameterised by its database
    // type: one value cannot run against both a `Pool<Sqlite>` and a
    // `Pool<Postgres>`. The compiler rejects it as `expected Sqlite, found
    // Postgres` — the type system doing exactly the job the dynamically typed
    // engine cannot. `work_coordinates.rs` builds per arm for the same reason.
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&character.work_id)
                .bind(&character.character_node_id)
                .bind(&character.prominence)
                .bind(i64::from(character.is_pov))
                .bind(&character.added_at)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&character.work_id)
                .bind(&character.character_node_id)
                .bind(&character.prominence)
                .bind(i64::from(character.is_pov))
                .bind(&character.added_at)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(true)
}

/// Every character in a work, most prominent first.
///
/// Ordered by prominence rank rather than alphabetically: a browse page listing
/// characters should lead with the protagonist, and §15.1's four values are a rank
/// order by construction.
pub async fn characters_for_work(db: &Database, work_id: &str) -> Result<Vec<WorkCharacter>> {
    let sql = db.sql(
        "SELECT work_id, character_node_id, prominence, is_pov, added_at
         FROM work_characters
         WHERE work_id = ?1
         ORDER BY CASE prominence
                    WHEN 'protagonist' THEN 0
                    WHEN 'supporting'  THEN 1
                    WHEN 'cameo'       THEN 2
                    ELSE 3
                  END,
                  character_node_id",
        "SELECT work_id::text, character_node_id, prominence, is_pov, added_at
         FROM work_characters
         WHERE work_id = $1::uuid
         ORDER BY CASE prominence
                    WHEN 'protagonist' THEN 0
                    WHEN 'supporting'  THEN 1
                    WHEN 'cameo'       THEN 2
                    ELSE 3
                  END,
                  character_node_id",
    );
    match db.backend() {
        Backend::Sqlite => {
            let rows: Vec<CharacterRow> = sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            Ok(rows.into_iter().map(row_to_character).collect())
        }
        Backend::Postgres => {
            let rows: Vec<CharacterRow> = sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            Ok(rows.into_iter().map(row_to_character).collect())
        }
    }
}

/// One character in a work.
pub async fn character_in_work(
    db: &Database,
    work_id: &str,
    character_node_id: &str,
) -> Result<Option<WorkCharacter>> {
    let sql = db.sql(
        "SELECT work_id, character_node_id, prominence, is_pov, added_at
         FROM work_characters WHERE work_id = ?1 AND character_node_id = ?2",
        "SELECT work_id::text, character_node_id, prominence, is_pov, added_at
         FROM work_characters WHERE work_id = $1::uuid AND character_node_id = $2",
    );
    match db.backend() {
        Backend::Sqlite => {
            let row: Option<CharacterRow> = sqlx::query_as(&sql)
                .bind(work_id)
                .bind(character_node_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            Ok(row.map(row_to_character))
        }
        Backend::Postgres => {
            let row: Option<CharacterRow> = sqlx::query_as(&sql)
                .bind(work_id)
                .bind(character_node_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?;
            Ok(row.map(row_to_character))
        }
    }
}

fn row_to_character(row: CharacterRow) -> WorkCharacter {
    let (work_id, character_node_id, prominence, is_pov, added_at) = row;
    WorkCharacter {
        work_id,
        character_node_id,
        prominence,
        is_pov: is_pov != 0,
        added_at,
    }
}

/// Attach an attribute to a character in a work.
///
/// The composite FK makes this fail when the character is not in the work, which is
/// the point: an attribute is a fact *about a character present in this work*, and a
/// row asserting otherwise is a data-entry error worth rejecting loudly rather than a
/// row that quietly never matches.
///
/// Returns `false` when the attribute was already present, so the caller can tell an
/// idempotent re-tag from a new claim.
pub async fn add_character_attribute(
    db: &Database,
    work_id: &str,
    character_node_id: &str,
    attribute_node_id: &str,
    added_at: &str,
) -> Result<bool> {
    // `DO NOTHING` rather than `DO UPDATE`: there is nothing to update. An attribute
    // row is (work, character, attribute) with no payload, so the only alternative
    // to "already there" is an error the caller did not ask for.
    let sql = db.sql(
        "INSERT INTO work_character_attributes
             (work_id, character_node_id, attribute_node_id, added_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (work_id, character_node_id, attribute_node_id) DO NOTHING",
        "INSERT INTO work_character_attributes
             (work_id, character_node_id, attribute_node_id, added_at)
         VALUES ($1::uuid, $2, $3, $4)
         ON CONFLICT (work_id, character_node_id, attribute_node_id) DO NOTHING",
    );
    // No casts on the node columns: TEXT on both engines (0011).
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(work_id)
            .bind(character_node_id)
            .bind(attribute_node_id)
            .bind(added_at)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(work_id)
            .bind(character_node_id)
            .bind(attribute_node_id)
            .bind(added_at)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// The attributes of one character in one work.
pub async fn attributes_of_character(
    db: &Database,
    work_id: &str,
    character_node_id: &str,
) -> Result<Vec<String>> {
    let sql = db.sql(
        "SELECT attribute_node_id FROM work_character_attributes
         WHERE work_id = ?1 AND character_node_id = ?2 ORDER BY attribute_node_id",
        "SELECT attribute_node_id FROM work_character_attributes
         WHERE work_id = $1::uuid AND character_node_id = $2 ORDER BY attribute_node_id",
    );
    match db.backend() {
        Backend::Sqlite => Ok(sqlx::query_scalar(&sql)
            .bind(work_id)
            .bind(character_node_id)
            .fetch_all(db.sqlite_pool().expect("sqlite handle"))
            .await?),
        Backend::Postgres => Ok(sqlx::query_scalar(&sql)
            .bind(work_id)
            .bind(character_node_id)
            .fetch_all(db.postgres_pool().expect("postgres handle"))
            .await?),
    }
}

/// Remove a character's attribute.
///
/// Separate from [`upsert_character`] because removing an attribute must NOT remove
/// the character: "Alice, but no longer a vampire" and "Alice is not in this work"
/// are different edits, and conflating them would silently un-attribute a character
/// who is still present.
pub async fn remove_character_attribute(
    db: &Database,
    work_id: &str,
    character_node_id: &str,
    attribute_node_id: &str,
) -> Result<bool> {
    let sql = db.sql(
        "DELETE FROM work_character_attributes
         WHERE work_id = ?1 AND character_node_id = ?2 AND attribute_node_id = ?3",
        "DELETE FROM work_character_attributes
         WHERE work_id = $1::uuid AND character_node_id = $2 AND attribute_node_id = $3",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(work_id)
            .bind(character_node_id)
            .bind(attribute_node_id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(work_id)
            .bind(character_node_id)
            .bind(attribute_node_id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// Remove a character from a work.
///
/// Cascades to that character's attributes via the composite FK, but deliberately
/// NOT to `ship_participants`: a ship node is shared across works, so removing Alice
/// from one fic must not edit a pairing that other fics also use. If the ship is
/// genuinely gone, its node is deleted separately.
pub async fn remove_character(
    db: &Database,
    work_id: &str,
    character_node_id: &str,
) -> Result<bool> {
    let sql = db.sql(
        "DELETE FROM work_characters WHERE work_id = ?1 AND character_node_id = ?2",
        "DELETE FROM work_characters WHERE work_id = $1::uuid AND character_node_id = $2",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(work_id)
            .bind(character_node_id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(work_id)
            .bind(character_node_id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// Add a character to a ship's participant set.
///
/// Idempotent, and deliberately order-independent: the pair of rows *is* the ship, so
/// inserting "alice then bob" and "bob then alice" leave the same state.
pub async fn add_ship_participant(
    db: &Database,
    ship_node_id: &str,
    character_node_id: &str,
) -> Result<bool> {
    let sql = db.sql(
        "INSERT INTO ship_participants (ship_node_id, character_node_id)
         VALUES (?1, ?2)
         ON CONFLICT (ship_node_id, character_node_id) DO NOTHING",
        "INSERT INTO ship_participants (ship_node_id, character_node_id)
         VALUES ($1, $2)
         ON CONFLICT (ship_node_id, character_node_id) DO NOTHING",
    );
    // No casts: both columns are TEXT on both engines (0011).
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(ship_node_id)
            .bind(character_node_id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(ship_node_id)
            .bind(character_node_id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// A ship's participants, sorted.
///
/// Sorted because the set is the identity: two callers reading the same ship must see
/// the same order, or the node would have a shape that depends on insertion history.
pub async fn ship_participants(db: &Database, ship_node_id: &str) -> Result<Vec<String>> {
    // No dialect arm is needed for the *shape* — both columns are TEXT on both
    // engines — but the placeholder still must be rewritten. `Database::sql` is what
    // rewrites `?n` to `$n`; a bare `?1` string sent straight to the driver is
    // handed to PostgreSQL literally, which rejects it as `operator does not exist:
    // ? integer`. Caught by the second-dialect gate, and invisible on SQLite because
    // SQLite understands `?1` natively.
    let sql = db.sql(
        "SELECT character_node_id FROM ship_participants
         WHERE ship_node_id = ?1 ORDER BY character_node_id",
        "SELECT character_node_id FROM ship_participants
         WHERE ship_node_id = $1 ORDER BY character_node_id",
    );
    match db.backend() {
        Backend::Sqlite => Ok(sqlx::query_scalar(&sql)
            .bind(ship_node_id)
            .fetch_all(db.sqlite_pool().expect("sqlite handle"))
            .await?),
        Backend::Postgres => Ok(sqlx::query_scalar(&sql)
            .bind(ship_node_id)
            .fetch_all(db.postgres_pool().expect("postgres handle"))
            .await?),
    }
}

/// Remove a character from a ship's participant set.
///
/// Leaves the ship node in place with a smaller set, which is a legitimate state: a
/// duo tag that turns out not to be a duo. Deleting the node itself cascades to the
/// remaining participants and to `work_relationships`.
pub async fn remove_ship_participant(
    db: &Database,
    ship_node_id: &str,
    character_node_id: &str,
) -> Result<bool> {
    // Same `?n` rewrite reason as [`ship_participants`]: both columns are TEXT on
    // both engines, but the placeholder still has to go through `Database::sql`.
    let sql = db.sql(
        "DELETE FROM ship_participants
         WHERE ship_node_id = ?1 AND character_node_id = ?2",
        "DELETE FROM ship_participants
         WHERE ship_node_id = $1 AND character_node_id = $2",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(ship_node_id)
            .bind(character_node_id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(ship_node_id)
            .bind(character_node_id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// Record a work's claim about a pairing.
///
/// `id` is the caller's, because a relationship is a row that has identity before the
/// database sees it: the ship node's participants and the claim must agree, and they
/// are written in one transaction.
///
/// Upsert on `(work_id, ship_node_id, rel_type)`: one work makes one claim of one kind
/// about one pairing. Without that UNIQUE a work could hold both `romantic` and
/// `platonic` rows for the same ship, and a `NOT ... type:romantic` exclusion would
/// then silently exclude the work anyway — the exclusion would be wrong in a way no
/// query can report.
pub async fn upsert_relationship(db: &Database, rel: &WorkRelationship) -> Result<bool> {
    if !REL_TYPE_VALUES.contains(&rel.rel_type.as_str()) {
        return Err(anyhow!(
            "rel_type must be one of {REL_TYPE_VALUES:?}, got {:?}",
            rel.rel_type
        ));
    }
    let sql = db.sql(
        "INSERT INTO work_relationships
             (id, work_id, ship_node_id, rel_type, prominence, dynamics, label, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (work_id, ship_node_id, rel_type) DO UPDATE SET
             prominence = excluded.prominence,
             dynamics   = excluded.dynamics,
             label      = excluded.label,
             added_at   = excluded.added_at",
        "INSERT INTO work_relationships
             (id, work_id, ship_node_id, rel_type, prominence, dynamics, label, added_at)
         VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (work_id, ship_node_id, rel_type) DO UPDATE SET
             prominence = excluded.prominence,
             dynamics   = excluded.dynamics,
             label      = excluded.label,
             added_at   = excluded.added_at",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&rel.id)
                .bind(&rel.work_id)
                .bind(&rel.ship_node_id)
                .bind(&rel.rel_type)
                .bind(&rel.prominence)
                .bind(&rel.dynamics)
                .bind(&rel.label)
                .bind(&rel.added_at)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&rel.id)
                .bind(&rel.work_id)
                .bind(&rel.ship_node_id)
                .bind(&rel.rel_type)
                .bind(&rel.prominence)
                .bind(&rel.dynamics)
                .bind(&rel.label)
                .bind(&rel.added_at)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(true)
}

/// Every relationship a work claims.
pub async fn relationships_for_work(db: &Database, work_id: &str) -> Result<Vec<WorkRelationship>> {
    let sql = db.sql(
        "SELECT id, work_id, ship_node_id, rel_type, prominence, dynamics, label, added_at
         FROM work_relationships WHERE work_id = ?1 ORDER BY ship_node_id, rel_type",
        "SELECT id, work_id::text, ship_node_id, rel_type, prominence, dynamics, label, added_at
         FROM work_relationships WHERE work_id = $1::uuid ORDER BY ship_node_id, rel_type",
    );
    match db.backend() {
        Backend::Sqlite => {
            let rows: Vec<RelationshipRow> = sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            Ok(rows.into_iter().map(row_to_relationship).collect())
        }
        Backend::Postgres => {
            let rows: Vec<RelationshipRow> = sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            Ok(rows.into_iter().map(row_to_relationship).collect())
        }
    }
}

/// The relationships of a work that include a given character.
///
/// The store-side counterpart of the compiler's `sp.character_node_id` arm: the join
/// through `ship_participants` is what makes "involving X" a statement about the
/// pairing rather than about a ship node by name.
pub async fn relationships_involving(
    db: &Database,
    work_id: &str,
    character_node_id: &str,
) -> Result<Vec<WorkRelationship>> {
    let sql = db.sql(
        "SELECT r.id, r.work_id, r.ship_node_id, r.rel_type, r.prominence,
                r.dynamics, r.label, r.added_at
         FROM work_relationships r
         JOIN ship_participants sp ON sp.ship_node_id = r.ship_node_id
         WHERE r.work_id = ?1 AND sp.character_node_id = ?2
         ORDER BY r.ship_node_id, r.rel_type",
        "SELECT r.id, r.work_id::text, r.ship_node_id, r.rel_type, r.prominence,
                r.dynamics, r.label, r.added_at
         FROM work_relationships r
         JOIN ship_participants sp ON sp.ship_node_id = r.ship_node_id
         WHERE r.work_id = $1::uuid AND sp.character_node_id = $2
         ORDER BY r.ship_node_id, r.rel_type",
    );
    match db.backend() {
        Backend::Sqlite => {
            let rows: Vec<RelationshipRow> = sqlx::query_as(&sql)
                .bind(work_id)
                .bind(character_node_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?;
            Ok(rows.into_iter().map(row_to_relationship).collect())
        }
        Backend::Postgres => {
            let rows: Vec<RelationshipRow> = sqlx::query_as(&sql)
                .bind(work_id)
                .bind(character_node_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?;
            Ok(rows.into_iter().map(row_to_relationship).collect())
        }
    }
}

fn row_to_relationship(row: RelationshipRow) -> WorkRelationship {
    let (id, work_id, ship_node_id, rel_type, prominence, dynamics, label, added_at) = row;
    WorkRelationship {
        id,
        work_id,
        ship_node_id,
        rel_type,
        prominence,
        dynamics,
        label,
        added_at,
    }
}
