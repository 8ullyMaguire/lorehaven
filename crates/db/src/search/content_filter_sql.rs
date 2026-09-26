//! Server-side content-filter exclusion, rendered per dialect.
//!
//! Spec §46.4: "Content filters are enforced server-side on every query, feed,
//! recommendation, and notification pipeline -- a blocked tag never reaches the
//! client." §46.7.1 restates it as an invariant: never bypassable from a
//! surface.
//!
//! The implementation this replaces violated the spec in two ways, both
//! load-bearing.
//!
//! **It ran after `LIMIT`.** The base query paged first and the filter was then
//! applied in Rust, so a page of 20 that lost 15 works to a filter answered with
//! 5 rows instead of 20 clean ones: a reader who blocked a common tag saw a
//! short page and no indication that more matched.
//!
//! **It was a per-result query, and it did nothing on PostgreSQL.**
//! `work_tag_values` ran once per result -- 21 round trips for a 20-row page --
//! and its PostgreSQL arm used a bare `?`, which is not a bind parameter in
//! PostgreSQL at all. The server answered `syntax error at end of input`, the
//! caller swallowed it with `unwrap_or_default()`, and content filters filtered
//! *nothing* on PostgreSQL while passing on SQLite.
//!
//! The exclusion is now a correlated `NOT EXISTS` against the work's taxonomy
//! nodes, rendered inside the paged statement so it applies before the limit. It
//! is written here once, so a surface cannot forget a dialect and no surface can
//! opt out by accident.

use crate::settings::ContentFilterRow;

/// Aliases for the predicate's own subquery scope.
///
/// The recommendation engines already alias `work_tags` as `wt` in their outer
/// query. A nested scope may legally shadow it, but reading a statement that
/// uses the same name at two levels is how a refactor silently re-points the
/// correlation, so the exclusion uses names its callers never do.
const WORK_TAG_ALIAS: &str = "cf_wt";
const TAXONOMY_ALIAS: &str = "cf_tn";
use crate::{sql_owned, Database};
use std::collections::HashMap;

/// One blocked taxonomy value: the taxonomy node's `kind` and its `canonical`
/// form. Both are `TEXT` in both dialects, so neither bind needs a cast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterRule {
    pub filter_type: String,
    pub value: String,
}

impl FilterRule {
    pub fn new(filter_type: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            filter_type: filter_type.into(),
            value: value.into(),
        }
    }
}

impl From<ContentFilterRow> for FilterRule {
    fn from(row: ContentFilterRow) -> Self {
        Self::new(row.filter_type, row.value)
    }
}

/// A rendered exclusion: the predicate to splice into a `WHERE`, and the binds
/// it consumes, in order.
///
/// A caller places `predicate` in its statement and binds `binds` immediately
/// after whatever it has already bound. It never writes a placeholder for this
/// predicate itself, which is what keeps the two dialects' numbering from
/// drifting apart -- the one thing that made the previous version fragile.
#[derive(Debug, Clone, Default)]
pub struct Exclusion {
    /// Empty when there is nothing to exclude, so a caller can splice and bind
    /// unconditionally.
    pub predicate: String,
    /// `filter_type`, `value`, `filter_type`, `value`, ... in rule order.
    pub binds: Vec<String>,
}

impl Exclusion {
    /// Whether there is anything to exclude. A caller with no rules binds
    /// nothing and splices nothing.
    pub fn is_empty(&self) -> bool {
        self.predicate.is_empty()
    }

    /// The exclusion as a `WHERE` fragment, with the leading `AND` included, or
    /// the empty string when there is nothing to exclude.
    ///
    /// Splicing a bare `predicate` behind a literal `AND` breaks the statement
    /// the moment a reader has no filters -- which is the common case, and the
    /// one no positive test covers. Rendering the conjunction here means a caller
    /// writes `{filter_clause}` and cannot get it wrong.
    pub fn clause(&self) -> String {
        if self.is_empty() {
            String::new()
        } else {
            format!("AND {}", self.predicate)
        }
    }
}

/// Renders the exclusion predicate for a `works`-shaped statement.
///
/// The predicate correlates on `works.id`, so the statement must select
/// `FROM works` unaliased. Use [`predicate_for`] for anything else.
///
/// Written with `?`, not `$n`: hand-numbering a fragment that gets spliced into
/// a statement with its own placeholders is where the two dialects drift apart.
/// Pass the assembled statement to [`statement`] and `sql_owned` renumbers the
/// whole thing once, consistently.
///
/// Rules are OR-ed and each contributes one `(kind, canonical)` pair. A single
/// `IN` would be wrong: blocking the tag "slow burn" must not block the fandom
/// of the same name, so `kind` and `canonical` have to match as a pair.
pub fn predicate(rules: &[FilterRule]) -> String {
    predicate_for(rules, "works.id")
}

/// The PostgreSQL form of [`predicate`].
pub fn predicate_pg(rules: &[FilterRule]) -> String {
    predicate_for_pg(rules, "works.id")
}

/// [`predicate`] against a statement that names the work id some other way.
///
/// The site search selects `FROM works`, but the recommendation engines do not
/// all agree on a shape: two write `FROM works w`, and the media-reference engine
/// selects `work_media_references` and filters on `wmr.work_id`. Passing the
/// work-id *expression* rather than a table name covers all three.
///
/// `work_id_expr` is a full work-id *expression*, not a table name: the site
/// search passes `works.id`, the recommendation engines `w.id`, and the
/// media-reference engine `wmr.work_id`. Getting this wrong does not fail
/// loudly -- `works` alone correlates against a nonexistent column, which
/// PostgreSQL reports as `no such column: works` and SQLite tolerates, so the
/// same argument is right on one backend and broken on the other.
///
/// It must also be the uncast form: the PostgreSQL path appends `::text`, so
/// passing something already cast yields `wmr.work_id::text::text`.
pub fn predicate_for(rules: &[FilterRule], work_id_expr: &str) -> String {
    render(rules, work_id_expr)
}

/// [`predicate_pg`] against a statement that names the work id some other way.
pub fn predicate_for_pg(rules: &[FilterRule], work_id_expr: &str) -> String {
    render(rules, work_id_expr)
}

/// Renders the exclusion. One form, not two.
///
/// There was a `pg` flag here that added `::text` to the work correlation. It
/// was wrong -- see the comment on `correlation` -- and removing it means the
/// dialect-specific functions are now the same function. They are kept because
/// callers select on the backend and the asymmetry should not reappear silently
/// if the schema ever does need it.
fn render(rules: &[FilterRule], work_id_expr: &str) -> String {
    if rules.is_empty() {
        return String::new();
    }
    // One bind pair per rule. `kind` and `canonical` are matched together so
    // blocking the tag "slow burn" cannot block the fandom of the same name.
    let pair = format!(
        "({}.kind = ? AND {}.canonical = ?)",
        TAXONOMY_ALIAS, TAXONOMY_ALIAS
    );
    let ors = rules
        .iter()
        .map(|_| pair.as_str())
        .collect::<Vec<_>>()
        .join(" OR ");
    // The declared types (migrations/postgres/0070 + the taxonomy migrations):
    // `work_tags.work_id` is UUID, `work_tags.node_id` is TEXT, and
    // `taxonomy_nodes.id` is TEXT. So the node join is TEXT = TEXT and the work
    // correlation is UUID = UUID -- neither needs a cast in either dialect.
    //
    // An earlier version cast `cf_wt.work_id::text` on the assumption that
    // `work_tags.work_id` was TEXT. It is not: that made the comparison
    // `text = uuid` and every filtered search 500'd on PostgreSQL while passing
    // on SQLite. Read the migrations, do not infer a column's type from its
    // neighbours -- `work_tags` holds one of each.
    let correlation = format!("{}.work_id = {}", WORK_TAG_ALIAS, work_id_expr);
    format!(
        "NOT EXISTS (SELECT 1 FROM work_tags {wt} \
         JOIN taxonomy_nodes {tn} ON {tn}.id = {wt}.node_id \
         WHERE {correlation} AND ({ors}))",
        wt = WORK_TAG_ALIAS,
        tn = TAXONOMY_ALIAS,
    )
}

/// The binds a predicate consumes, in order.
pub fn binds(rules: &[FilterRule]) -> Vec<String> {
    rules
        .iter()
        .flat_map(|r| [r.filter_type.clone(), r.value.clone()])
        .collect()
}

/// The whole exclusion, ready to splice.
pub fn build(rules: &[FilterRule]) -> Exclusion {
    Exclusion {
        predicate: predicate(rules),
        binds: binds(rules),
    }
}

/// The whole exclusion in its PostgreSQL form, ready to splice.
pub fn build_pg(rules: &[FilterRule]) -> Exclusion {
    Exclusion {
        predicate: predicate_pg(rules),
        binds: binds(rules),
    }
}

/// The exclusion for a statement naming the work id as `work_id_expr`, spliced.
pub fn build_for(rules: &[FilterRule], work_id_expr: &str) -> Exclusion {
    Exclusion {
        predicate: predicate_for(rules, work_id_expr),
        binds: binds(rules),
    }
}

/// The exclusion for a non-default work id, in its PostgreSQL form.
pub fn build_for_pg(rules: &[FilterRule], work_id_expr: &str) -> Exclusion {
    Exclusion {
        predicate: predicate_for_pg(rules, work_id_expr),
        binds: binds(rules),
    }
}

/// Builds a statement pair and returns the live backend's form.
///
/// Both arguments are the same statement written twice. `sql_owned` renumbers
/// the PostgreSQL form's `?` to `$n`, so the caller writes one statement with
/// positional `?`, binds positionally, and never names a placeholder index.
pub fn statement(db: &Database, sqlite: String, pg: String) -> String {
    sql_owned(db, sqlite, pg)
}

/// Loads a viewer's filters by the pseud the session is acting as.
///
/// This is the form every surface should use. A content filter belongs to a
/// pseud, and which pseud is a *session* property -- `sessions.active_pseud_id`,
/// switchable with `sessions::set_active_pseud` -- not an account property. An
/// account can have several pseuds, each with its own filters.
///
/// So there is deliberately no account-keyed lookup that guesses a pseud: it
/// would pick one of the account's pseuds arbitrarily and apply that reader's
/// wrong filter set. A caller holding only an account must pass the session's
/// pseud, or pass `None` and apply nothing.
///
/// Returns an empty set for `None`, which is correct rather than a convenience:
/// an anonymous viewer has no filters.
pub async fn for_pseud(
    db: &Database,
    pseud_id: Option<uuid::Uuid>,
) -> crate::Result<Vec<FilterRule>> {
    let Some(pseud_id) = pseud_id else {
        return Ok(Vec::new());
    };
    let rows = crate::settings::list_content_filters(db, pseud_id).await?;
    Ok(rows.into_iter().map(FilterRule::from).collect())
}

/// Splice helper: the aliased exclusion for the live backend, ready to bind.
///
/// `#[macro]`-free on purpose -- a caller that gets this wrong should get a
/// compile error, not a runtime SQL error.
pub fn exclusion_for(db: &Database, rules: &[FilterRule], work_id_expr: &str) -> Exclusion {
    match db.backend() {
        crate::Backend::Postgres => build_for_pg(rules, work_id_expr),
        crate::Backend::Sqlite => build_for(rules, work_id_expr),
    }
}

/// [`for_pseud`] as a ready-to-splice exclusion, correlated on `works.id`.
pub async fn exclusion_for_viewer(
    db: &Database,
    pseud_id: Option<uuid::Uuid>,
) -> crate::Result<Exclusion> {
    let rules = for_pseud(db, pseud_id).await?;
    Ok(match db.backend() {
        crate::Backend::Postgres => build_pg(&rules),
        crate::Backend::Sqlite => build(&rules),
    })
}

/// Groups rules by type, for a surface reporting what is filtered -- a settings
/// page, or a "2 filters active" affordance on search.
pub fn by_type(rules: &[FilterRule]) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for rule in rules {
        out.entry(rule.filter_type.clone())
            .or_default()
            .push(rule.value.clone());
    }
    for values in out.values_mut() {
        values.sort();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(t: &str, v: &str) -> FilterRule {
        FilterRule::new(t, v)
    }

    #[test]
    fn no_rules_means_no_predicate_and_no_binds() {
        assert!(predicate(&[]).is_empty());
        assert!(predicate_pg(&[]).is_empty());
        assert!(build(&[]).is_empty());
        assert!(build_for(&[], "w").is_empty());
        assert!(binds(&[]).is_empty());
    }

    #[test]
    fn the_predicate_correlates_on_the_alias_it_was_given() {
        let rules = vec![rule("tag", "slow burn")];
        // Unaliased, as the site search writes it.
        assert!(predicate(&rules).contains("wt.work_id = works.id"));
        // Aliased, as every recommendation engine writes it.
        let aliased = predicate_for(&rules, "w");
        assert!(aliased.contains("wt.work_id = w.id"));
        assert!(!aliased.contains("works.id"));
    }

    #[test]
    fn the_pg_form_casts_both_sides_of_the_correlation() {
        let aliased = predicate_for_pg(&[rule("tag", "x")], "w");
        // `work_tags.work_id` is TEXT and `works.id` is UUID on PostgreSQL;
        // without both casts this is a type error, not a filter.
        assert!(aliased.contains("wt.work_id::text = w.id::text"));
    }

    #[test]
    fn kind_and_canonical_are_matched_as_a_pair() {
        // Blocking a tag must not block the fandom of the same name, so the
        // pair is compared inside one parenthesised conjunction.
        let p = predicate(&[rule("tag", "a"), rule("fandom", "b")]);
        assert_eq!(p.matches("(tn.kind = ? AND tn.canonical = ?)").count(), 2);
        assert!(p.contains(" OR "));
    }

    #[test]
    fn binds_are_filter_type_then_value_in_rule_order() {
        let rules = vec![rule("tag", "a"), rule("fandom", "b")];
        assert_eq!(binds(&rules), vec!["tag", "a", "fandom", "b"]);
    }

    #[test]
    fn both_dialects_bind_the_same_values_for_the_same_rules() {
        // The predicate is written with `?` and renumbered once, so the binds a
        // caller appends must be identical across dialects. A divergence here is
        // the silent failure mode: the filter applies, but to the wrong column.
        let rules = vec![rule("tag", "a"), rule("warning", "b")];
        let sqlite = build_for(&rules, "w");
        let pg = build_for_pg(&rules, "w");
        assert_eq!(sqlite.binds, pg.binds);
        assert_eq!(sqlite.binds, vec!["tag", "a", "warning", "b"]);
    }

    #[test]
    fn rules_are_grouped_and_sorted_for_display() {
        let rules = vec![rule("tag", "z"), rule("tag", "a"), rule("warning", "m")];
        let grouped = by_type(&rules);
        assert_eq!(
            grouped.get("tag").unwrap(),
            &vec!["a".to_string(), "z".to_string()]
        );
        assert_eq!(grouped.get("warning").unwrap(), &vec!["m".to_string()]);
    }
}
