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

/// Minimum similarity ratio (0–1) for two strings to be considered a fuzzy
/// match in taxonomy autocomplete.
///
/// The score is `1 - (levenshtein / max_len)`. A floor of 0.7 means
/// strings must share at least 70% of their characters in the best
/// alignment — e.g. "harry poter" still matches "harry potter" but
/// "xyz" does not match "harry potter".
pub const FUZZY_SIMILARITY_FLOOR: f64 = 0.7;

/// Compute the Levenshtein edit distance between two byte strings.
///
/// The distance is the minimum number of single-character edits
/// (insertions, deletions, substitutions) required to change one
/// string into the other.
pub fn levenshtein(a: &[u8], b: &[u8]) -> usize {
    let (la, lb) = (a.len(), b.len());
    if la == 0 {
        return lb;
    }
    if lb == 0 {
        return la;
    }
    // Use two rolling rows to keep O(min(la,lb)) space.
    let (long, short) = if la >= lb { (a, b) } else { (b, a) };
    let (long_len, short_len) = (long.len(), short.len());
    let mut prev: Vec<usize> = (0..=short_len).collect();
    let mut curr: Vec<usize> = vec![0; short_len + 1];
    for i in 1..=long_len {
        curr[0] = i;
        for j in 1..=short_len {
            let cost = if long[i - 1] == short[j - 1] { 0 } else { 1 };
            curr[j] = std::cmp::min(
                std::cmp::min(curr[j - 1] + 1, prev[j] + 1),
                prev[j - 1] + cost,
            );
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[short_len]
}

/// Similarity ratio between two strings, in the range `0.0`–`1.0`.
///
/// Computed as `1 - (levenshtein / max(len_a, len_b))`. When both
/// strings are empty the similarity is `1.0`.
pub fn similarity(a: &str, b: &str) -> f64 {
    let la = a.chars().count();
    let lb = b.chars().count();
    if la == 0 && lb == 0 {
        return 1.0;
    }
    let max_len = std::cmp::max(la, lb) as f64;
    if max_len == 0.0 {
        return 1.0;
    }
    let dist = levenshtein(a.as_bytes(), b.as_bytes());
    1.0 - (dist as f64 / max_len)
}

/// A fuzzy match result: a node's id, canonical name, and similarity score.
#[derive(Debug, Clone, PartialEq)]
pub struct FuzzyMatch {
    pub id: String,
    pub canonical: String,
    pub score: f64,
}

/// Find taxonomy nodes whose normalized name is similar to `query`,
/// subject to `FUZZY_SIMILARITY_FLOOR`.
///
/// Returns up to `limit` results, sorted by descending similarity then
/// by ascending canonical name. Exact-prefix matches from the caller
/// are expected to be merged separately; this function fills gaps.
pub fn fuzzy_match(
    query: &str,
    candidates: impl IntoIterator<Item = (String, String)>,
    limit: usize,
) -> Vec<FuzzyMatch> {
    let mut scored: Vec<FuzzyMatch> = candidates
        .into_iter()
        .map(|(id, norm)| {
            let score = similarity(query, &norm);
            FuzzyMatch {
                id,
                canonical: norm,
                score,
            }
        })
        .filter(|m| m.score >= FUZZY_SIMILARITY_FLOOR)
        .collect();
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.canonical.cmp(&b.canonical))
    });
    scored.truncate(limit);
    scored
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

    #[test]
    fn levenshtein_empty_string() {
        assert_eq!(levenshtein(b"", b""), 0);
        assert_eq!(levenshtein(b"a", b""), 1);
        assert_eq!(levenshtein(b"", b"abc"), 3);
    }

    #[test]
    fn levenshtein_identical() {
        assert_eq!(levenshtein(b"abc", b"abc"), 0);
    }

    #[test]
    fn levenshtein_single_edit() {
        assert_eq!(levenshtein(b"abc", b"ab"), 1); // deletion
        assert_eq!(levenshtein(b"abc", b"abcd"), 1); // insertion
        assert_eq!(levenshtein(b"abc", b"axc"), 1); // substitution
    }

    #[test]
    fn levenshtein_known_distances() {
        assert_eq!(levenshtein(b"kitten", b"sitting"), 3);
        assert_eq!(levenshtein(b"harry poter", b"harry potter"), 1);
    }

    #[test]
    fn similarity_full_and_zero() {
        assert_eq!(similarity("", ""), 1.0);
        assert_eq!(similarity("abc", "abc"), 1.0);
        assert_eq!(similarity("abc", "xyz"), 0.0);
    }

    #[test]
    fn similarity_partial() {
        // "harry poter" vs "harry potter": 1 edit out of 12 chars
        let s = similarity("harry poter", "harry potter");
        assert!((s - (1.0 - 1.0 / 12.0)).abs() < 1e-9);
        // "harry poter" vs "harry potter" should exceed the floor
        assert!(s >= FUZZY_SIMILARITY_FLOOR);
    }

    #[test]
    fn fuzzy_match_finds_typo() {
        let candidates = vec![
            ("1".to_owned(), "lord of the rings".to_owned()),
            ("2".to_owned(), "harry potter".to_owned()),
        ];
        let results = fuzzy_match("harry poter", candidates, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "2");
        assert!(results[0].score >= FUZZY_SIMILARITY_FLOOR);
    }

    #[test]
    fn fuzzy_match_respects_limit() {
        let candidates = vec![
            ("1".to_owned(), "harry potter".to_owned()),
            ("2".to_owned(), "harry poter".to_owned()),
        ];
        let results = fuzzy_match("harry poter", candidates, 1);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn fuzzy_match_filters_below_floor() {
        let candidates = vec![("1".to_owned(), "xyz".to_owned())];
        let results = fuzzy_match("harry potter", candidates, 10);
        assert!(results.is_empty());
    }
}
