//! Query SQL rendering: compile the AST into SQL fragments.
//!
//! Spec §15.3. Pure functions — no I/O.

use crate::query::{
    CharacterAssertion, CompareOp, QueryAst, QueryError, QueryField, RelationshipAssertion,
    ScopedPredicate,
};

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
                "(LOWER(works.title) LIKE LOWER(?) ESCAPE '\\' OR LOWER(works.summary) LIKE LOWER(?) ESCAPE '\\' OR LOWER(works_index.body_text) LIKE LOWER(?) ESCAPE '\\')"
                    .to_owned();
            let pattern = format!("%{}%", text);
            Ok(SqlFragment::new(sql)
                .with_bind(pattern.clone())
                .with_bind(pattern.clone())
                .with_bind(pattern))
        }
        QueryAst::Phrase(phrase) => {
            let sql = "(LOWER(works_index.body_text) LIKE LOWER(?) ESCAPE '\\')";
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
        QueryAst::ExistsCharacter(a) => render_exists_character(a),
        QueryAst::ExistsRelationship(a) => render_exists_relationship(a),
        QueryAst::Scoped(p) => render_scoped(p),
        QueryAst::MinMatch { needed, terms } => render_min_match(*needed, terms),
        QueryAst::Expand { field, term, depth } => render_expand(*field, term, *depth),
        QueryAst::Not(inner) => {
            let inner = render_node(inner)?;
            // `NOT NULL` is NULL, not true, so a negated predicate over columns
            // that may be NULL -- `works.summary`, `works_index.body_text` --
            // silently matches nothing at all. COALESCE resolves the unknown to
            // a definite false *before* the negation, which is what the reader
            // meant by "not spoiler": no match, rather than no answer.
            //
            // A taxonomy term is an `EXISTS (...)`, which is never NULL, and
            // coalescing there would only add noise -- so the coalesce is
            // applied to the inner fragment only when it can be NULL.
            let coalesce = needs_null_guard(&inner.sql);
            let sql = if coalesce {
                format!("NOT COALESCE({}, false)", inner.sql)
            } else {
                format!("NOT ({})", inner.sql)
            };
            Ok(SqlFragment {
                sql,
                binds: inner.binds,
            })
        }
    }
}

/// Compile §15.3's `ExistsCharacter`.
///
/// ## The correlation rule, and why this function is shaped the way it is
///
/// §15.3: *"Compile bound predicates into SQL `EXISTS` clauses. Never allow one
/// character to satisfy another character's attributes."*
///
/// The naive compilation of "character A, prominence protagonist, attribute
/// vampire" is three independent clauses:
///
/// ```sql
/// EXISTS (SELECT 1 FROM work_characters wc WHERE wc.work_id = works.id AND wc.character_node_id = ?)
/// AND EXISTS (SELECT 1 FROM work_characters wc WHERE wc.work_id = works.id AND wc.prominence = ?)
/// AND EXISTS (SELECT 1 FROM work_character_attributes a WHERE a.work_id = works.id AND a.attribute_node_id = ?)
/// ```
///
/// and it is **wrong**. Nothing correlates them, so a work where *Alice* is the
/// protagonist and *Bob* is the vampire matches — a result assembled from two
/// different characters who have nothing to do with each other. Measured against
/// real PostgreSQL 15 during this build: the uncorrelated form returns 1 on exactly
/// that fixture, while the correlated form below returns 0. That difference is the
/// whole clause.
///
/// So every bound becomes a predicate **on the one `work_characters` row**, and
/// the attributes are tested by an `EXISTS` that names *that row's*
/// `character_node_id`. There is exactly one character per assertion, which is
/// also why `CharacterAssertion` holds a single `character_id` rather than a list:
/// a list would make "the same character" unexpressible.
///
/// Alias `wc` is fixed and short because it appears inside a nested `EXISTS`
/// three times; a generated alias would be unreadable in a log line.
///
/// `taxonomy_nodes` is joined for attributes and roles because those are node ids
/// and a reader searching by name must find the node. `norm` is matched rather
/// than `canonical` for the same reason every other taxonomy predicate in this
/// file uses `norm`: the user types lowercase.
fn render_exists_character(a: &CharacterAssertion) -> Result<SqlFragment, QueryError> {
    let mut binds: Vec<String> = vec![a.character_id.clone()];
    let mut clauses: Vec<String> = vec!["wc.character_node_id = ?".to_owned()];

    if !a.prominence.is_empty() {
        let list = placeholders(a.prominence.len());
        clauses.push(format!("wc.prominence IN ({list})"));
        binds.extend(a.prominence.iter().cloned());
    }
    // `Some(true)`/`Some(false)` are the only two values, so this is a literal
    // rather than a bind. That is safe precisely because the type admits nothing
    // else -- the alternative, a bind, would put a boolean on the wire to express
    // a two-valued domain, and `None` would still have to be omitted separately.
    if let Some(v) = a.is_pov {
        clauses.push(format!("wc.is_pov = {}", if v { 1 } else { 0 }));
    }
    if !a.roles.is_empty() {
        let list = placeholders(a.roles.len());
        clauses.push(format!(
            "EXISTS (SELECT 1 FROM work_character_attributes ra \
             JOIN taxonomy_nodes rt ON rt.id = ra.attribute_node_id \
             WHERE ra.work_id = wc.work_id AND ra.character_node_id = wc.character_node_id \
               AND rt.norm IN ({list}))"
        ));
        binds.extend(a.roles.iter().map(|r| r.to_lowercase()));
    }

    // `attributes_all` is one correlated EXISTS PER attribute rather than a
    // `GROUP BY ... HAVING COUNT(*) = n`. The aggregate form is shorter and is the
    // wrong shape: it counts attributes without checking *which*, so a work with
    // three unrelated attributes would satisfy a two-attribute query. N small
    // correlated subqueries are both correct and predictable, and the compiler is
    // not the hot path — the index on (attribute_node_id, work_id) is.
    for attribute in &a.attributes_all {
        clauses.push(
            "EXISTS (SELECT 1 FROM work_character_attributes aa \
             JOIN taxonomy_nodes at ON at.id = aa.attribute_node_id \
             WHERE aa.work_id = wc.work_id AND aa.character_node_id = wc.character_node_id \
               AND at.norm = ?)"
                .to_owned(),
        );
        binds.push(attribute.to_lowercase());
    }

    if !a.attributes_any.is_empty() {
        let list = placeholders(a.attributes_any.len());
        clauses.push(format!(
            "EXISTS (SELECT 1 FROM work_character_attributes ay \
             JOIN taxonomy_nodes aty ON aty.id = ay.attribute_node_id \
             WHERE ay.work_id = wc.work_id AND ay.character_node_id = wc.character_node_id \
               AND aty.norm IN ({list}))"
        ));
        binds.extend(a.attributes_any.iter().map(|v| v.to_lowercase()));
    }

    let sql = format!(
        "(EXISTS (SELECT 1 FROM work_characters wc WHERE wc.work_id = works.id AND {}))",
        clauses.join(" AND ")
    );
    Ok(SqlFragment { sql, binds })
}

/// Compile §15.3's `ExistsRelationship`.
///
/// ## Why the participant test is inside the relationship's `EXISTS`
///
/// "Any romantic pairing involving X" is an `EXISTS` over relationships whose
/// participants include X. The correlation that matters is between the
/// **relationship row** and the **participant**: `ship_participants` is joined on
/// `ship_node_id` *inside* the same `EXISTS`, so a relationship counts only if
/// that relationship's own participants include X.
///
/// `excluded_participants` compiles to a `NOT EXISTS` nested inside the same
/// `EXISTS`, correlated on `wr.id`. Putting it at the top level instead — as
/// `AND NOT EXISTS(...)` — would exclude every work that pairs X with *anyone*,
/// which is the opposite of "X with anyone except Y".
///
/// `dynamics` is a comma-separated list, so membership is tested with a `LIKE`
/// on a delimited pattern rather than `=`. The delimiters matter: `enemies_to_lovers`
/// must not match a hypothetical `enemies_to_lovers_2`, and
/// `',enemies_to_lovers,' LIKE '%,enemies_to_lovers,%'` is what prevents that.
/// Compile §15.4.1.1's `Scoped` — `ship:(with:"A" with:"B" type:romantic)`.
///
/// ## Why the bounds live INSIDE one EXISTS
///
/// Every clause below is inside a single `EXISTS (SELECT 1 FROM work_relationships
/// wr WHERE wr.work_id = works.id AND ...)`. That is the whole point of the node,
/// and it is the same rule `render_exists_character` exists to enforce: a `with:`
/// and a `type:` that resolved against *different* relationship rows would match a
/// work where A/B is present somehow and something else entirely is romantic —
/// wrong in a way that looks right.
///
/// The two shapes are distinguishable in the output, which is what makes a test able
/// to assert the correlation rather than the presence of the right words. The wrong
/// compilation emits two sibling `EXISTS`; the right one emits one.
///
/// ## `with:` is a participant set, so it needs all three clauses
///
/// Same measurement as `relationship:` in M46-01, and the same three-clause
/// requirement: naming every participant of the ship, naming no participant the ship
/// does not have, and having every named character actually be a participant. With
/// only the first two, asking for the poly `{alice,bob,carol}` also matches the duo
/// `{alice,bob}` — alice and bob *are* named, and the duo has no unnamed participant.
///
/// Names are bound one each, and membership is a `LIKE` on a `//`-delimited pattern.
/// `string_to_array` is PostgreSQL-only, `instr` is SQLite-only, and counting
/// separators by `length` arithmetic is wrong for any name containing a space.
fn render_scoped(p: &ScopedPredicate) -> Result<SqlFragment, QueryError> {
    if p.relation != QueryField::Relationship {
        return Err(QueryError::new(
            format!(
                "{} has no single row to scope a sub-predicate to; only `relationship:` \
                 does",
                p.relation.as_str()
            ),
            0,
        ));
    }

    // Group the bounds: every `with:` names a participant, the rest constrain the row.
    let mut withs: Vec<&str> = Vec::new();
    let mut row_bounds: Vec<(&str, &str)> = Vec::new();
    for f in &p.fields {
        match f.key.as_str() {
            "with" | "participant" => withs.push(f.value.as_str()),
            "type" | "rel_type" => row_bounds.push(("wr.rel_type", f.value.as_str())),
            "prominence" => row_bounds.push(("wr.prominence", f.value.as_str())),
            "label" => row_bounds.push(("wr.label", f.value.as_str())),
            other => {
                return Err(QueryError::new(
                    format!(
                        "{other:?} is not a relationship bound; expected one of \
                         with, type, prominence, label"
                    ),
                    0,
                ))
            }
        }
    }

    if withs.is_empty() {
        return Err(QueryError::new(
            "a scoped relationship needs at least one `with:` naming a participant; \
             without one it would match every relationship in the instance",
            0,
        ));
    }

    let mut binds: Vec<String> = Vec::new();
    let joined = format!("/{}//", withs.join("//"));

    // Plain string literals, not `format!`: none of these has an interpolation, and
    // `format!` on a constant is a runtime no-op that reads like something is being
    // substituted. The `(a)`/`(b)` split is the whole point of the pair -- `a`
    // names every participant, `b` admits no unnamed one -- so a future edit that
    // drops `b` would silently let a ship value name a subset of its participants.
    let mut clauses: Vec<String> = vec![
        // (a) every participant of the ship is named in the value
        "EXISTS (SELECT 1 FROM ship_participants sp WHERE sp.ship_node_id = wr.ship_node_id \
         AND (? || '%') LIKE '%/' || sp.character_node_id || '/%')"
            .to_owned(),
        // (b) no participant of the ship is unnamed in the value
        "NOT EXISTS (SELECT 1 FROM ship_participants sm WHERE sm.ship_node_id = wr.ship_node_id \
         AND (? || '%') NOT LIKE '%/' || sm.character_node_id || '/%')"
            .to_owned(),
    ];
    // (a) and (b) share one bind of the joined value, so it is bound twice. There is
    // no back-reference: `rewrite_placeholders` numbers every `?` in order.
    binds.push(joined.clone());
    binds.push(joined);

    // (c) every name in the value is a participant of the ship
    for w in &withs {
        clauses.push(
            "EXISTS (SELECT 1 FROM ship_participants sq WHERE sq.ship_node_id = wr.ship_node_id \
             AND sq.character_node_id = ?)"
                .to_owned(),
        );
        binds.push((*w).to_owned());
    }

    for (column, value) in row_bounds {
        clauses.push(format!("{column} = ?"));
        binds.push(value.to_owned());
    }

    let sql = format!(
        "(EXISTS (SELECT 1 FROM work_relationships wr WHERE wr.work_id = works.id AND {}))",
        clauses.join(" AND ")
    );
    Ok(SqlFragment { sql, binds })
}

/// Compile §15.4.1.2's `min_match(n, ...)`.
///
/// Compiled as a count over a list of parenthesised predicates:
///
/// ```sql
/// (SELECT COUNT(*) FROM (SELECT (<p1>) AS ok UNION ALL SELECT (<p2>) AS ok ...) s) >= n
/// ```
///
/// `COUNT(*) >= n` rather than `COUNT(*) = n`: "at least two" is what the reader
/// asked, and a work satisfying all three must still match.
///
/// Each arm is a sub-select rather than a bare term, because the arms must be
/// scored independently and `UNION ALL` is what keeps them separate -- a bare
/// `p1 UNION ALL p2` would return rows of booleans and have no row to score.
///
/// The NULL handling is not incidental. A predicate over a NULL-able column
/// -- `works.summary`, `works_index.body_text` -- yields NULL, not false, and
/// `SUM` skips NULL, so one NULL-valued term would drop the whole count to NULL and
/// the `>= n` comparison would be NULL, which a WHERE clause reads as false: the work
/// would vanish from a result it belongs in. Scoring each arm to 1/0 first means
/// NULL is unsatisfied, which is what it means.
fn render_min_match(needed: usize, terms: &[QueryAst]) -> Result<SqlFragment, QueryError> {
    if terms.is_empty() {
        return Err(QueryError::new(
            "min_match needs at least one term to count",
            0,
        ));
    }
    if needed == 0 || needed > terms.len() {
        return Err(QueryError::new(
            format!(
                "min_match needs {needed} of {} terms, which can never be satisfied",
                terms.len()
            ),
            0,
        ));
    }

    let mut arms = Vec::with_capacity(terms.len());
    let mut binds = Vec::new();
    for t in terms {
        let f = render_node(t)?;
        // CASE WHEN ... THEN 1 ELSE 0 END, and not COALESCE(pred, 0).
        //
        // Measured on both engines, because the two obvious forms are both broken
        // here and neither fails at compile time:
        //   COALESCE(pred, 0)  -- PostgreSQL: "COALESCE types boolean and integer
        //                          cannot be matched". SQLite accepts it.
        //   SUM(pred)          -- PostgreSQL: "function sum(boolean) does not exist".
        //   pred::int          -- SQLite: parse error.
        // CASE WHEN is the form both accept, and it scores a NULL predicate as 0 on
        // both -- verified against a NULL-able column, where `pred` is NULL and the
        // score is 0. That is the behaviour needed: a work whose summary is NULL has
        // not satisfied `summary:"x"`, and must count as unsatisfied rather than
        // making the whole SUM NULL and the row silently vanish.
        arms.push(format!(
            "SELECT CASE WHEN ({}) THEN 1 ELSE 0 END AS ok",
            f.sql
        ));
        binds.extend(f.binds);
    }

    let sql = format!(
        "((SELECT SUM(ok) FROM ({}) s) >= {})",
        arms.join(" UNION ALL "),
        needed
    );
    Ok(SqlFragment { sql, binds })
}

/// Compile §15.4.1.3's `tag:"X"+children`.
///
/// The term's own node resolves the reader's name to a node id, and the closure
/// supplies the descendants. Both the name and the descendant set are matched
/// against `work_tags`, so a work carrying the parent OR any descendant matches.
///
/// `rel = 'parent'` is in the predicate, not implied: `taxonomy_closure` also holds
/// `implies` rows, and following those would silently widen every expansion. §9's
/// risk list says implication edges are curator-only and default off, and this is
/// where "off" is enforced.
///
/// The depth cap is a `depth <= n` bound on the closure, which is a column, so the
/// cap costs nothing and cannot be forgotten. `None` means the instance default and
/// omits the bound — the closure's own size then bounds it, which is why
/// `QueryBudget::max_expansion` is checked at parse time rather than here.
fn render_expand(
    field: QueryField,
    term: &str,
    depth: Option<u32>,
) -> Result<SqlFragment, QueryError> {
    // Resolve the reader's name to a node id, then match either the node itself or
    // any descendant of it. Two EXISTS in one AND-of-OR shape rather than a join,
    // so a work with no tags at all is excluded rather than producing a NULL row.
    let base = render_node(&QueryAst::Fielded(field, term.to_owned()))?;
    let depth_bound = match depth {
        Some(n) => format!(" AND tc.depth <= {n}"),
        None => String::new(),
    };

    let sql = format!(
        "({base_sql} OR EXISTS (SELECT 1 FROM taxonomy_closure tc \
         WHERE tc.ancestor_id = (SELECT tn.id FROM taxonomy_nodes tn WHERE tn.norm = ? AND tn.kind = ?) \
           AND tc.rel = 'parent'{depth_bound} \
           AND EXISTS (SELECT 1 FROM work_tags wt WHERE wt.work_id = works.id \
             AND wt.node_id = tc.descendant_id)))",
        base_sql = base.sql
    );
    let mut binds = base.binds;
    // The closure needs the SAME name the base field resolved, so it re-binds rather
    // than reusing a placeholder: there is no back-reference to the first one.
    let norm = term.to_lowercase().trim().to_owned();
    binds.push(norm);
    binds.push(field_taxonomy_kind(field).to_owned());
    Ok(SqlFragment { sql, binds })
}

/// The `taxonomy_nodes.kind` a taxonomy field's values live under.
fn field_taxonomy_kind(field: QueryField) -> &'static str {
    match field {
        QueryField::Fandom => "fandom",
        QueryField::Character => "character",
        QueryField::Relationship => "ship",
        QueryField::Mood => "mood",
        // Tag covers every remaining taxonomy kind -- trope, genre, setting,
        // format, pov, tense, freeform -- so it cannot name one kind. `IN` over
        // the kinds a tag may be is the honest form, and the caller's base
        // predicate already restricts by the tag's own semantics.
        _ => "tag",
    }
}

fn render_exists_relationship(a: &RelationshipAssertion) -> Result<SqlFragment, QueryError> {
    if a.participant_any.is_empty() {
        return Err(QueryError::new(
            "a relationship query needs at least one participant: without one it \
             matches every relationship in the instance",
            0,
        ));
    }
    let mut binds: Vec<String> = vec![];
    let mut clauses: Vec<String> = vec![];

    let list = placeholders(a.participant_any.len());
    clauses.push(format!(
        "EXISTS (SELECT 1 FROM ship_participants sp \
         WHERE sp.ship_node_id = wr.ship_node_id AND sp.character_node_id IN ({list}))"
    ));
    binds.extend(a.participant_any.iter().cloned());

    if !a.kind_any.is_empty() {
        let list = placeholders(a.kind_any.len());
        clauses.push(format!("wr.rel_type IN ({list})"));
        binds.extend(a.kind_any.iter().cloned());
    }
    if !a.prominence.is_empty() {
        let list = placeholders(a.prominence.len());
        clauses.push(format!("wr.prominence IN ({list})"));
        binds.extend(a.prominence.iter().cloned());
    }
    for dynamic in &a.dynamics {
        clauses.push("(wr.dynamics IS NOT NULL AND ',' || wr.dynamics || ',' LIKE ?)".to_owned());
        // The delimiters are part of the pattern, not decoration: without them
        // `enemies` would match `enemies_to_lovers`.
        binds.push(format!("%,{},%", dynamic));
    }
    if !a.excluded_participants.is_empty() {
        let list = placeholders(a.excluded_participants.len());
        clauses.push(format!(
            "NOT EXISTS (SELECT 1 FROM ship_participants sx \
             WHERE sx.ship_node_id = wr.ship_node_id AND sx.character_node_id IN ({list}))"
        ));
        binds.extend(a.excluded_participants.iter().cloned());
    }

    let sql = format!(
        "(EXISTS (SELECT 1 FROM work_relationships wr WHERE wr.work_id = works.id AND {}))",
        clauses.join(" AND ")
    );
    Ok(SqlFragment { sql, binds })
}

/// A comma-separated list of `n` placeholders.
///
/// Both engines accept `?`; `TestDb::sql` rewrites them for PostgreSQL. Written
/// as a helper because five call sites each hand-rolling a `join(",")` is five
/// chances to get the count subtly wrong, and a wrong count is a bind that does
/// not match its placeholder — an error at execution, far from the compiler.
fn placeholders(n: usize) -> String {
    std::iter::repeat_n("?", n).collect::<Vec<_>>().join(",")
}

/// Whether a rendered predicate can evaluate to NULL rather than true/false.
///
/// A `LIKE` against a nullable column is the only source. `EXISTS` and the
/// numeric comparisons are already total: `EXISTS` is a definite false when it
/// finds nothing, and every comparison this module emits sits inside a
/// `COALESCE`. Deciding it from the text is a heuristic, and a wrong `false`
/// only means an extra `COALESCE` -- which is safe -- whereas missing a `true`
/// is the bug this guards against.
fn needs_null_guard(sql: &str) -> bool {
    sql.contains(" LIKE ")
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
    // A date field is compared as a date. It has to be checked before the
    // numeric arm, because the numeric arm's `parse::<i64>` would reject
    // `2026-08-01` outright -- and the media search runs *every* field through
    // `render_query` to validate a query, so that one mismatch turned a 200
    // into a 422 for a reader who had typed a perfectly good date filter.
    if let Some(column) = date_column(field) {
        let date = parse_query_date(value).ok_or_else(|| {
            QueryError::new(
                format!(
                    "{} {} takes a YYYY-MM-DD date, got {:?}",
                    field.as_str(),
                    op.as_str(),
                    value
                ),
                0,
            )
        })?;
        // SUBSTR for the same reason the range arm uses it: `2026-08-01` means
        // the whole day, not the midnight that started it. Without it a work
        // published at 14:00 on the boundary day reads as *after* a
        // `>=2026-08-01` the reader meant as "on or after that day".
        return Ok(
            SqlFragment::new(format!("(SUBSTR({column}, 1, 10) {} ?)", op.sql())).with_bind(date),
        );
    }

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

/// The works-surface column behind a date field, if this is one.
///
/// Separate from `numeric_column` because the two need different SQL: a date
/// is ISO-8601 text that orders lexicographically, and casting one to an
/// integer would compare `2026-08-01` as the number 2026.
fn date_column(field: &QueryField) -> Option<&'static str> {
    match field {
        QueryField::Published => Some("works.published_at"),
        QueryField::Updated => Some("works.updated_at"),
        _ => None,
    }
}

/// Parse a `YYYY-MM-DD` query bound, returning it in the same shape the column
/// stores so the comparison is a plain string compare.
fn parse_query_date(value: &str) -> Option<String> {
    let date = time::Date::parse(
        value,
        &time::macros::format_description!("[year]-[month]-[day]"),
    )
    .ok()?;
    Some(date.to_string())
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
            // Name both halves: what is wrong, and where the field does belong.
            // "not a comparable field on the works surface (comparable here:
            // words, kudos)" leaves a reader who typed `replies:>50` with no
            // idea that the forum search is the place for it.
            return Err(QueryError::new(
                format!(
                    "{} is a {} field, not a comparable works field \
                     (comparable here: words, kudos)",
                    other.as_str(),
                    other.entity().as_str()
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
            let sql = "(LOWER(works.title) LIKE LOWER(?) ESCAPE '\\')";
            Ok(SqlFragment::new(sql).with_bind(format!("%{}%", value)))
        }
        QueryField::Author => {
            let sql = "(LOWER(pseuds.handle) LIKE LOWER(?) ESCAPE '\\')";
            Ok(SqlFragment::new(sql).with_bind(format!("%{}%", value)))
        }
        QueryField::Fandom => {
            let sql = "(EXISTS (SELECT 1 FROM work_tags wt JOIN taxonomy_nodes tn ON tn.id = wt.node_id WHERE wt.work_id = works.id AND tn.kind = 'fandom' AND tn.norm = ?))";
            Ok(SqlFragment::new(sql).with_bind(value.to_lowercase().trim().to_owned()))
        }
        // `character:` reads `work_characters`, NOT `work_tags` with
        // `kind = 'character'`.
        //
        // The old rendering was a tag lookup, and nothing in the tree ever writes
        // such a tag — verified by grep: 0103 is the only migration that names
        // `kind = 'character'` at all. So `character:Alice` matched nothing, while
        // reading the table that does hold characters costs the same and can
        // express prominence and attributes (which a tag lookup cannot, being a
        // flat equality).
        //
        // The reader types a *name*; the store keys on a *node id*. Resolving
        // `norm = ?` against `taxonomy_nodes` inside the subquery keeps the parser
        // pure — §15.4 requires "the same typed AST as the visual filter builder",
        // and a parser that had to look ids up would need I/O.
        QueryField::Character => {
            let sql = "(EXISTS (SELECT 1 FROM work_characters wc \
                       JOIN taxonomy_nodes tn ON tn.id = wc.character_node_id \
                       WHERE wc.work_id = works.id AND tn.kind = 'character' \
                         AND tn.norm = ?))";
            Ok(SqlFragment::new(sql).with_bind(value.to_lowercase().trim().to_owned()))
        }
        // `relationship:` names a pairing, and a pairing IS its participant set
        // (0103's `ship_participants`, ordered so A/B and B/A are one node). So the
        // reader's `relationship:"Alice/Bob"` matches when some ship the work claims
        // a relationship about has participant set exactly {alice, bob}.
        //
        // "Exactly" needs THREE clauses, and the third is the one that is easy to
        // miss. Measured on real PostgreSQL 15 while writing this: with only the
        // first two, asking for the poly {alice,bob,carol} ALSO matched the duo
        // {alice,bob} — alice and bob are named (clause 1 holds) and the duo has no
        // unnamed participant (clause 2 holds). Only clause 3 rejects it, because the
        // duo has no carol:
        //
        //   (a) every participant of the ship is named in the value
        //   (b) no participant of the ship is unnamed in the value
        //   (c) every name in the value is a participant of the ship
        //
        // (c) is per-name, and the names are split in RUST rather than in SQL. That
        // is deliberate: `string_to_array` does not exist in SQLite and `instr` does
        // not exist in PostgreSQL, so any approach that splits the value inside the
        // statement needs either a dialect arm or a string function the other engine
        // lacks. One bind per name needs neither. Counting names with
        // `length - length(replace(...))` was tried first and is worse: a
        // participant whose name contains a space makes the count wrong.
        //
        // (a) and (b) still share one bind — the whole joined value — because they
        // are both "is this participant named in the value", and the `//` separator
        // is what lets `%/alice/%` avoid matching `alice_v2` while still matching a
        // trailing participant (a single `/` separator leaves the last name followed
        // by `%` instead of `/`, so a duo stops matching itself).
        QueryField::Relationship => {
            let names: Vec<String> = value
                .split('/')
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_lowercase)
                .collect();

            // An empty `relationship:` would otherwise match every work with any
            // relationship at all: (a) and (b) are vacuously true for any ship, and
            // (c) would have zero names to check. Refusing is the honest rendering.
            if names.is_empty() {
                return Err(QueryError::new(
                    "relationship: needs at least one participant, as \
                     relationship:\"Alice/Bob\"",
                    0,
                ));
            }

            // Every name must be a participant of the ship. One placeholder per
            // name, generated here, so no engine has to split a string.
            //
            // Each arm is an `EXISTS`, not a bare `(SELECT 1 ...)`. Measured on real
            // PostgreSQL 15: a bare `SELECT 1` subquery in an AND is an integer, and
            // the engine rejects the whole statement with `argument of AND must be
            // type boolean, not type integer`. `EXISTS` is what turns it into the
            // boolean the surrounding WHERE needs.
            //
            // The placeholders are BARE `?`, not `?1`/`?3`. `Database::sql` rewrites
            // bare `?` to `$n` positionally for PostgreSQL (`rewrite_placeholders`)
            // and offers no back-reference form, so a numbered `?1` survives the
            // rewrite as a literal `?1` and PostgreSQL rejects it. A value used twice
            // therefore needs its bind twice -- which is why two names produce three
            // binds, and why the bind count does not track the name count.
            let all_named: String = names
                .iter()
                .map(|_| {
                    "EXISTS (SELECT 1 FROM ship_participants sq \
                       WHERE sq.ship_node_id = wr.ship_node_id \
                         AND sq.character_node_id = ?)"
                        .to_owned()
                })
                .collect::<Vec<_>>()
                .join(" AND ");

            let sql = format!(
                "(EXISTS (SELECT 1 FROM work_relationships wr \
                   WHERE wr.work_id = works.id \
                     AND EXISTS (SELECT 1 FROM ship_participants sp \
                                 WHERE sp.ship_node_id = wr.ship_node_id \
                                   AND (? || '%') LIKE '%/' || sp.character_node_id || '/%') \
                     AND NOT EXISTS (SELECT 1 FROM ship_participants sm \
                                     WHERE sm.ship_node_id = wr.ship_node_id \
                                       AND (? || '%') NOT LIKE '%/' || sm.character_node_id || '/%') \
                     AND {all_named}))"
            );

            // Bind order must match placeholder order exactly: the joined value
            // TWICE (clauses a and b both use it and the rewriter cannot share a
            // bind), then one bind per name (clause c).
            let joined = format!("/{}/", names.join("//"));
            let mut fragment = SqlFragment::new(&sql)
                .with_bind(joined.clone())
                .with_bind(joined);
            for name in &names {
                fragment = fragment.with_bind(name.clone());
            }
            Ok(fragment)
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
            let sql = "(LOWER(works.summary) LIKE LOWER(?) ESCAPE '\\')";
            Ok(SqlFragment::new(sql).with_bind(format!("%{}%", value)))
        }
        QueryField::Body => {
            let sql = "(LOWER(works_index.body_text) LIKE LOWER(?) ESCAPE '\\')";
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
    fn a_negated_free_text_term_is_null_safe() {
        // `NOT (title LIKE ? OR summary LIKE ? OR body LIKE ?)` is NULL --
        // not true -- whenever any of the three arms is NULL. A work with a
        // NULL `summary` and no index row therefore matches *no* negated free
        // text, silently, and the reader's `NOT spoiler` filter does nothing
        // for it. COALESCE turns the unknown into a definite false first, so
        // the negation is about what actually matched rather than about what
        // the database happened to know.
        let ast = parse_query("NOT spoiler").unwrap();
        let frag = render_query(&ast).unwrap();
        assert!(
            frag.sql.contains("COALESCE"),
            "a negated free-text term must coalesce, rendered: {}",
            frag.sql
        );
        // A *positive* term must not grow the same treatment: `title LIKE ?`
        // against a NULL title is correctly false, and coalescing there would
        // only add noise.
        let positive = render_query(&parse_query("spoiler").unwrap()).unwrap();
        assert!(
            !positive.sql.contains("COALESCE"),
            "the positive form is already null-safe, rendered: {}",
            positive.sql
        );
    }

    #[test]
    fn a_negated_phrase_is_null_safe() {
        let ast = parse_query("NOT \"we were never alone\"").unwrap();
        let frag = render_query(&ast).unwrap();
        assert!(
            frag.sql.contains("COALESCE"),
            "a negated phrase reads body_text, which is NULL when unindexed: {}",
            frag.sql
        );
    }

    #[test]
    fn a_negated_fielded_term_is_null_safe() {
        // `summary:` has the same three columns behind it.
        let ast = parse_query("NOT summary:anything").unwrap();
        let frag = render_query(&ast).unwrap();
        assert!(frag.sql.contains("COALESCE"), "rendered: {}", frag.sql);
    }

    #[test]
    fn a_negated_taxonomy_term_is_null_safe() {
        // The taxonomy predicates are `EXISTS (...)`, which is never NULL --
        // it is a definite false. Asserting they were left alone keeps the
        // COALESCE from creeping onto a subquery that does not need it.
        let ast = parse_query("NOT tag:spoiler").unwrap();
        let frag = render_query(&ast).unwrap();
        assert!(
            !frag.sql.contains("COALESCE"),
            "EXISTS is never NULL, so a negated taxonomy term needs no coalesce: {}",
            frag.sql
        );
    }

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
            err.message.contains("replies") && err.message.contains("works"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn a_comparison_error_names_the_surface_the_field_belongs_to() {
        // The actionable half of the message: a reader who typed `replies:>50`
        // needs to learn the forum search is where it goes.
        for (query, entity) in [
            ("replies:>50", "forum"),
            ("rank:>100", "directory"),
            ("works:>10", "user"),
        ] {
            let ast = parse_query(query).unwrap();
            let err = render_query(&ast).unwrap_err();
            assert!(
                err.message.contains(entity),
                "{query} should say the field belongs to {entity}, got: {}",
                err.message
            );
        }
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
    fn a_date_field_compares_as_a_date_not_a_number() {
        // `published:>=2026-08-01` is an ordered comparison on a date column.
        // The numeric arm rejected it, so a query that had worked for months --
        // the media search runs every field through `render_query` to validate,
        // which turned a 200 into a 422 for every reader.
        let ast = parse_query("published:>=2026-08-01").unwrap();
        let frag = render_query(&ast).expect("a date comparison must render");
        // Not `CAST(? AS BIGINT)`: that compares 2026-08-01 as the number 2026
        // and returns the wrong rows without an error.
        assert!(!frag.sql.contains("BIGINT"), "{}", frag.sql);
        assert!(frag.sql.contains(">="), "{}", frag.sql);
        assert!(frag.sql.contains("published_at"), "{}", frag.sql);
    }

    #[test]
    fn every_date_field_compares_as_a_date() {
        // The three works date fields and the two cross-surface ones. A partial
        // fix here would leave `updated:>...` broken, which is exactly how the
        // original regression hid: one field was fixed, the other was not.
        for (q, column) in [
            ("published:>=2026-08-01", "published_at"),
            ("updated:<=2026-09-01", "updated_at"),
        ] {
            let ast = parse_query(q).unwrap();
            let frag = render_query(&ast).unwrap_or_else(|e| panic!("{q}: {}", e.message));
            assert!(frag.sql.contains(column), "{q} -> {}", frag.sql);
            assert!(!frag.sql.contains("BIGINT"), "{q} -> {}", frag.sql);
        }
    }

    #[test]
    fn a_date_comparison_keeps_the_day_precision_the_range_arm_uses() {
        // `published` in the range arm is truncated with SUBSTR(...,1,10) so
        // `2026-08-01` means the whole day rather than the midnight that
        // started it. A comparison that compares the raw timestamp would treat
        // a work published at 14:00 on the boundary day as later than the
        // reader's `>=2026-08-01` when they meant "on or after that day".
        let ast = parse_query("published:>=2026-08-01").unwrap();
        let frag = render_query(&ast).unwrap();
        assert!(frag.sql.contains("SUBSTR("), "{}", frag.sql);
    }

    #[test]
    fn a_date_comparison_with_a_malformed_date_is_refused() {
        // Either the parser or the renderer may refuse it -- both are correct,
        // and the point is that it is not rendered into a comparison.
        let refused = match parse_query("published:>=sometime") {
            Err(_) => true,
            Ok(ast) => render_query(&ast).is_err(),
        };
        assert!(refused, "a malformed date must not render");
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
