//! Characters, relationships, and their facets: persistence for §6.4 (M6-05).
//!
//! ## What this is for
//!
//! Spec §6.4: a **character_assertion** is *"prominence + roles + attributes"*, and
//! a **relationship_assertion** is *"participants (a set, >2 supported) + kind +
//! prominence + dynamics"*. This module stores those facts: which characters are in
//! a work, how prominent they are, which roles and attributes they carry, and which
//! relationships a work claims about which pairings, with which dynamics.
//!
//! The query nodes that read them (`ExistsCharacter`, `ExistsRelationship`) are
//! §11.2's, and `main_char_attr` — the highest-scored character's attribute — is
//! compiled from the same rows in M7.
//!
//! ## The three facets have one shape
//!
//! A role, an attribute and a dynamic are each *a named thing this work says about
//! this character (or this pairing)*. All three are therefore a set of
//! `canonical_entities` references, and **none of them is a compiled-in
//! vocabulary**: M6-01 made one table own names, and a `CHECK ('mentor', …)` here
//! would be a second vocabulary that a curator could not extend without a
//! migration. 0127 says the same thing at the schema's own level.
//!
//! ## A pairing is a participant SET; the type belongs to the work
//!
//! The same pair of characters can be romantic in one fic and platonic in another,
//! sometimes in the same fic. So a `canonical_entities` row with
//! `kind = 'relationship'` (§6.4's kind list) is the *identity* of a pairing, and
//! `work_relationships.rel_type` is the *claim* a particular work makes about it.
//!
//! `relationship_participants` therefore has no surrogate id and no ordinal column.
//! A/B and B/A insert the same two rows, so they produce the same set and therefore
//! the same relationship node — a writer cannot split one pairing into two nodes by
//! choosing a different order, which is the exact failure an `ord` column would
//! invite. A third participant is one more row: >2 is the same table, not a special
//! case.
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
//! The node columns are **TEXT on both engines** — `canonical_entities.id` is TEXT in
//! both arms of 0011 — so `character_node_id`, `relationship_node_id`,
//! `attribute_node_id`, `role_node_id` and `dynamic_node_id` take no cast anywhere.
//! Worth stating because the instinct is
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
    /// §6.4's third facet: what this character *is* in this work (`mentor`, `rival`,
    /// `narrator`). A set, sorted, and populated by the bulk read — the same shape as a
    /// relationship's dynamics, because both are "named things this claim carries".
    pub roles: Vec<String>,
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
    /// The pairing's identity: a `canonical_entities` row of kind
    /// `relationship` (§6.4's kind list — the estate used to spell it `ship`).
    pub relationship_node_id: String,
    pub rel_type: String,
    pub prominence: String,
    /// The dynamics this claim carries, as a set of entity ids (§6.4).
    ///
    /// Loaded from `work_relationship_dynamics`, ordered, and empty when the
    /// claim carries none. It was a comma-separated string until 0127: a list
    /// in a string cannot be joined, cannot be indexed, and `a,b` and `b,a` are
    /// different strings describing the same claim.
    pub dynamics: Vec<String>,
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
    let rows: Vec<CharacterRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    let mut characters: Vec<WorkCharacter> = rows.into_iter().map(row_to_character).collect();
    attach_roles(db, work_id, &mut characters).await?;
    Ok(characters)
}

/// Fill in each character's roles with one query per work, not one per character.
///
/// The same short-circuit `attach_dynamics` uses: a page shows one work, so "no
/// characters" means no query and otherwise one query answers all of them. A
/// `LEFT JOIN` onto the set would return one row per role and force the caller to
/// regroup, which is how a character silently multiplies.
async fn attach_roles(
    db: &Database,
    work_id: &str,
    characters: &mut [WorkCharacter],
) -> Result<()> {
    if characters.is_empty() {
        return Ok(());
    }
    let sql = db.sql(
        "SELECT character_node_id, role_node_id FROM work_character_roles
         WHERE work_id = ?1 ORDER BY character_node_id, role_node_id",
        "SELECT character_node_id, role_node_id FROM work_character_roles
         WHERE work_id = $1::uuid ORDER BY character_node_id, role_node_id",
    );
    let pairs: Vec<(String, String)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    for character in characters.iter_mut() {
        character.roles = pairs
            .iter()
            .filter(|(node, _)| node == &character.character_node_id)
            .map(|(_, role)| role.clone())
            .collect();
    }
    Ok(())
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
        roles: Vec::new(),
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

/// Attach a role to a character in a work (§6.4: a character_assertion's roles).
///
/// A role is a named thing — `mentor`, `love_interest`, `antagonist` — and it is a
/// `canonical_entities` reference for the reason the module header gives: the
/// instance owns its vocabulary, and a role is not a place to compile one in.
///
/// The composite FK makes it fail when the character is not in the work, exactly as
/// an attribute does: a role is a fact *about a character present in this work*.
///
/// Returns `false` when the character already held the role.
pub async fn add_character_role(
    db: &Database,
    work_id: &str,
    character_node_id: &str,
    role_node_id: &str,
    added_at: &str,
) -> Result<bool> {
    let sql = db.sql(
        "INSERT INTO work_character_roles
             (work_id, character_node_id, role_node_id, added_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (work_id, character_node_id, role_node_id) DO NOTHING",
        "INSERT INTO work_character_roles
             (work_id, character_node_id, role_node_id, added_at)
         VALUES ($1::uuid, $2, $3, $4)
         ON CONFLICT (work_id, character_node_id, role_node_id) DO NOTHING",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(work_id)
            .bind(character_node_id)
            .bind(role_node_id)
            .bind(added_at)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(work_id)
            .bind(character_node_id)
            .bind(role_node_id)
            .bind(added_at)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// The roles of one character in one work, sorted.
///
/// Sorted for the same reason a participant set is: two readers of the same
/// assertion must see the same order, or the shape of the claim depends on the
/// history of its edits.
pub async fn roles_of_character(
    db: &Database,
    work_id: &str,
    character_node_id: &str,
) -> Result<Vec<String>> {
    let sql = db.sql(
        "SELECT role_node_id FROM work_character_roles
         WHERE work_id = ?1 AND character_node_id = ?2 ORDER BY role_node_id",
        "SELECT role_node_id FROM work_character_roles
         WHERE work_id = $1::uuid AND character_node_id = $2 ORDER BY role_node_id",
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

/// Remove a character's role. The character and the other facets stay.
pub async fn remove_character_role(
    db: &Database,
    work_id: &str,
    character_node_id: &str,
    role_node_id: &str,
) -> Result<bool> {
    let sql = db.sql(
        "DELETE FROM work_character_roles
         WHERE work_id = ?1 AND character_node_id = ?2 AND role_node_id = ?3",
        "DELETE FROM work_character_roles
         WHERE work_id = $1::uuid AND character_node_id = $2 AND role_node_id = $3",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(work_id)
            .bind(character_node_id)
            .bind(role_node_id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(work_id)
            .bind(character_node_id)
            .bind(role_node_id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// Remove a character from a work.
///
/// Cascades to that character's attributes and roles via the composite FK, but
/// deliberately NOT to `relationship_participants`: a relationship node is shared
/// across works, so removing Alice from one fic must not edit a pairing that other
/// fics also use. If the pairing is genuinely gone, its node is deleted separately.
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

/// Add a character to a relationship's participant set.
///
/// Idempotent, and deliberately order-independent: the pair of rows *is* the
/// pairing, so inserting "alice then bob" and "bob then alice" leave the same
/// state. **A set, so more than two participants is the same table**: a
/// three-way pairing is three rows, and nothing in the schema counts them
/// (§6.4's ">2 supported").
pub async fn add_relationship_participant(
    db: &Database,
    relationship_node_id: &str,
    character_node_id: &str,
) -> Result<bool> {
    let sql = db.sql(
        "INSERT INTO relationship_participants (relationship_node_id, character_node_id)
         VALUES (?1, ?2)
         ON CONFLICT (relationship_node_id, character_node_id) DO NOTHING",
        "INSERT INTO relationship_participants (relationship_node_id, character_node_id)
         VALUES ($1, $2)
         ON CONFLICT (relationship_node_id, character_node_id) DO NOTHING",
    );
    // No casts: both columns are TEXT on both engines (0011).
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(relationship_node_id)
            .bind(character_node_id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(relationship_node_id)
            .bind(character_node_id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// A pairing's participants, sorted.
///
/// Sorted because the set is the identity: two callers reading the same pairing
/// must see the same order, or the node would have a shape that depends on
/// insertion history.
pub async fn relationship_participants(
    db: &Database,
    relationship_node_id: &str,
) -> Result<Vec<String>> {
    // No dialect arm is needed for the *shape* — both columns are TEXT on both
    // engines — but the placeholder still must be rewritten. `Database::sql` is what
    // rewrites `?n` to `$n`; a bare `?1` string sent straight to the driver is
    // handed to PostgreSQL literally, which rejects it as `operator does not exist:
    // ? integer`. Caught by the second-dialect gate, and invisible on SQLite because
    // SQLite understands `?1` natively.
    let sql = db.sql(
        "SELECT character_node_id FROM relationship_participants
         WHERE relationship_node_id = ?1 ORDER BY character_node_id",
        "SELECT character_node_id FROM relationship_participants
         WHERE relationship_node_id = $1 ORDER BY character_node_id",
    );
    match db.backend() {
        Backend::Sqlite => Ok(sqlx::query_scalar(&sql)
            .bind(relationship_node_id)
            .fetch_all(db.sqlite_pool().expect("sqlite handle"))
            .await?),
        Backend::Postgres => Ok(sqlx::query_scalar(&sql)
            .bind(relationship_node_id)
            .fetch_all(db.postgres_pool().expect("postgres handle"))
            .await?),
    }
}

/// Remove a character from a pairing's participant set.
///
/// Leaves the pairing's node in place with a smaller set, which is a legitimate
/// state: a duo tag that turns out not to be a duo. Deleting the node itself
/// cascades to the remaining participants and to `work_relationships`.
pub async fn remove_relationship_participant(
    db: &Database,
    relationship_node_id: &str,
    character_node_id: &str,
) -> Result<bool> {
    // Same `?n` rewrite reason as [`relationship_participants`]: both columns are
    // TEXT on both engines, but the placeholder still has to go through
    // `Database::sql`.
    let sql = db.sql(
        "DELETE FROM relationship_participants
         WHERE relationship_node_id = ?1 AND character_node_id = ?2",
        "DELETE FROM relationship_participants
         WHERE relationship_node_id = $1 AND character_node_id = $2",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(relationship_node_id)
            .bind(character_node_id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(relationship_node_id)
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
/// database sees it: the pairing's participants and the claim must agree, and they
/// are written in one transaction.
///
/// Upsert on `(work_id, relationship_node_id, rel_type)`: one work makes one claim of
/// one kind about one pairing. Without that UNIQUE a work could hold both `romantic`
/// and `platonic` rows for the same pairing, and a `NOT ... type:romantic` exclusion
/// would then silently exclude the work anyway — the exclusion would be wrong in a
/// way no query can report.
///
/// **The dynamics are not written here.** They are a set in
/// `work_relationship_dynamics`, and a set is replaced with
/// [`set_relationship_dynamics`] by a caller that knows which claim it means;
/// upserting a row and replacing its dynamics in one statement would make "change
/// the prominence" and "change the dynamics" the same edit.
pub async fn upsert_relationship(db: &Database, rel: &WorkRelationship) -> Result<bool> {
    if !REL_TYPE_VALUES.contains(&rel.rel_type.as_str()) {
        return Err(anyhow!(
            "rel_type must be one of {REL_TYPE_VALUES:?}, got {:?}",
            rel.rel_type
        ));
    }
    let sql = db.sql(
        "INSERT INTO work_relationships
             (id, work_id, relationship_node_id, rel_type, prominence, label, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (work_id, relationship_node_id, rel_type) DO UPDATE SET
             prominence = excluded.prominence,
             label      = excluded.label,
             added_at   = excluded.added_at",
        "INSERT INTO work_relationships
             (id, work_id, relationship_node_id, rel_type, prominence, label, added_at)
         VALUES ($1, $2::uuid, $3, $4, $5, $6, $7)
         ON CONFLICT (work_id, relationship_node_id, rel_type) DO UPDATE SET
             prominence = excluded.prominence,
             label      = excluded.label,
             added_at   = excluded.added_at",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&rel.id)
                .bind(&rel.work_id)
                .bind(&rel.relationship_node_id)
                .bind(&rel.rel_type)
                .bind(&rel.prominence)
                .bind(&rel.label)
                .bind(&rel.added_at)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&rel.id)
                .bind(&rel.work_id)
                .bind(&rel.relationship_node_id)
                .bind(&rel.rel_type)
                .bind(&rel.prominence)
                .bind(&rel.label)
                .bind(&rel.added_at)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(true)
}

/// Replace a claim's dynamics with `dynamics`.
///
/// Clear-and-fill, for the same reason the closure is (M6-03): a dynamics set is
/// the whole of what this claim says about the pairing's dynamics, so a stale row
/// is a dynamic the claim no longer has. Done in one transaction so a reader never
/// sees the claim with no dynamics at all.
///
/// Returns the number of dynamics the claim now carries.
pub async fn set_relationship_dynamics(
    db: &Database,
    relationship_id: &str,
    dynamics: &[String],
    added_at: &str,
) -> Result<usize> {
    let delete_sql = db.sql(
        "DELETE FROM work_relationship_dynamics WHERE relationship_id = ?1",
        "DELETE FROM work_relationship_dynamics WHERE relationship_id = $1",
    );
    let insert_sql = db.sql(
        "INSERT INTO work_relationship_dynamics
             (relationship_id, dynamic_node_id, added_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT (relationship_id, dynamic_node_id) DO NOTHING",
        "INSERT INTO work_relationship_dynamics
             (relationship_id, dynamic_node_id, added_at)
         VALUES ($1, $2, $3)
         ON CONFLICT (relationship_id, dynamic_node_id) DO NOTHING",
    );

    let inserted = match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            let mut tx = pool.begin().await?;
            sqlx::query(&delete_sql)
                .bind(relationship_id)
                .execute(&mut *tx)
                .await?;
            let mut inserted = 0;
            for dynamic in dynamics {
                inserted += sqlx::query(&insert_sql)
                    .bind(relationship_id)
                    .bind(dynamic)
                    .bind(added_at)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
            }
            tx.commit().await?;
            inserted
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            let mut tx = pool.begin().await?;
            sqlx::query(&delete_sql)
                .bind(relationship_id)
                .execute(&mut *tx)
                .await?;
            let mut inserted = 0;
            for dynamic in dynamics {
                inserted += sqlx::query(&insert_sql)
                    .bind(relationship_id)
                    .bind(dynamic)
                    .bind(added_at)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
            }
            tx.commit().await?;
            inserted
        }
    };
    Ok(inserted as usize)
}

/// A claim's dynamics, sorted.
pub async fn dynamics_of_relationship(
    db: &Database,
    relationship_id: &str,
) -> Result<Vec<String>> {
    let sql = db.sql(
        "SELECT dynamic_node_id FROM work_relationship_dynamics
         WHERE relationship_id = ?1 ORDER BY dynamic_node_id",
        "SELECT dynamic_node_id FROM work_relationship_dynamics
         WHERE relationship_id = $1 ORDER BY dynamic_node_id",
    );
    match db.backend() {
        Backend::Sqlite => Ok(sqlx::query_scalar(&sql)
            .bind(relationship_id)
            .fetch_all(db.sqlite_pool().expect("sqlite handle"))
            .await?),
        Backend::Postgres => Ok(sqlx::query_scalar(&sql)
            .bind(relationship_id)
            .fetch_all(db.postgres_pool().expect("postgres handle"))
            .await?),
    }
}

/// Add one dynamic to a claim, idempotently.
///
/// Returns `false` when the claim already carried it, so a caller can tell a new
/// claim from a re-assertion.
pub async fn add_relationship_dynamic(
    db: &Database,
    relationship_id: &str,
    dynamic_node_id: &str,
    added_at: &str,
) -> Result<bool> {
    let sql = db.sql(
        "INSERT INTO work_relationship_dynamics
             (relationship_id, dynamic_node_id, added_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT (relationship_id, dynamic_node_id) DO NOTHING",
        "INSERT INTO work_relationship_dynamics
             (relationship_id, dynamic_node_id, added_at)
         VALUES ($1, $2, $3)
         ON CONFLICT (relationship_id, dynamic_node_id) DO NOTHING",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(relationship_id)
            .bind(dynamic_node_id)
            .bind(added_at)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(relationship_id)
            .bind(dynamic_node_id)
            .bind(added_at)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// Remove one dynamic from a claim. The claim itself stays.
pub async fn remove_relationship_dynamic(
    db: &Database,
    relationship_id: &str,
    dynamic_node_id: &str,
) -> Result<bool> {
    let sql = db.sql(
        "DELETE FROM work_relationship_dynamics
         WHERE relationship_id = ?1 AND dynamic_node_id = ?2",
        "DELETE FROM work_relationship_dynamics
         WHERE relationship_id = $1 AND dynamic_node_id = $2",
    );
    let affected = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(relationship_id)
            .bind(dynamic_node_id)
            .execute(db.sqlite_pool().expect("sqlite handle"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(relationship_id)
            .bind(dynamic_node_id)
            .execute(db.postgres_pool().expect("postgres handle"))
            .await?
            .rows_affected(),
    };
    Ok(affected > 0)
}

/// Every relationship a work claims, each with its dynamics.
pub async fn relationships_for_work(db: &Database, work_id: &str) -> Result<Vec<WorkRelationship>> {
    let sql = db.sql(
        "SELECT id, work_id, relationship_node_id, rel_type, prominence, label, added_at
         FROM work_relationships
         WHERE work_id = ?1
         ORDER BY relationship_node_id, rel_type",
        "SELECT id, work_id::text, relationship_node_id, rel_type, prominence, label, added_at
         FROM work_relationships
         WHERE work_id = $1::uuid
         ORDER BY relationship_node_id, rel_type",
    );
    let rows: Vec<RelationshipRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    let mut relationships: Vec<WorkRelationship> =
        rows.into_iter().map(row_to_relationship).collect();
    attach_dynamics(db, &mut relationships).await?;
    Ok(relationships)
}

/// The relationships of a work that include a given character.
///
/// The store-side counterpart of the compiler's `sp.character_node_id` arm: the
/// join through `relationship_participants` is what makes "involving X" a
/// statement about the pairing rather than about a pairing node by name.
pub async fn relationships_involving(
    db: &Database,
    work_id: &str,
    character_node_id: &str,
) -> Result<Vec<WorkRelationship>> {
    let sql = db.sql(
        "SELECT r.id, r.work_id, r.relationship_node_id, r.rel_type, r.prominence,
                r.label, r.added_at
         FROM work_relationships r
         JOIN relationship_participants sp
           ON sp.relationship_node_id = r.relationship_node_id
         WHERE r.work_id = ?1 AND sp.character_node_id = ?2
         ORDER BY r.relationship_node_id, r.rel_type",
        "SELECT r.id, r.work_id::text, r.relationship_node_id, r.rel_type, r.prominence,
                r.label, r.added_at
         FROM work_relationships r
         JOIN relationship_participants sp
           ON sp.relationship_node_id = r.relationship_node_id
         WHERE r.work_id = $1::uuid AND sp.character_node_id = $2
         ORDER BY r.relationship_node_id, r.rel_type",
    );
    let rows: Vec<RelationshipRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .bind(character_node_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .bind(character_node_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    let mut relationships: Vec<WorkRelationship> =
        rows.into_iter().map(row_to_relationship).collect();
    attach_dynamics(db, &mut relationships).await?;
    Ok(relationships)
}

/// Fill in each relationship's dynamics with one query per work rather than one per
/// claim.
///
/// A work usually carries a handful of claims and a page shows one work, so the
/// short-circuit is honest: no claims means no query, and otherwise one query
/// answers all of them. The alternative — a `LEFT JOIN` onto the set — would return
/// one row per dynamic and force the caller to regroup, which is how a claim
/// silently multiplies.
async fn attach_dynamics(
    db: &Database,
    relationships: &mut [WorkRelationship],
) -> Result<()> {
    if relationships.is_empty() {
        return Ok(());
    }
    let placeholders: Vec<String> = (1..=relationships.len()).map(|i| format!("?{i}")).collect();
    let pg_placeholders: Vec<String> = (1..=relationships.len()).map(|i| format!("${i}")).collect();
    let ids: Vec<&str> = relationships.iter().map(|r| r.id.as_str()).collect();
    // The two statements are owned before `db.sql` picks one: it returns a borrow of the string it
    // was given, so a `format!` passed inline is dropped while the statement is still in use.
    let sqlite_sql = format!(
        "SELECT relationship_id, dynamic_node_id FROM work_relationship_dynamics
         WHERE relationship_id IN ({})
         ORDER BY relationship_id, dynamic_node_id",
        placeholders.join(", ")
    );
    let postgres_sql = format!(
        "SELECT relationship_id, dynamic_node_id FROM work_relationship_dynamics
         WHERE relationship_id IN ({})
         ORDER BY relationship_id, dynamic_node_id",
        pg_placeholders.join(", ")
    );
    let sql = db.sql(&sqlite_sql, &postgres_sql);
    let pairs: Vec<(String, String)> = match db.backend() {
        Backend::Sqlite => {
            let mut q = sqlx::query_as(&sql);
            for id in &ids {
                q = q.bind(*id);
            }
            q.fetch_all(db.sqlite_pool().expect("sqlite handle")).await?
        }
        Backend::Postgres => {
            let mut q = sqlx::query_as(&sql);
            for id in &ids {
                q = q.bind(*id);
            }
            q.fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    for relationship in relationships.iter_mut() {
        relationship.dynamics = pairs
            .iter()
            .filter(|(id, _)| id == &relationship.id)
            .map(|(_, dynamic)| dynamic.clone())
            .collect();
    }
    Ok(())
}

fn row_to_relationship(row: RelationshipRow) -> WorkRelationship {
    let (id, work_id, relationship_node_id, rel_type, prominence, label, added_at) = row;
    WorkRelationship {
        id,
        work_id,
        relationship_node_id,
        rel_type,
        prominence,
        dynamics: Vec::new(),
        label,
        added_at,
    }
}
