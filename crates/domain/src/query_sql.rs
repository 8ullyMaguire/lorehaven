//! Query SQL rendering: compile the AST into SQL fragments.
//!
//! Spec §15.3. Pure functions — no I/O.

use crate::query::{CompareOp, QueryAst, QueryError, QueryField};

/// A rendered SQL fragment with its bind parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct SqlFragment {
    pub sql: String,
    pub binds: Vec<String>,
}

impl SqlFragment {
    pub fn new(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            binds: Vec::new(),
        }
    }

    pub fn with_bind(mut self, bind: impl Into<String>) -> Self {
        self.binds.push(bind.into());
        self
    }
}

/// Render a query AST to a SQL WHERE clause fragment.
pub fn render_query(ast: &QueryAst) -> Result<SqlFragment, QueryError> {
    render_node(ast)
}

fn render_node(ast: &QueryAst) -> Result<SqlFragment, QueryError> {
    match ast {
        QueryAst::Text(text) => {
            let sql =
                "(LOWER(works.title) LIKE LOWER(?) OR LOWER(works.summary) LIKE LOWER(?) OR LOWER(works_index.body_text) LIKE LOWER(?))"
                    .to_owned();
            let pattern = format!("%{}%", text);
            Ok(SqlFragment::new(sql)
                .with_bind(pattern.clone())
                .with_bind(pattern.clone())
                .with_bind(pattern))
        }
        QueryAst::Phrase(phrase) => {
            let sql = "(LOWER(works_index.body_text) LIKE LOWER(?))";
            let pattern = format!("%{}%", phrase);
            Ok(SqlFragment::new(sql).with_bind(pattern))
        }
        QueryAst::Fielded(field, value) => render_fielded(field, value),
        QueryAst::Comparison(field, op, value) => render_comparison(field, *op, value),
        QueryAst::And(parts) => {
            let mut fragments = Vec::new();
            for part in parts {
                fragments.push(render_node(part)?);
            }
            let sql = fragments
                .iter()
                .map(|f| f.sql.as_str())
                .collect::<Vec<_>>()
                .join(" AND ");
            let binds: Vec<String> = fragments.iter().flat_map(|f| f.binds.clone()).collect();
            Ok(SqlFragment {
                sql: format!("({})", sql),
                binds,
            })
        }
        QueryAst::Or(parts) => {
            let mut fragments = Vec::new();
            for part in parts {
                fragments.push(render_node(part)?);
            }
            let sql = fragments
                .iter()
                .map(|f| f.sql.as_str())
                .collect::<Vec<_>>()
                .join(" OR ");
            let binds: Vec<String> = fragments.iter().flat_map(|f| f.binds.clone()).collect();
            Ok(SqlFragment {
                sql: format!("({})", sql),
                binds,
            })
        }
        QueryAst::Not(inner) => {
            let inner = render_node(inner)?;
            Ok(SqlFragment {
                sql: format!("NOT ({})", inner.sql),
                binds: inner.binds,
            })
        }
    }
}

fn quality_threshold(value: &str) -> Result<String, QueryError> {
    let value = value
        .parse::<i64>()
        .map_err(|_| QueryError::new("quality must be an integer from 0 to 1000", 0))?;
    if !(0..=1000).contains(&value) {
        return Err(QueryError::new("quality must be from 0 to 1000", 0));
    }
    Ok(value.to_string())
}

/// Render a comparison: `field<op>value`.
///
/// The works-surface fields are the ones this module can bind a column for;
/// every other entity's fields are rejected with a message naming the field, so
/// a query written for the wrong surface fails loudly instead of matching zero
/// rows for a reason the reader cannot see. The forum, directory, bookmark and
/// user surfaces render their own fields through their own modules — the
/// *operators* are shared, the columns are not.
fn render_comparison(
    field: &QueryField,
    op: CompareOp,
    value: &str,
) -> Result<SqlFragment, QueryError> {
    // Re-validate: the parser checks the value is an integer, but
    // `render_query` is public and can be handed an AST built by hand.
    let n = value.parse::<i64>().map_err(|_| {
        QueryError::new(
            format!(
                "{} {} takes an integer, got {:?}",
                field.as_str(),
                op.as_str(),
                value
            ),
            0,
        )
    })?;

    let column = numeric_column(field)?;
    Ok(
        SqlFragment::new(format!("({column} {} CAST(? AS BIGINT))", op.sql()))
            .with_bind(n.to_string()),
    )
}

/// The SQL expression for a works-surface numeric field.
///
/// `words` and `kudos` are both computed rather than stored on `works`, so both
/// are subqueries. Each coalesces to 0: a work with no chapters sums to NULL
/// and a work never kudosed has no row in the aggregate table, and a
/// `words:>0` / `kudos:>0` that excluded them would be silently wrong.
fn numeric_column(field: &QueryField) -> Result<&'static str, QueryError> {
    Ok(match field {
        // `work_metric_aggregates.kudos` is INTEGER (migration 0068).
        QueryField::Kudos => {
            "COALESCE((SELECT wma.kudos FROM work_metric_aggregates wma WHERE wma.work_id = works.id), 0)"
        }
        // Summed over live chapters' current revisions, matching the `word_count`
        // the search row reports in `search/ast_search.rs`.
        QueryField::Words => {
            "COALESCE((SELECT SUM(CAST(cr.word_count AS BIGINT)) FROM chapters c \
             JOIN chapter_revisions cr ON cr.id = c.current_revision_id \
             WHERE c.work_id = works.id AND c.deleted_at IS NULL), 0)"
        }
        other => {
            return Err(QueryError::new(
                format!(
                    "{} is not a comparable field on the works surface \
                     (comparable here: words, kudos)",
                    other.as_str()
                ),
                0,
            ))
        }
    })
}

fn render_fielded(field: &QueryField, value: &str) -> Result<SqlFragment, QueryError> {
    match field {
        // A field that belongs to another surface. Rejecting it with a message
        // naming the field is the point of a shared language: `category:meta`
        // on the works search is a reader mistake, and a silent zero-row answer
        // would look like "the forum has no meta category".
        QueryField::Replies
        | QueryField::Category
        | QueryField::Kind
        | QueryField::Active
        | QueryField::Pinned
        | QueryField::Locked
        | QueryField::Rank
        | QueryField::Type_
        | QueryField::Submitter
        | QueryField::Rec
        | QueryField::Note
        | QueryField::User
        | QueryField::Works
        | QueryField::UserFandom
        | QueryField::Joined
        | QueryField::Bookmarked => Err(QueryError::new(
            format!(
                "{} is a {} field, not a works field",
                field.as_str(),
                field.entity().as_str()
            ),
            0,
        )),
        // Reachable through a hand-built AST or an explicit `words:5000`. The
        // parser routes a comparison through `Comparison`, but equality on a
        // numeric field is a legitimate query and must not be a server error.
        QueryField::Words | QueryField::Kudos => {
            let column = numeric_column(field)?;
            Ok(SqlFragment::new(format!("({column} = CAST(? AS BIGINT))"))
                .with_bind(value.to_owned()))
        }
        QueryField::MinQuality => {
            let threshold = quality_threshold(value)?;
            // Stored weights are instance-controlled; missing signals do not qualify.
            Ok(SqlFragment::new("((SELECT SUM(CAST(qs.value AS DOUBLE PRECISION) * qs.weight) / NULLIF(CAST(SUM(qs.weight) AS BIGINT), 0) FROM quality_signals qs WHERE qs.work_id = works.id AND qs.weight > 0) >= CAST(? AS DOUBLE PRECISION))").with_bind(threshold))
        }
        QueryField::Quality => {
            let (kind, threshold) = value
                .split_once(':')
                .ok_or_else(|| QueryError::new("quality requires signal:minimum", 0))?;
            kind.parse::<crate::media::QualitySignalKind>()
                .map_err(|_| QueryError::new("unknown quality signal", 0))?;
            Ok(SqlFragment::new("EXISTS (SELECT 1 FROM quality_signals qs WHERE qs.work_id = works.id AND qs.signal_kind = ? AND qs.value >= CAST(? AS BIGINT))")
                .with_bind(kind).with_bind(quality_threshold(threshold)?))
        }
        QueryField::Completion => {
            if !matches!(value, "complete" | "in_progress" | "abandoned" | "on_hold") {
                return Err(QueryError::new("unknown completion status", 0));
            }
            Ok(SqlFragment::new("works.completion = ?").with_bind(value))
        }
        QueryField::Published => {
            let (start, end) = value
                .split_once("..")
                .ok_or_else(|| QueryError::new("published requires YYYY-MM-DD..YYYY-MM-DD", 0))?;
            let parse = |s: &str| {
                time::Date::parse(
                    s,
                    &time::macros::format_description!("[year]-[month]-[day]"),
                )
                .map_err(|_| QueryError::new("invalid date", 0))
            };
            let start = parse(start)?;
            let end = parse(end)?;
            if start > end {
                return Err(QueryError::new("date range is reversed", 0));
            }
            Ok(SqlFragment::new(
                "(SUBSTR(works.published_at, 1, 10) >= ? AND SUBSTR(works.published_at, 1, 10) <= ?)",
            )
            .with_bind(start.to_string())
            .with_bind(end.to_string()))
        }
        QueryField::Rating => {
            if !matches!(value, "general" | "teen" | "mature" | "explicit") {
                return Err(QueryError::new("unknown rating", 0));
            }
            Ok(SqlFragment::new("works.rating = ?").with_bind(value))
        }
        QueryField::Format => {
            value
                .parse::<crate::media::MediaFormat>()
                .map_err(|_| QueryError::new("unknown media format", 0))?;
            Ok(SqlFragment::new("(works.format = ?)").with_bind(value))
        }
        QueryField::Edition => {
            value
                .parse::<crate::media::EditionKind>()
                .map_err(|_| QueryError::new("unknown edition kind", 0))?;
            Ok(SqlFragment::new("EXISTS (SELECT 1 FROM media_editions me WHERE me.work_id = works.id AND me.edition_kind = ?)").with_bind(value))
        }
        QueryField::Updated => {
            let (start, end) = value
                .split_once("..")
                .ok_or_else(|| QueryError::new("updated requires YYYY-MM-DD..YYYY-MM-DD", 0))?;
            let parse = |s: &str| {
                time::Date::parse(
                    s,
                    &time::macros::format_description!("[year]-[month]-[day]"),
                )
                .map_err(|_| QueryError::new("invalid date", 0))
            };
            let start = parse(start)?;
            let end = parse(end)?;
            if start > end {
                return Err(QueryError::new("date range is reversed", 0));
            }
            // Inclusive date bounds; timestamps are canonical TEXT in both dialects.
            Ok(SqlFragment::new(
                "(SUBSTR(works.updated_at, 1, 10) >= ? AND SUBSTR(works.updated_at, 1, 10) <= ?)",
            )
            .with_bind(start.to_string())
            .with_bind(end.to_string()))
        }
        QueryField::Title => {
            let sql = "(LOWER(works.title) LIKE LOWER(?))";
            Ok(SqlFragment::new(sql).with_bind(format!("%{}%", value)))
        }
        QueryField::Author => {
            let sql = "(LOWER(pseuds.handle) LIKE LOWER(?))";
            Ok(SqlFragment::new(sql).with_bind(format!("%{}%", value)))
        }
        QueryField::Fandom => {
            let sql = "(EXISTS (SELECT 1 FROM work_tags wt JOIN taxonomy_nodes tn ON tn.id = wt.node_id WHERE wt.work_id = works.id AND tn.kind = 'fandom' AND tn.norm = ?))";
            Ok(SqlFragment::new(sql).with_bind(value.to_lowercase().trim().to_owned()))
        }
        QueryField::Character => {
            let sql = "(EXISTS (SELECT 1 FROM work_tags wt JOIN taxonomy_nodes tn ON tn.id = wt.node_id WHERE wt.work_id = works.id AND tn.kind = 'character' AND tn.norm = ?))";
            Ok(SqlFragment::new(sql).with_bind(value.to_lowercase().trim().to_owned()))
        }
        QueryField::Relationship => {
            let sql = "(EXISTS (SELECT 1 FROM work_tags wt JOIN taxonomy_nodes tn ON tn.id = wt.node_id WHERE wt.work_id = works.id AND tn.kind = 'ship' AND tn.norm = ?))";
            Ok(SqlFragment::new(sql).with_bind(value.to_lowercase().trim().to_owned()))
        }
        QueryField::Tag => {
            let sql = "(EXISTS (SELECT 1 FROM work_tags wt JOIN taxonomy_nodes tn ON tn.id = wt.node_id WHERE wt.work_id = works.id AND tn.kind = 'tag' AND tn.norm = ?))";
            Ok(SqlFragment::new(sql).with_bind(value.to_lowercase().trim().to_owned()))
        }
        QueryField::Mood => {
            let sql = "(EXISTS (SELECT 1 FROM work_moods wm JOIN taxonomy_nodes tn ON tn.id = wm.node_id WHERE wm.work_id = works.id AND tn.kind = 'mood' AND tn.norm = ?))";
            Ok(SqlFragment::new(sql).with_bind(value.to_lowercase().trim().to_owned()))
        }
        QueryField::Summary => {
            let sql = "(LOWER(works.summary) LIKE LOWER(?))";
            Ok(SqlFragment::new(sql).with_bind(format!("%{}%", value)))
        }
        QueryField::Body => {
            let sql = "(LOWER(works_index.body_text) LIKE LOWER(?))";
            Ok(SqlFragment::new(sql).with_bind(format!("%{}%", value)))
        }
        QueryField::Language => {
            let sql = "(works.language = ?)";
            Ok(SqlFragment::new(sql).with_bind(value.trim().to_owned()))
        }
        QueryField::Status => {
            let sql = "(works.lifecycle = ?)";
            Ok(SqlFragment::new(sql).with_bind(value.trim().to_owned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::parse_query;

    #[test]
    fn render_text_produces_like() {
        let ast = parse_query("winter").unwrap();
        let frag = render_query(&ast).unwrap();
        assert!(frag.sql.contains("LIKE"));
        assert_eq!(frag.binds.len(), 3);
        assert_eq!(frag.binds[0], "%winter%");
    }

    #[test]
    fn render_fielded_fandom_uses_exists() {
        let ast = parse_query("fandom:harrypotter").unwrap();
        let frag = render_query(&ast).unwrap();
        assert!(frag.sql.contains("EXISTS"));
        assert!(frag.sql.contains("work_tags"));
        assert_eq!(frag.binds, vec!["harrypotter".to_owned()]);
    }

    #[test]
    fn render_and_joins_with_and() {
        let ast = parse_query("fandom:x AND tag:y").unwrap();
        let frag = render_query(&ast).unwrap();
        assert!(frag.sql.contains(" AND "));
        assert_eq!(frag.binds.len(), 2);
    }

    #[test]
    fn render_or_joins_with_or() {
        let ast = parse_query("a OR b").unwrap();
        let frag = render_query(&ast).unwrap();
        assert!(frag.sql.contains(" OR "));
    }

    #[test]
    fn render_not_wraps_in_not() {
        let ast = parse_query("NOT a").unwrap();
        let frag = render_query(&ast).unwrap();
        assert!(frag.sql.starts_with("NOT "));
    }

    #[test]
    fn render_implicit_and() {
        let ast = parse_query("a b c").unwrap();
        let frag = render_query(&ast).unwrap();
        assert!(frag.sql.contains("AND"));
    }

    #[test]
    fn render_language_uses_equality() {
        let ast = parse_query("language:en").unwrap();
        let frag = render_query(&ast).unwrap();
        assert!(frag.sql.contains("works.language = ?"));
        assert_eq!(frag.binds, vec!["en".to_owned()]);
    }

    #[test]
    fn render_status_uses_equality() {
        let ast = parse_query("status:published").unwrap();
        let frag = render_query(&ast).unwrap();
        assert!(frag.sql.contains("works.lifecycle = ?"));
        assert_eq!(frag.binds, vec!["published".to_owned()]);
    }

    #[test]
    fn render_parentheses() {
        let ast = parse_query("(a OR b) AND c").unwrap();
        let frag = render_query(&ast).unwrap();
        assert!(frag.sql.contains("AND"));
        assert!(frag.sql.contains("OR"));
    }

    #[test]
    fn render_fielded_normalizes_case() {
        let ast = parse_query("fandom:MyFandom").unwrap();
        let frag = render_query(&ast).unwrap();
        assert_eq!(frag.binds, vec!["myfandom".to_owned()]);
    }

    #[test]
    fn render_status_published_matches_lifecycle() {
        let ast = parse_query("status:published").unwrap();
        let frag = render_query(&ast).unwrap();
        assert_eq!(frag.binds, vec!["published".to_owned()]);
    }

    // --- Comparison rendering ------------------------------------------------

    #[test]
    fn render_words_greater_than_binds_one_integer() {
        let ast = parse_query("words:>10000").unwrap();
        let frag = render_query(&ast).unwrap();
        assert_eq!(frag.binds, vec!["10000".to_owned()]);
        assert!(
            frag.sql.contains(">"),
            "expected a > comparison: {}",
            frag.sql
        );
        assert!(frag.sql.contains("chapter_revisions"), "{}", frag.sql);
    }

    #[test]
    fn each_operator_renders_its_own_sql() {
        for (query, expected) in [
            ("words:>100", ">"),
            ("words:>=100", ">="),
            ("words:<100", "<"),
            ("words:<=100", "<="),
        ] {
            let ast = parse_query(query).unwrap();
            let frag = render_query(&ast).unwrap();
            assert!(
                frag.sql.contains(&format!("{expected} CAST(? AS BIGINT)")),
                "{query} rendered as: {}",
                frag.sql
            );
        }
    }

    #[test]
    fn a_numeric_comparison_coalesces_to_zero() {
        // A work with no chapters must still satisfy `words:>=0`, and a work
        // never kudosed must still satisfy `kudos:>=0`. Without COALESCE both
        // sum/select to NULL and the comparison is NULL, so neither matches.
        for query in ["words:>=0", "kudos:>=0"] {
            let ast = parse_query(query).unwrap();
            let frag = render_query(&ast).unwrap();
            assert!(
                frag.sql.contains("COALESCE"),
                "{query} must coalesce, rendered: {}",
                frag.sql
            );
        }
    }

    #[test]
    fn kudos_reads_the_aggregate_table() {
        let ast = parse_query("kudos:>5").unwrap();
        let frag = render_query(&ast).unwrap();
        assert!(
            frag.sql.contains("work_metric_aggregates"),
            "expected the aggregate table, rendered: {}",
            frag.sql
        );
    }

    #[test]
    fn a_comparison_combines_with_other_terms() {
        let ast = parse_query("words:>1000 AND tag:romance").unwrap();
        let frag = render_query(&ast).unwrap();
        assert!(frag.sql.contains(" AND "));
        // One bind for the comparison, one for the taxonomy lookup.
        assert_eq!(frag.binds.len(), 2);
    }

    #[test]
    fn a_foreign_comparison_field_is_rejected() {
        // `replies:>50` is a forum query. On the works surface it must say so
        // rather than return zero rows for an invisible reason.
        let ast = parse_query("replies:>50").unwrap();
        let err = render_query(&ast).unwrap_err();
        assert!(
            err.message.contains("replies") && err.message.contains("works surface"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn a_foreign_equality_field_is_rejected() {
        for query in [
            "category:meta",
            "rec:true",
            "user:nightowl",
            "rank:character",
            "note:reread",
        ] {
            let ast = parse_query(query).unwrap();
            let err = render_query(&ast).unwrap_err();
            assert!(
                err.message.contains("not a works field"),
                "{query} rendered instead of erroring: {:?}",
                err.message
            );
        }
    }

    #[test]
    fn the_error_names_the_entity_the_field_belongs_to() {
        let ast = parse_query("category:meta").unwrap();
        let err = render_query(&ast).unwrap_err();
        assert!(err.message.contains("forum"), "got: {}", err.message);
    }

    #[test]
    fn a_hand_built_non_numeric_comparison_is_rejected() {
        // `render_query` is public, so the parser's integer check is not the
        // only guard.
        let ast = crate::query::QueryAst::Comparison(
            QueryField::Words,
            CompareOp::Gt,
            "not-a-number".to_owned(),
        );
        let err = render_query(&ast).unwrap_err();
        assert!(err.message.contains("integer"), "got: {}", err.message);
    }

    #[test]
    fn a_numeric_equality_comparison_is_accepted() {
        // `words:5000` is equality, not a comparison, and must not 500.
        let ast = parse_query("words:5000").unwrap();
        let frag = render_query(&ast).unwrap();
        assert!(frag.sql.contains("= CAST(? AS BIGINT)"), "{}", frag.sql);
        assert!(!frag.sql.contains(">="), "{}", frag.sql);
    }
}
