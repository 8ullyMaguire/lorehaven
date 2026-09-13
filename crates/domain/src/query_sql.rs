//! Query SQL rendering: compile the AST into SQL fragments.
//!
//! Spec §15.3. Pure functions — no I/O.

use crate::query::{QueryAst, QueryError, QueryField};

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

fn render_fielded(field: &QueryField, value: &str) -> Result<SqlFragment, QueryError> {
    match field {
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
}
