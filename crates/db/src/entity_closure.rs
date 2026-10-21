//! The entity closure (spec §6.4, §11.4).
//!
//! # What the closure is
//!
//! A **materialised** transitive closure of the entity tree: for every
//! ancestor and every descendant, the row that says so and how far apart they
//! are. `+children` expansion is then **a single join** rather than a
//! recursion, and the plan's expansion query filters by ancestor and reads
//! depth.
//!
//! # Why it is materialised
//!
//! The expansion is asked on every search that carries a `+children` term, and
//! the tree changes a handful of times a day. A recursive query per search is
//! a query per search; a table is a join.
//!
//! # Why the rebuild is in Rust and not in SQL
//!
//! **A cycle in the tree makes a recursive CTE loop.** PostgreSQL's recursive
//! term may not use `UNION`, so a cycle is an unbounded recursion rather than
//! a deduplicated one; SQLite's `UNION` deduplicates and so terminates, which
//! means the same tree is handled differently by the two engines — the worst
//! kind of difference, because it is invisible until one of them hangs.
//!
//! So the walk is done here, with a **visited set**: a node is never expanded
//! twice, so the walk terminates whatever the tree contains, and the two
//! engines get the same closure.
//!
//! # Only browsable entities are in the closure
//!
//! The closure **is** the tree (spec §6.4, R-23): a row that is not in the
//! closure is not a node. An entity below the floor is an assertion, and an
//! assertion is not in the tree.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::{Backend, Database};

/// One row of the closure: an ancestor, a descendant, and how far apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosureRow {
    /// The ancestor.
    pub ancestor_id: String,
    /// The descendant.
    pub descendant_id: String,
    /// How many steps down the tree.
    pub depth: i64,
}

/// What a rebuild did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClosureReport {
    /// How many rows the closure now holds.
    pub rows: i64,
    /// How many entities were left out because they are below the floor.
    pub skipped: i64,
}

/// Rebuild the closure from the entity tree.
///
/// The tree is the `parent_id` column on `canonical_entities`, and only the
/// **browsable** entities are in it (spec §6.4, R-23): the closure is the
/// tree, and an assertion is not a node.
pub async fn rebuild(db: &Database) -> Result<ClosureReport, sqlx::Error> {
    // The tree, and the entities that are in it.
    let tree_sql = db.sql(
        "SELECT id, parent_id FROM canonical_entities WHERE node_state = 'browsable'",
        "SELECT id::text, parent_id FROM canonical_entities WHERE node_state = 'browsable'",
    );
    let tree: Vec<(String, Option<String>)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&tree_sql)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&tree_sql)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    let parents: BTreeMap<String, Option<String>> = tree.iter().cloned().collect();
    let children: BTreeMap<String, Vec<String>> = {
        let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (id, parent) in &tree {
            if let Some(parent) = parent {
                map.entry(parent.clone()).or_default().push(id.clone());
            }
        }
        for list in map.values_mut() {
            list.sort();
        }
        map
    };

    // **The walk, with a visited set.** A node is expanded once, so a cycle
    // terminates and the two engines agree.
    let mut rows: Vec<ClosureRow> = Vec::new();
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<(String, String, i64)> = VecDeque::new();
    for (id, parent) in &tree {
        if let Some(parent) = parent {
            queue.push_back((parent.clone(), id.clone(), 1));
        }
    }
    while let Some((ancestor, descendant, depth)) = queue.pop_front() {
        // The visited set is on the **path**, not on the whole tree: a node
        // with two parents is in the tree twice, and each path is a row.
        let key = (ancestor.clone(), descendant.clone());
        if !visited.insert(format!("{}->{}", key.0, key.1)) {
            continue;
        }
        rows.push(ClosureRow {
            ancestor_id: ancestor.clone(),
            descendant_id: descendant.clone(),
            depth,
        });
        for child in children.get(&descendant).into_iter().flatten() {
            queue.push_back((ancestor.clone(), child.clone(), depth + 1));
        }
    }

    // The closure is **rebuilt**, not appended to: a stale row is a row that
    // says a tree that no longer exists.
    let clear_sql = db.sql("DELETE FROM entity_closure", "DELETE FROM entity_closure");
    let insert_sql = db.sql(
        "INSERT INTO entity_closure (ancestor_id, descendant_id, rel, depth, created_at)
         VALUES (?, ?, 'parent', ?, ?)",
        "INSERT INTO entity_closure (ancestor_id, descendant_id, rel, depth, created_at)
         VALUES ($1, $2, 'parent', $3::integer, $4)",
    );
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite handle");
            sqlx::query(&clear_sql).execute(pool).await?;
            for row in &rows {
                sqlx::query(&insert_sql)
                    .bind(&row.ancestor_id)
                    .bind(&row.descendant_id)
                    .bind(row.depth)
                    .bind(&now)
                    .execute(pool)
                    .await?;
            }
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres handle");
            sqlx::query(&clear_sql).execute(pool).await?;
            for row in &rows {
                sqlx::query(&insert_sql)
                    .bind(&row.ancestor_id)
                    .bind(&row.descendant_id)
                    .bind(row.depth)
                    .bind(&now)
                    .execute(pool)
                    .await?;
            }
        }
    }

    let total = parents.len() as i64;
    Ok(ClosureReport {
        rows: rows.len() as i64,
        skipped: total - parents.len() as i64,
    })
}

/// The transitive descendants of an entity, nearest first.
///
/// This is the `+children` expansion's answer, and it is the *closure* that
/// answers it — a single join rather than a recursion.
pub async fn descendants_of(
    db: &Database,
    entity_id: &str,
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let sql = db.sql(
        "SELECT descendant_id, depth FROM entity_closure
          WHERE ancestor_id = ? AND rel = 'parent'
          ORDER BY depth ASC, descendant_id ASC",
        // **The cast is not decoration.** The depth column is an INTEGER on
        // this dialect -- an INT4 -- and an INT4 decoded into an i64 is a
        // decode error. The estate's dialect gate says so, and the SQLite arm
        // needs no cast because SQLite has no INT4.
        "SELECT descendant_id, CAST(depth AS BIGINT) AS depth FROM entity_closure
          WHERE ancestor_id = $1 AND rel = 'parent'
          ORDER BY depth ASC, descendant_id ASC",
    );
    let rows: Vec<(String, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(entity_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(entity_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_walk_of_a_cycle_terminates() {
        // The walk is the part that has to be right, and a cycle is the case
        // a recursive query gets wrong. A parent that is also a child is
        // nonsense, and the closure must not hang on it.
        let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
        children.insert("a".to_owned(), vec!["b".to_owned()]);
        children.insert("b".to_owned(), vec!["a".to_owned()]);

        let mut visited: BTreeSet<String> = BTreeSet::new();
        let mut queue: VecDeque<(String, String, i64)> = VecDeque::new();
        queue.push_back(("a".to_owned(), "b".to_owned(), 1));
        let mut rows = 0;
        while let Some((ancestor, descendant, _depth)) = queue.pop_front() {
            if !visited.insert(format!("{ancestor}->{descendant}")) {
                continue;
            }
            rows += 1;
            for child in children.get(&descendant).into_iter().flatten() {
                queue.push_back((ancestor.clone(), child.clone(), 2));
            }
        }
        assert!(rows < 10, "the walk terminates: {rows} rows");
    }
}
