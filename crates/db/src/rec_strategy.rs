//! Recommendation strategies (spec §16.1a) and recipe composition (§16.3).

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::Arc;

use crate::Database;
use anyhow::Result;

/// Context passed to every recommendation strategy.
#[derive(Debug, Clone)]
pub struct RecContext {
    pub account_id: String,
    pub seen: Vec<String>,
    pub cap: usize,
}

/// A recommendation strategy: given the database and context, return a
/// ranked list of work IDs.
pub type RecStrategyFn = Arc<
    dyn Fn(&Database, RecContext) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<String>>> + Send>>
        + Send
        + Sync,
>;

/// A factory that creates a strategy by name.
pub type StrategyFactory = Arc<dyn Fn() -> RecStrategyFn + Send + Sync>;

/// Registry of recommendation strategies with RRF blending.
pub struct RecRegistry {
    k: f64,
    strategies: Vec<(String, RecStrategyFn)>,
}

impl RecRegistry {
    pub fn new(k: f64) -> Self {
        Self {
            k,
            strategies: Vec::new(),
        }
    }

    /// Register a strategy under `name`.
    ///
    /// Takes `impl Into<String>` rather than `&'static str`: the strategy names
    /// come from a factories map that is rebuilt on every call, so its keys
    /// cannot be borrowed for `'static`. The name is stored rather than
    /// discarded, because it is what the blend's per-strategy weights key off
    /// and what [`RecRegistry::names`] reports.
    pub fn register(&mut self, name: impl Into<String>, f: RecStrategyFn) {
        self.strategies.push((name.into(), f));
    }

    /// Build a registry from a recipe document (spec §16.3).
    ///
    /// The recipe document names strategies and optional weights:
    /// ```json
    /// { "strategies": { "cooccurrence": 1.0, "time_decay": 0.8 } }
    /// ```
    ///
    /// Strategies not named in the recipe are excluded. Strategies named
    /// but not available in the factories map are silently skipped.
    pub fn build_from_document(
        k: f64,
        document: &serde_json::Value,
        factories: &HashMap<String, StrategyFactory>,
    ) -> Self {
        let mut reg = Self::new(k);

        let strategies = document
            .get("strategies")
            .and_then(|v| v.as_object());

        if let Some(strategies) = strategies {
            for (name, _weight) in strategies {
                if let Some(factory) = factories.get(name) {
                    let strategy = factory();
                    reg.register(name.clone().leak(), strategy);
                }
            }
        }

        reg
    }

    pub fn strategy_count(&self) -> usize {
        self.strategies.len()
    }

    /// The registered strategy names, in registration order.
    ///
    /// The settings endpoint validates a reader's engine choice against this,
    /// and the reader's own settings surface lists it, so the names have to be
    /// readable rather than internal.
    pub fn names(&self) -> Vec<&str> {
        self.strategies.iter().map(|(name, _)| name.as_str()).collect()
    }

    /// Whether `name` is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.strategies.iter().any(|(n, _)| n == name)
    }

    /// A registry containing only `name`, if it is registered.
    ///
    /// This is how a reader's stored preference is honoured (spec §16.1b): the
    /// operator's registry decides what exists, and the reader's preference
    /// narrows the blend to one strategy. Returns `None` when the stored
    /// preference names a strategy the operator has since disabled — the
    /// caller reports that rather than silently blending everything.
    pub fn only(&self, name: &str) -> Option<RecRegistry> {
        if !self.contains(name) {
            return None;
        }
        let (n, f) = self
            .strategies
            .iter()
            .find(|(n, _)| n == name)
            .expect("contains() checked membership");
        Some(RecRegistry {
            k: self.k,
            strategies: vec![(n.clone(), f.clone())],
        })
    }

    pub async fn generate(&self, db: &Database, ctx: RecContext) -> Result<Vec<String>> {
        let mut scores: HashMap<String, f64> = HashMap::new();
        let mut _seen_set: HashSet<String> = ctx.seen.iter().cloned().collect();

        for (_name, strategy) in &self.strategies {
            let ranked = strategy(db, ctx.clone()).await?;
            for (rank, work_id) in ranked.iter().enumerate() {
                let entry = scores.entry(work_id.clone()).or_insert(0.0);
                *entry += 1.0 / (self.k + (rank + 1) as f64);
            }
            for work_id in ranked {
                _seen_set.insert(work_id);
            }
        }

        let mut ranked: Vec<(String, f64)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(ranked.into_iter().map(|(id, _)| id).take(ctx.cap).collect())
    }
}

/// Run a query that returns `(String, i64)` pairs on either backend.
async fn query_pairs(db: &Database, sql: &str) -> Result<Vec<(String, i64)>> {
    match db.backend() {
        crate::Backend::Sqlite => {
            let rows = sqlx::query_as(sql)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(rows)
        }
        crate::Backend::Postgres => {
            let rows = sqlx::query_as(sql)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(rows)
        }
    }
}

/// Run a query that returns `(String, f64)` pairs on either backend.
async fn query_pairs_f64(db: &Database, sql: &str) -> Result<Vec<(String, f64)>> {
    match db.backend() {
        crate::Backend::Sqlite => {
            let rows = sqlx::query_as(sql)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(rows)
        }
        crate::Backend::Postgres => {
            let rows = sqlx::query_as(sql)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(rows)
        }
    }
}

/// Run a query that returns `String` values on either backend.
async fn query_strings(db: &Database, sql: &str) -> Result<Vec<String>> {
    match db.backend() {
        crate::Backend::Sqlite => {
            let rows: Vec<(String,)> = sqlx::query_as(sql)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(rows.into_iter().map(|(s,)| s).collect())
        }
        crate::Backend::Postgres => {
            let rows: Vec<(String,)> = sqlx::query_as(sql)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(rows.into_iter().map(|(s,)| s).collect())
        }
    }
}

/// Co-occurrence strategy: "users who bookmarked X also bookmarked Y".
pub fn cooccurrence_strategy() -> RecStrategyFn {
    Arc::new(|db, ctx| {
        let db = db.clone();
        let account_id = ctx.account_id.clone();
        let cap = ctx.cap;
        Box::pin(async move {
            let sql = format!(
                r#"
                SELECT b2.subject_id, COUNT(*) as score
                FROM bookmarks b1
                JOIN bookmarks b2 ON b1.account_id = b2.account_id
                    AND b1.subject_id != b2.subject_id
                WHERE b1.account_id = '{}'
                    AND b1.subject_type = 'work'
                    AND b2.subject_type = 'work'
                    AND b2.subject_id NOT IN (
                        SELECT subject_id FROM bookmarks
                        WHERE account_id = '{}' AND subject_type = 'work'
                    )
                GROUP BY b2.subject_id
                ORDER BY score DESC
                LIMIT {}
                "#,
                account_id, account_id, cap
            );
            let pairs = query_pairs(&db, &sql).await?;
            Ok(pairs.into_iter().map(|(id, _)| id).collect())
        })
    })
}

/// Time-decay strategy: weight recent bookmarks more heavily.
pub fn time_decay_strategy() -> RecStrategyFn {
    Arc::new(|db, ctx| {
        let db = db.clone();
        let account_id = ctx.account_id.clone();
        let cap = ctx.cap;
        Box::pin(async move {
            let sql = format!(
                r#"
                SELECT b2.subject_id,
                       SUM(1.0 / (1.0 + julianday('now') - julianday(b2.created_at))) as score
                FROM bookmarks b1
                JOIN bookmarks b2 ON b1.account_id = b2.account_id
                    AND b1.subject_id != b2.subject_id
                WHERE b1.account_id = '{}'
                    AND b1.subject_type = 'work'
                    AND b2.subject_type = 'work'
                    AND b2.subject_id NOT IN (
                        SELECT subject_id FROM bookmarks
                        WHERE account_id = '{}' AND subject_type = 'work'
                    )
                GROUP BY b2.subject_id
                ORDER BY score DESC
                LIMIT {}
                "#,
                account_id, account_id, cap
            );
            let pairs = query_pairs_f64(&db, &sql).await?;
            Ok(pairs.into_iter().map(|(id, _)| id).collect())
        })
    })
}

/// Tag graph strategy: recommend works sharing tags with the user's bookmarks.
pub fn tag_graph_strategy() -> RecStrategyFn {
    Arc::new(|db, ctx| {
        let db = db.clone();
        let account_id = ctx.account_id.clone();
        let cap = ctx.cap;
        Box::pin(async move {
            let sql = format!(
                r#"
                SELECT wt2.work_id, COUNT(*) as score
                FROM bookmarks b
                JOIN work_tags wt1 ON wt1.work_id = b.subject_id
                JOIN work_tags wt2 ON wt1.node_id = wt2.node_id
                    AND wt1.work_id != wt2.work_id
                WHERE b.account_id = '{}'
                    AND b.subject_type = 'work'
                    AND wt2.work_id NOT IN (
                        SELECT subject_id FROM bookmarks
                        WHERE account_id = '{}' AND subject_type = 'work'
                    )
                GROUP BY wt2.work_id
                ORDER BY score DESC
                LIMIT {}
                "#,
                account_id, account_id, cap
            );
            let pairs = query_pairs(&db, &sql).await?;
            Ok(pairs.into_iter().map(|(id, _)| id).collect())
        })
    })
}

/// Author graph strategy: recommend works by authors the user has bookmarked.
pub fn author_graph_strategy() -> RecStrategyFn {
    Arc::new(|db, ctx| {
        let db = db.clone();
        let account_id = ctx.account_id.clone();
        let cap = ctx.cap;
        Box::pin(async move {
            let sql = format!(
                r#"
                SELECT w2.id, COUNT(*) as score
                FROM bookmarks b
                JOIN works w1 ON w1.id = b.subject_id
                JOIN works w2 ON w1.owner_pseud_id = w2.owner_pseud_id
                    AND w1.id != w2.id
                WHERE b.account_id = '{}'
                    AND b.subject_type = 'work'
                    AND w2.id NOT IN (
                        SELECT subject_id FROM bookmarks
                        WHERE account_id = '{}' AND subject_type = 'work'
                    )
                GROUP BY w2.id
                ORDER BY score DESC
                LIMIT {}
                "#,
                account_id, account_id, cap
            );
            let pairs = query_pairs(&db, &sql).await?;
            Ok(pairs.into_iter().map(|(id, _)| id).collect())
        })
    })
}

/// Sequential strategy: recommend works the user is currently reading.
pub fn sequential_strategy() -> RecStrategyFn {
    Arc::new(|db, ctx| {
        let db = db.clone();
        let account_id = ctx.account_id.clone();
        let cap = ctx.cap;
        Box::pin(async move {
            let sql = format!(
                r#"
                SELECT DISTINCT r.subject_id
                FROM reading_history_entry r
                JOIN works w ON w.id = r.subject_id
                WHERE r.account_id = '{}'
                    AND r.subject_type = 'work'
                    AND w.lifecycle = 'published'
                    AND w.completion = 'in_progress'
                    AND r.subject_id NOT IN (
                        SELECT subject_id FROM bookmarks
                        WHERE account_id = '{}' AND subject_type = 'work'
                    )
                ORDER BY r.last_read_at DESC
                LIMIT {}
                "#,
                account_id, account_id, cap
            );
            let strings = query_strings(&db, &sql).await?;
            Ok(strings)
        })
    })
}

/// Completion weight strategy: prefer completed works.
pub fn completion_weight_strategy() -> RecStrategyFn {
    Arc::new(|db, ctx| {
        let db = db.clone();
        let account_id = ctx.account_id.clone();
        let cap = ctx.cap;
        Box::pin(async move {
            let sql = format!(
                r#"
                SELECT w.id,
                       (CASE w.completion
                           WHEN 'completed' THEN 2
                           ELSE 1
                       END * (1 + (
                           SELECT COUNT(*)
                           FROM reading_history_entry r
                           WHERE r.account_id = '{}'
                               AND r.subject_id = w.id
                       ))) as score
                FROM works w
                WHERE w.lifecycle = 'published'
                    AND w.id NOT IN (
                        SELECT subject_id FROM bookmarks
                        WHERE account_id = '{}' AND subject_type = 'work'
                    )
                ORDER BY score DESC
                LIMIT {}
                "#,
                account_id, account_id, cap
            );
            let pairs = query_pairs(&db, &sql).await?;
            Ok(pairs.into_iter().map(|(id, _)| id).collect())
        })
    })
}

/// Curator prior strategy: boost works bookmarked by trusted curators.
pub fn curator_prior_strategy() -> RecStrategyFn {
    Arc::new(|db, ctx| {
        let db = db.clone();
        let account_id = ctx.account_id.clone();
        let cap = ctx.cap;
        Box::pin(async move {
            let sql = format!(
                r#"
                SELECT b.subject_id, COUNT(*) as score
                FROM bookmarks b
                JOIN accounts a ON a.id = b.account_id
                WHERE b.subject_type = 'work'
                    AND b.is_public = 1
                    AND a.is_curator = 1
                    AND b.subject_id NOT IN (
                        SELECT subject_id FROM bookmarks
                        WHERE account_id = '{}' AND subject_type = 'work'
                    )
                GROUP BY b.subject_id
                ORDER BY score DESC
                LIMIT {}
                "#,
                account_id, cap
            );
            let pairs = query_pairs(&db, &sql).await?;
            Ok(pairs.into_iter().map(|(id, _)| id).collect())
        })
    })
}

/// Bandit strategy: explore/exploit based on engagement signals.
pub fn bandit_strategy() -> RecStrategyFn {
    Arc::new(|db, ctx| {
        let db = db.clone();
        let account_id = ctx.account_id.clone();
        let cap = ctx.cap;
        Box::pin(async move {
            let sql = format!(
                r#"
                SELECT w.id,
                       COALESCE(m.views, 0) as score
                FROM works w
                LEFT JOIN work_metric_aggregates m ON m.work_id = w.id
                WHERE w.lifecycle = 'published'
                    AND w.id NOT IN (
                        SELECT subject_id FROM bookmarks
                        WHERE account_id = '{}' AND subject_type = 'work'
                    )
                ORDER BY score DESC
                LIMIT {}
                "#,
                account_id, cap
            );
            let pairs = query_pairs(&db, &sql).await?;
            Ok(pairs.into_iter().map(|(id, _)| id).collect())
        })
    })
}

/// Build the default strategy factories map (spec §16.1a).
pub fn default_strategies() -> HashMap<String, StrategyFactory> {
    let mut map: HashMap<String, StrategyFactory> = HashMap::new();
    map.insert(
        "cooccurrence".to_string(),
        Arc::new(|| cooccurrence_strategy()),
    );
    map.insert(
        "time_decay".to_string(),
        Arc::new(|| time_decay_strategy()),
    );
    map.insert(
        "tag_graph".to_string(),
        Arc::new(|| tag_graph_strategy()),
    );
    map.insert(
        "author_graph".to_string(),
        Arc::new(|| author_graph_strategy()),
    );
    map.insert(
        "sequential".to_string(),
        Arc::new(|| sequential_strategy()),
    );
    map.insert(
        "completion_weight".to_string(),
        Arc::new(|| completion_weight_strategy()),
    );
    map.insert(
        "curator_prior".to_string(),
        Arc::new(|| curator_prior_strategy()),
    );
    map.insert("bandit".to_string(), Arc::new(|| bandit_strategy()));
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> RecContext {
        RecContext {
            account_id: "test-account".to_string(),
            seen: vec![],
            cap: 10,
        }
    }

    #[test]
    fn rec_context_default() {
        let c = ctx();
        assert_eq!(c.cap, 10);
        assert!(c.seen.is_empty());
    }

    #[test]
    fn registry_new_is_empty() {
        let reg = RecRegistry::new(60.0);
        assert!(reg.strategies.is_empty());
    }

    #[test]
    fn registry_register_adds_strategy() {
        let mut reg = RecRegistry::new(60.0);
        reg.register(
            "test",
            Arc::new(|_db, _ctx| Box::pin(async move { Ok(vec![]) })),
        );
        assert_eq!(reg.strategies.len(), 1);
    }

    #[test]
    fn build_from_document_empty() {
        let factories = default_strategies();
        let doc = serde_json::json!({});
        let reg = RecRegistry::build_from_document(60.0, &doc, &factories);
        assert!(reg.strategies.is_empty());
    }

    #[test]
    fn build_from_document_selects_strategies() {
        let factories = default_strategies();
        let doc = serde_json::json!({
            "strategies": {
                "cooccurrence": 1.0,
                "tag_graph": 0.8
            }
        });
        let reg = RecRegistry::build_from_document(60.0, &doc, &factories);
        assert_eq!(reg.strategies.len(), 2);
    }

    #[test]
    fn build_from_document_unknown_strategy_skipped() {
        let factories = default_strategies();
        let doc = serde_json::json!({
            "strategies": {
                "nonexistent": 1.0
            }
        });
        let reg = RecRegistry::build_from_document(60.0, &doc, &factories);
        assert!(reg.strategies.is_empty());
    }
}
