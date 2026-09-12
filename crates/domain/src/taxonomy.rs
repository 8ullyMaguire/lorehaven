//! Structured taxonomy: nodes, aliases, normalization, merge policy.
//!
//! Spec §15.1–15.3. Pure functions — no I/O.

/// A taxonomy node kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// A fandom (e.g. "Harry Potter").
    Fandom,
    /// A ship / relationship.
    Ship,
    /// A character.
    Character,
    /// A general tag.
    Tag,
    /// A warning.
    Warning,
    /// A mood / tone.
    Mood,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fandom => "fandom",
            Self::Ship => "ship",
            Self::Character => "character",
            Self::Tag => "tag",
            Self::Warning => "warning",
            Self::Mood => "mood",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "fandom" => Self::Fandom,
            "ship" => Self::Ship,
            "character" => Self::Character,
            "tag" => Self::Tag,
            "warning" => Self::Warning,
            "mood" => Self::Mood,
            _ => return None,
        })
    }
}

/// The normalized lookup form of a tag: lowercase, trimmed.
///
/// This is what aliases resolve to and what nodes are indexed by.
pub fn canonical_form(text: &str) -> String {
    text.trim().to_lowercase()
}

/// Resolve an alias to the canonical form.
///
/// Returns `None` if the alias is empty after normalization.
pub fn resolve_alias(alias: &str) -> Option<String> {
    let norm = canonical_form(alias);
    if norm.is_empty() {
        None
    } else {
        Some(norm)
    }
}

/// Merge policy: two nodes merge behind one canonical id.
///
/// The survivor keeps its id and canonical; the absorbed node's aliases
/// redirect to the survivor.
#[derive(Debug, Clone)]
pub struct MergePlan {
    /// The node that survives.
    pub survivor_id: String,
    /// The node that is absorbed (will be deleted).
    pub absorbed_id: String,
    /// Aliases that should be re-pointed at the survivor.
    pub redirect_aliases: Vec<String>,
}

/// Compute a merge plan for two nodes.
///
/// The survivor is the node with the lower id (lexicographic), so the
/// result is independent of argument order.
pub fn merge_plan(a_id: &str, b_id: &str, a_aliases: &[String], b_aliases: &[String]) -> MergePlan {
    let (survivor_id, absorbed_id) = if a_id <= b_id {
        (a_id.to_owned(), b_id.to_owned())
    } else {
        (b_id.to_owned(), a_id.to_owned())
    };

    let mut redirect_aliases = Vec::new();
    for alias in a_aliases.iter().chain(b_aliases.iter()) {
        let norm = canonical_form(alias);
        if !norm.is_empty() {
            redirect_aliases.push(norm);
        }
    }
    redirect_aliases.sort();
    redirect_aliases.dedup();

    MergePlan {
        survivor_id,
        absorbed_id,
        redirect_aliases,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_form_lowercases_and_trims() {
        assert_eq!(canonical_form("  Harry Potter  "), "harry potter");
        assert_eq!(canonical_form("FANDOM"), "fandom");
        assert_eq!(canonical_form(""), "");
    }

    #[test]
    fn resolve_alias_rejects_empty() {
        assert_eq!(resolve_alias("   "), None);
        assert_eq!(resolve_alias("tag"), Some("tag".to_owned()));
    }

    #[test]
    fn node_kind_round_trips() {
        for kind in [
            NodeKind::Fandom,
            NodeKind::Ship,
            NodeKind::Character,
            NodeKind::Tag,
            NodeKind::Warning,
            NodeKind::Mood,
        ] {
            assert_eq!(NodeKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(NodeKind::parse("unknown"), None);
    }

    #[test]
    fn merge_plan_picks_lower_id_as_survivor() {
        let plan = merge_plan("b", "a", &["alias1".to_owned()], &["alias2".to_owned()]);
        assert_eq!(plan.survivor_id, "a");
        assert_eq!(plan.absorbed_id, "b");
        assert_eq!(plan.redirect_aliases, vec!["alias1", "alias2"]);
    }

    #[test]
    fn merge_plan_is_symmetric() {
        let plan_ab = merge_plan("a", "b", &["x".to_owned()], &["y".to_owned()]);
        let plan_ba = merge_plan("b", "a", &["y".to_owned()], &["x".to_owned()]);
        assert_eq!(plan_ab.survivor_id, plan_ba.survivor_id);
        assert_eq!(plan_ab.redirect_aliases, plan_ba.redirect_aliases);
    }
}
