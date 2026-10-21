//! The entity floor (spec §6.4, R-23).
//!
//! # What the floor is
//!
//! **A canonical entity is created only when its tag clears a work-count
//! floor.** Below the floor the assertion is still stored, still searchable and
//! still countable, but **it is not a node in the tree**.
//!
//! # Why it is not optional
//!
//! 97 % of the freeform namespace is singletons: 587,258 of 634,190 names are
//! on exactly one work. Without a floor, "browse the taxonomy" is 634,231 rows
//! of one-off user phrases, and every count on the page is a count of one.
//!
//! The measured values, against the AO3 mirror:
//!
//! | floor | tags that qualify | % of 634,231 |
//! |---:|---:|---:|
//! | 1 work | 634,231 | 100 % |
//! | 2 works | 142,788 | 22 % |
//! | **5 works** | **56,197** | **8 %** |
//! | 10 works | 30,694 | 4 % |
//! | 20 works | 16,975 | 2 % |
//! | 60 works | 6,612 | 1 % |
//!
//! # What is *not* below the floor
//!
//! The **assertion** is still there. A tag on one work is still stored, still
//! searchable, and still countable — the floor decides what appears in the
//! *tree*, not what the instance knows. An entity below the floor is a
//! `assertion_only`: the work says "this is a Slow Burn", and the taxonomy
//! does not claim there is a Slow Burn *node*.
//!
//! That distinction is the card's own: *"the assertion is still stored, still
//! searchable and still countable, but it is not a node in the tree."*

use std::collections::BTreeMap;

use darkhold_domain::taxonomy::canonical_form;

use crate::{Backend, Database};

/// The default floor: **five works** (spec §6.4).
pub const DEFAULT_FLOOR: i64 = 5;

/// The floor for each kind of entity.
///
/// The default is [`DEFAULT_FLOOR`], and a kind may have its own. The floor
/// is **configuration and not a constant**, because the question it answers is
/// preference-shaped -- "how big does a name have to be before this instance
/// will call it a node?" -- and the answer carries no authority over any
/// reader, any ranking, or any account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityFloors {
    /// The floor for kinds with no override.
    pub default: i64,
    /// The floor for a named kind.
    pub by_kind: BTreeMap<String, i64>,
}

impl Default for EntityFloors {
    fn default() -> Self {
        Self {
            default: DEFAULT_FLOOR,
            by_kind: BTreeMap::new(),
        }
    }
}

impl EntityFloors {
    /// The floor for one kind.
    pub fn for_kind(&self, kind: &str) -> i64 {
        self.by_kind.get(kind).copied().unwrap_or(self.default)
    }
}

/// Whether an entity is a node in the tree, or only an assertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeState {
    /// It clears the floor: it is a node in the tree.
    Browsable,
    /// It is below the floor: the assertion is stored, and the tree does not
    /// claim it.
    AssertionOnly,
}

impl NodeState {
    /// The storage form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Browsable => "browsable",
            Self::AssertionOnly => "assertion_only",
        }
    }

    /// The storage form, from a column value.
    ///
    /// An unrecognised value is **browsable**: a state the instance does not
    /// know is not a reason to hide a name from the browse, and the alternative
    /// — guessing at a new state — is how a taxonomy disappears.
    pub fn parse(raw: &str) -> Self {
        match raw {
            "assertion_only" => Self::AssertionOnly,
            _ => Self::Browsable,
        }
    }
}

/// What a recount did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecountReport {
    /// How many entities were counted.
    pub counted: i64,
    /// How many are now nodes in the tree.
    pub browsable: i64,
    /// How many are assertions only.
    pub assertion_only: i64,
    /// How many changed state.
    pub changed: i64,
}

/// Recompute every entity's work count and its state (spec §6.4).
///
/// The recount is **idempotent**: it counts the works again and sets the state
/// again, and running it twice changes nothing the second time. That is what
/// makes it a bin's job rather than a one-off script.
pub async fn recount(db: &Database, floors: &EntityFloors) -> Result<RecountReport, sqlx::Error> {
    // The count is a LEFT JOIN so that an entity with no works is counted at
    // zero rather than being left out of the recount — an entity nobody has
    // tagged is exactly the one the floor is about.
    let count_sql = db.sql(
        // The alias is the **column's** name, and the count is cast on the
        // PostgreSQL arm: `COUNT` is an INT8 there and an INT4 in SQLite, and
        // the estate's dialect gate reads a column by the name it is given.
        "SELECT e.id, e.kind, CAST(COUNT(wt.work_id) AS INTEGER) AS node_count
           FROM canonical_entities e
           LEFT JOIN work_tags wt ON wt.node_id = e.id
          GROUP BY e.id, e.kind",
        "SELECT e.id::text, e.kind, CAST(COUNT(wt.work_id) AS BIGINT) AS node_count
           FROM canonical_entities e
           LEFT JOIN work_tags wt ON wt.node_id = e.id
          GROUP BY e.id, e.kind",
    );
    let counts: Vec<(String, String, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&count_sql)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&count_sql)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    let set_sql = db.sql(
        "UPDATE canonical_entities SET node_count = ?, node_state = ? WHERE id = ?",
        "UPDATE canonical_entities SET node_count = $1, node_state = $2 WHERE id::text = $3",
    );

    let mut report = RecountReport::default();
    for (id, kind, work_count) in counts {
        let state = if work_count >= floors.for_kind(&kind) {
            NodeState::Browsable
        } else {
            NodeState::AssertionOnly
        };
        report.counted += 1;
        match state {
            NodeState::Browsable => report.browsable += 1,
            NodeState::AssertionOnly => report.assertion_only += 1,
        }
        let affected = match db.backend() {
            Backend::Sqlite => sqlx::query(&set_sql)
                .bind(work_count)
                .bind(state.as_str())
                .bind(&id)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?
                .rows_affected(),
            Backend::Postgres => sqlx::query(&set_sql)
                .bind(work_count)
                .bind(state.as_str())
                .bind(&id)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?
                .rows_affected(),
        };
        if affected > 0 {
            report.changed += 1;
        }
    }
    Ok(report)
}

/// The state an entity is in, and its work count.
pub async fn state_of(
    db: &Database,
    entity_id: &str,
) -> Result<Option<(NodeState, i64)>, sqlx::Error> {
    let sql = db.sql(
        "SELECT node_state, node_count FROM canonical_entities WHERE id = ?",
        "SELECT node_state, node_count::bigint FROM canonical_entities WHERE id::text = $1",
    );
    let row: Option<(String, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(entity_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(entity_id)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(|(state, count)| (NodeState::parse(&state), count)))
}

/// Mark an entity as a node or as an assertion only.
///
/// **This is what an entity created by a signal gets.** A name nobody has
/// curated and nobody has attached to five works is an assertion, not a node
/// — and the floor is what decides that, so the state is set from the
/// count rather than by hand.
pub async fn set_state(
    db: &Database,
    entity_id: &str,
    work_count: i64,
    floors: &EntityFloors,
) -> Result<NodeState, sqlx::Error> {
    let kind: Option<String> = {
        let sql = db.sql(
            "SELECT kind FROM canonical_entities WHERE id = ?",
            "SELECT kind FROM canonical_entities WHERE id::text = $1",
        );
        match db.backend() {
            Backend::Sqlite => {
                sqlx::query_scalar::<_, String>(&sql)
                    .bind(entity_id)
                    .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                    .await?
            }
            Backend::Postgres => {
                sqlx::query_scalar::<_, String>(&sql)
                    .bind(entity_id)
                    .fetch_optional(db.postgres_pool().expect("postgres handle"))
                    .await?
            }
        }
    };
    let Some(kind) = kind else {
        return Ok(NodeState::Browsable);
    };
    let state = if work_count >= floors.for_kind(&kind) {
        NodeState::Browsable
    } else {
        NodeState::AssertionOnly
    };
    let sql = db.sql(
        "UPDATE canonical_entities SET node_count = ?, node_state = ? WHERE id = ?",
        "UPDATE canonical_entities SET node_count = $1, node_state = $2 WHERE id::text = $3",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(work_count)
                .bind(state.as_str())
                .bind(entity_id)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(work_count)
                .bind(state.as_str())
                .bind(entity_id)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(state)
}

/// The browsable entities of a kind, and how many there are in all.
///
/// The **total** is the count of the entities *above the floor*, not the count
/// of the rows in the table: a reader who sees "12 of 900" learns something,
/// and a reader who sees "900" learns nothing.
pub async fn browsable(
    db: &Database,
    kind: &str,
    limit: i64,
    offset: i64,
) -> Result<(Vec<(String, String, String, i64)>, i64), sqlx::Error> {
    let total_sql = db.sql(
        "SELECT COUNT(*) FROM canonical_entities WHERE kind = ? AND node_state = 'browsable'",
        "SELECT COUNT(*)::bigint FROM canonical_entities WHERE kind = $1 AND node_state = 'browsable'",
    );
    let total: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar::<_, i64>(&total_sql)
                .bind(kind)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar::<_, i64>(&total_sql)
                .bind(kind)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    let page_sql = db.sql(
        "SELECT id, canonical, kind, node_count
           FROM canonical_entities
          WHERE kind = ? AND node_state = 'browsable'
          ORDER BY canonical ASC
          LIMIT ? OFFSET ?",
        "SELECT id::text, canonical, kind, node_count::bigint
           FROM canonical_entities
          WHERE kind = $1 AND node_state = 'browsable'
          ORDER BY canonical ASC
          LIMIT $2 OFFSET $3",
    );
    let rows: Vec<(String, String, String, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&page_sql)
                .bind(kind)
                .bind(limit)
                .bind(offset)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&page_sql)
                .bind(kind)
                .bind(limit)
                .bind(offset)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok((rows, total))
}

/// The normalised form of a name, for the floor's callers.
pub fn norm(name: &str) -> String {
    canonical_form(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_floor_is_five() {
        assert_eq!(DEFAULT_FLOOR, 5);
        assert_eq!(EntityFloors::default().for_kind("tag"), 5);
    }

    #[test]
    fn a_kind_may_have_its_own_floor() {
        let floors = EntityFloors {
            default: 5,
            by_kind: BTreeMap::from([("warning".to_owned(), 1)]),
        };
        assert_eq!(floors.for_kind("warning"), 1);
        assert_eq!(floors.for_kind("tag"), 5);
    }

    #[test]
    fn an_unrecognised_state_is_browsable() {
        // A state the instance does not know is not a reason to hide a name
        // from the browse.
        assert_eq!(NodeState::parse("browsable"), NodeState::Browsable);
        assert_eq!(NodeState::parse("assertion_only"), NodeState::AssertionOnly);
        assert_eq!(NodeState::parse("something-else"), NodeState::Browsable);
    }
}
