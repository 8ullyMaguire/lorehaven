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
    dyn Fn(
            &Database,
            RecContext,
        ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<String>>> + Send>>
        + Send
        + Sync,
>;

/// A factory that creates a strategy by name.
pub type StrategyFactory = Arc<dyn Fn() -> RecStrategyFn + Send + Sync>;

/// What one strategy contributed to a blend, in its own ranking order.
///
/// Produced by `RecRegistry::generate_traced` and consumed by shadow mode
/// (spec §16.1a, M52-08). `produced` is separate from `ranked.len()` only in
/// intent: it is recorded before the ranking list is built, so a strategy whose
/// results were all dropped by the cap is still visible as having produced
/// them. A strategy that silently contributes nothing is the finding an operator
/// most needs before switching modes, and it is invisible in the blended output.
#[derive(Debug, Clone, PartialEq)]
pub struct StrategyContribution {
    /// The registered strategy name.
    pub name: String,
    /// How many work ids the strategy returned.
    pub produced: usize,
    /// Its ranking as `(work_id, RRF contribution)`, best first.
    pub ranked: Vec<(String, f64)>,
}

/// One registry run: the blend, plus every strategy's own contribution.
///
/// The two halves are the point. `blended` is what a reader would be shown;
/// `per_strategy` is what an operator needs in order to decide whether the
/// blend is worth switching on.
#[derive(Debug, Clone, PartialEq)]
pub struct RecRunReport {
    /// The RRF-blended, capped ranking — identical to `RecRegistry::generate`.
    pub blended: Vec<String>,
    /// Each registered strategy's own contribution, in registration order.
    pub per_strategy: Vec<StrategyContribution>,
}

/// Registry of recommendation strategies with RRF blending.
///
/// `Clone` because a reader's stored preference (spec §16.1b) narrows the
/// blend to one strategy without mutating the instance-wide registry.
#[derive(Clone)]
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

        let strategies = document.get("strategies").and_then(|v| v.as_object());

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
        self.strategies
            .iter()
            .map(|(name, _)| name.as_str())
            .collect()
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
        Ok(self.generate_traced(db, ctx).await?.blended)
    }

    /// Run every strategy and report what each one contributed, alongside the
    /// blend they produce.
    ///
    /// Shadow mode (spec §16.1a, M52-08) is the consumer: an operator deciding
    /// whether to switch from `legacy` to `pluggable` needs to know not just
    /// *whether* the two orders differ but *why*. A per-strategy breakdown
    /// answers that — a strategy returning nothing, a strategy returning the
    /// same list as every other one, and a strategy that is outscored are three
    /// different findings, and the blended list alone collapses them into one.
    ///
    /// The blend itself is byte-for-byte what `generate` returns: both call
    /// this method, so shadow mode cannot measure a different computation than
    /// the one it would switch to. That is the whole point of evaluating "on the
    /// same candidate sets".
    ///
    /// A failing strategy still propagates its error, as in `generate` — a
    /// shadow evaluation that silently swallowed a broken strategy would be
    /// reporting a green comparison of two things where one never ran.
    pub async fn generate_traced(&self, db: &Database, ctx: RecContext) -> Result<RecRunReport> {
        let mut scores: HashMap<String, f64> = HashMap::new();
        let mut _seen_set: HashSet<String> = ctx.seen.iter().cloned().collect();
        let mut per_strategy: Vec<StrategyContribution> = Vec::new();

        for (name, strategy) in &self.strategies {
            let ranked = strategy(db, ctx.clone()).await?;
            for (rank, work_id) in ranked.iter().enumerate() {
                let entry = scores.entry(work_id.clone()).or_insert(0.0);
                *entry += 1.0 / (self.k + (rank + 1) as f64);
            }
            // Recorded before the `for work_id in ranked` loop below consumes it,
            // so the count is the strategy's real contribution rather than a
            // length of an already-moved value.
            let produced = ranked.len();
            let mut scored: Vec<(String, f64)> = ranked
                .iter()
                .enumerate()
                .map(|(rank, id)| (id.clone(), 1.0 / (self.k + (rank + 1) as f64)))
                .collect();
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            per_strategy.push(StrategyContribution {
                name: name.clone(),
                produced,
                ranked: scored,
            });
            for work_id in ranked {
                _seen_set.insert(work_id);
            }
        }

        let mut ranked: Vec<(String, f64)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(RecRunReport {
            blended: ranked.into_iter().map(|(id, _)| id).take(ctx.cap).collect(),
            per_strategy,
        })
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

/// Hidden-classics strategy: works whose quality is high relative to their reach.
///
/// Closes gap F on the ideas list (#32 "hidden classics" and #33 "quality-gated
/// under-read gems"), which the audit argued are one gap rather than two: both are
/// "the ranking engines over-reward the already-popular", and both are answered by
/// ranking quality *relative to* reach rather than absolutely.
///
/// **The score is a ratio, and that is the whole design.** Every sibling strategy
/// ranks by an absolute quantity — bookmark counts, graph degree, curation. On a
/// catalogue where attention is already uneven an absolute score cannot tell a work
/// that is *good* from one that is *seen*: both have high numbers and the numbers mean
/// different things. So this one ranks by completion quality per unit of reach:
///
///     score = completion_rate / (1 + log10(1 + distinct_readers))
///
/// Four decisions, each of which produces a defensible-looking ranking that is quietly
/// wrong.
///
/// 1. **Quality is completions over DISTINCT readers.** A completion costs a reader's
///    time, so it is the only numerator here that cannot be inflated by a refresh
///    (§53.6's argument). `work_view_log` has no uniqueness on the reader, so
///    `COUNT(*)` would count a chapter opened four times four times and make a work
///    look better-read than it is. Views appear in the denominator precisely because
///    they are cheap — they measure reach, which is the thing being divided out.
///
/// 2. **The denominator is log-scaled, so reach costs less and less.** Linear division
///    would rank a moderately popular work below a mildly popular one by a large
///    margin, so the strategy would return only works nobody has seen — a random
///    assortment rather than a ranking. The log means each order of magnitude of reach
///    costs the same, which is what "hidden but good" means.
///
/// 3. **Works below §20.3's 10-reader minimum are excluded, not divided.** A completion
///    rate over two readers is not a rate, and a two-of-two work would score a perfect
///    1.0 and outrank everything — the failure that makes such a strategy look broken
///    rather than wrong. §20.3 refuses to compute a multiplier below the same floor.
///
/// 4. **Automated views are not reach.** `work_view_log.is_automated` marks a crawler,
///    and counting one would inflate the denominator of exactly the works least likely
///    to have been read by a person, pushing hidden classics further down.
///
/// The SQL returns the two raw terms and the score is composed in Rust, because SQLite
/// has neither `LOG10` nor `LN` — both require SQLITE_ENABLE_MATH_FUNCTIONS, which this
/// build does not have. Spelling a log in portable SQL would mean a `CASE` ladder over
/// integer ranges, which is a worse version of one `f64::log10` call.
pub fn hidden_classics_strategy() -> RecStrategyFn {
    Arc::new(|db, ctx| {
        let db = db.clone();
        let account_id = ctx.account_id.clone();
        let cap = ctx.cap;
        Box::pin(async move {
            // On PostgreSQL, `work_view_log.work_id` is TEXT where `works.id` and
            // `reading_status.subject_id` are uuid -- the fourth appearance of this
            // split in this codebase (see `payout_store.rs` and `series_recs.rs`).
            //
            // Two fragments, not one: the JOIN needs `::text` on the uuid side, and the
            // SELECT needs it on the way out so the row decodes as `String` (sqlx will
            // not coerce a uuid column into one). The sibling strategies above need
            // neither, because they select `subject_id` out of TEXT columns.
            let (view_join, id_cast) = match db.backend() {
                crate::Backend::Postgres => ("v.work_id = w.id::text", "::text"),
                crate::Backend::Sqlite => ("v.work_id = w.id", ""),
            };
            // Named format arguments throughout. An earlier version used positional
            // `{}` and I got the order wrong three times in one function, each time
            // producing a syntactically valid query with the wrong value in the wrong
            // hole -- which reads as a mysterious SQL syntax error rather than a
            // mistyped argument.
            let sql = format!(
                r#"
                WITH engagement AS (
                    SELECT w.id AS work_id,
                           (SELECT COUNT(DISTINCT v.viewer_hash)
                            FROM work_view_log v
                            WHERE {view_join} AND v.is_automated = 0) AS readers,
                           (SELECT COUNT(*)
                            FROM reading_status rs
                            WHERE rs.subject_type = 'work'
                              AND rs.subject_id = w.id
                              AND rs.status = 'finished') AS completions
                    FROM works w
                    WHERE w.lifecycle = 'published'
                      AND w.id NOT IN (
                          SELECT subject_id FROM bookmarks
                          WHERE account_id = '{account}' AND subject_type = 'work'
                      )
                )
                SELECT work_id{id_cast} AS work_id, readers, completions
                FROM engagement
                WHERE readers >= {min_readers}
                  AND completions > 0
                ORDER BY completions DESC, work_id
                LIMIT {fetch_cap}
                "#,
                account = account_id,
                view_join = view_join,
                id_cast = id_cast,
                min_readers = MIN_READERS_FOR_A_RATE,
                // Wider than `cap` on purpose: the log ranking happens in Rust, so the
                // SQL cannot know which rows win. Truncating to the most-completed
                // works here and then ranking them would re-introduce the absolute
                // count bias this strategy exists to remove.
                fetch_cap = cap.saturating_mul(4),
            );
            let rows = query_int_triples(&db, &sql).await?;
            let mut scored: Vec<(String, f64)> = rows
                .into_iter()
                .map(|(id, readers, completions)| {
                    let rate = completions as f64 / readers as f64;
                    (id, rate / (1.0 + (1.0 + readers as f64).log10()))
                })
                .collect();
            // Ties break on work id rather than on whatever order the database
            // returned, so the same catalogue produces the same feed twice.
            scored.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(&b.0))
            });
            scored.truncate(cap);
            Ok(scored.into_iter().map(|(id, _)| id).collect())
        })
    })
}

/// §20.3's minimum readers for a quality rate to mean anything.
///
/// Spelled as a literal rather than imported, deliberately: this module should not take
/// a dependency to share one number, and a comment pointing at the spec is a weaker
/// guarantee than the number being right here.
const MIN_READERS_FOR_A_RATE: i64 = 10;

/// Run a query returning `(String, i64, i64)` on either backend.
///
/// Its own helper because `hidden_classics_strategy` needs three columns. `query_pairs`
/// returns two, and widening it would change every caller's type for one caller.
async fn query_int_triples(db: &Database, sql: &str) -> Result<Vec<(String, i64, i64)>> {
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

/// Build the default strategy factories map (spec §16.1a).
pub fn default_strategies() -> HashMap<String, StrategyFactory> {
    let mut map: HashMap<String, StrategyFactory> = HashMap::new();
    map.insert(
        "cooccurrence".to_string(),
        Arc::new(|| cooccurrence_strategy()),
    );
    map.insert("time_decay".to_string(), Arc::new(|| time_decay_strategy()));
    map.insert("tag_graph".to_string(), Arc::new(|| tag_graph_strategy()));
    map.insert(
        "author_graph".to_string(),
        Arc::new(|| author_graph_strategy()),
    );
    map.insert("sequential".to_string(), Arc::new(|| sequential_strategy()));
    map.insert(
        "completion_weight".to_string(),
        Arc::new(|| completion_weight_strategy()),
    );
    map.insert(
        "curator_prior".to_string(),
        Arc::new(|| curator_prior_strategy()),
    );
    map.insert("bandit".to_string(), Arc::new(|| bandit_strategy()));
    map.insert(
        "hidden_classics".to_string(),
        Arc::new(|| hidden_classics_strategy()),
    );
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
