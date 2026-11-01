//! "Similar to this one" over the only fields every catalogue row has (M63-04).
//!
//! # Why this is not a tag comparison
//!
//! The obvious similarity for a fanfiction catalogue is tag overlap, and on this
//! catalogue it does not work. Measured 2026-11-01: `work_tags` joins **none** of
//! the 1,247 rows that carry an endorsement count — every one of the 7,626 tagged
//! rows is `pawchive` with zero likes — so a tag-based neighbour list for any row
//! a reader can rank is empty. What is left is the text: title, summary and the
//! author's name, which every source publishes for every row.
//!
//! # Why idf-weighted cosine rather than overlap
//!
//! Raw overlap is dominated by the words this catalogue is made of. "Gamer",
//! "system", "litrpg" and "cyoa" appear in hundreds of summaries; two unrelated
//! LitRPGs therefore share most of their vocabulary and score 0.9 against each
//! other, which is not a ranking. Weighting each token by inverse document
//! frequency over the candidate population — a token in every document is worth
//! almost nothing, a rare shared token is worth most — is what turns "shares the
//! word gamer" into "shares the word Christchurch".
//!
//! The title is weighted three times the summary because a shared title token is
//! a much stronger claim than a shared summary word: "Worm" in the title names the
//! setting, "worm" in a two-hundred-word summary may be an aside about a worm.
//!
//! # What this module deliberately does not do
//!
//! It takes no database, no clock and no configuration. The population is handed
//! in, and the same input always produces the same order — the store decides which
//! rows are candidates and the route decides how many of them to show.

use std::collections::{BTreeSet, HashMap};

/// How much a title token counts against a summary token.
const TITLE_WEIGHT: f64 = 3.0;
/// How much a summary token counts.
const BODY_WEIGHT: f64 = 1.0;
/// What sharing an author adds to the cosine.
const AUTHOR_BONUS: f64 = 0.15;
/// Tokens shorter than this are articles and prepositions after the stopword pass
/// has already taken the real ones; four characters is where fandom vocabulary
/// starts ("worm", "cyoa", "gamer").
const MIN_TOKEN: usize = 3;
/// A document cannot contribute more than this many distinct tokens. Summaries on
/// the boards run to 1,500 characters and a small number of rows are pasted
/// threadmark indexes; the cap keeps one such row from dominating the idf.
const MAX_TOKENS: usize = 64;

/// Words that carry no topic. Kept short on purpose: every entry here is a token
/// that can no longer distinguish two rows, and a fandom word in this list would
/// silently disable the only positive signal the module has.
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "have", "has", "had", "his",
    "her", "hers", "him", "she", "they", "them", "their", "there", "here", "who", "whom", "what",
    "when", "where", "which", "while", "will", "would", "was", "were", "are", "is", "been",
    "being", "but", "not", "all", "any", "can", "could", "did", "does", "doing", "each", "one",
    "two", "you", "your", "our", "ours", "its", "it's", "out", "off", "own", "too", "very", "just",
    "like", "more", "most", "some", "such", "than", "then", "now", "new", "after", "before",
    "over", "under", "again", "also", "because", "how", "why", "yes", "about", "against",
    "between", "during", "through", "yet", "chapter", "story", "fic", "fics", "thread", "update",
    "updated", "author", "summary", "work", "works", "read", "reading", "written", "first", "last",
    "this", "self", "insert", "si",
];

/// Split a title or summary into comparable tokens.
///
/// Lowercases, splits on anything that is not alphanumeric, drops tokens shorter
/// than [`MIN_TOKEN`], drops [`STOPWORDS`], and deduplicates. Deduplication is
/// what makes the vector a *set* comparison: a summary that says "gamer" eleven
/// times is not eleven times more about gamers than one that says it once.
#[must_use]
pub fn tokens(text: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        let token = raw.trim().to_lowercase();
        if token.len() < MIN_TOKEN || STOPWORDS.contains(&token.as_str()) {
            continue;
        }
        out.insert(token);
        if out.len() >= MAX_TOKENS {
            break;
        }
    }
    out.into_iter().collect()
}

/// One row, as the similarity function sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seed {
    /// The catalogue row's id, so a hit can be looked back up.
    pub id: String,
    /// Distinct title tokens.
    pub title_tokens: Vec<String>,
    /// Distinct summary tokens.
    pub body_tokens: Vec<String>,
    /// The author, lowercased and trimmed, for the bonus comparison.
    pub author: String,
}

impl Seed {
    /// Build a seed from the fields every catalogue row carries.
    #[must_use]
    pub fn of(id: impl Into<String>, title: &str, summary: &str, author: &str) -> Self {
        Self {
            id: id.into(),
            title_tokens: tokens(title),
            body_tokens: tokens(summary),
            author: author.trim().to_lowercase(),
        }
    }

    /// Every token this row carries, with its field weight.
    fn weighted(&self) -> Vec<(&str, f64)> {
        let mut out: Vec<(&str, f64)> = Vec::with_capacity(self.title_tokens.len() + self.body_tokens.len());
        for token in &self.title_tokens {
            out.push((token.as_str(), TITLE_WEIGHT));
        }
        for token in &self.body_tokens {
            // A token that is already in the title keeps the title's weight: the
            // title is where the claim about the story is made.
            if self.title_tokens.contains(token) {
                continue;
            }
            out.push((token.as_str(), BODY_WEIGHT));
        }
        out
    }
}

/// Inverse document frequency over the candidate population.
///
/// `weight = ln((n + 1) / (df + 1)) + 1`, which floors at 1.0 for a token in
/// every document and grows slowly for rare ones. The `+1` outside the logarithm
/// is what keeps a ubiquitous token contributing something to the shape of the
/// comparison instead of collapsing the whole vector to zero.
#[derive(Debug, Clone, Default)]
pub struct Idf {
    df: HashMap<String, u32>,
    n: u32,
}

impl Idf {
    /// Count how many of `population` carry each token.
    #[must_use]
    pub fn build<'a>(population: impl IntoIterator<Item = &'a Seed>) -> Self {
        let mut df: HashMap<String, u32> = HashMap::new();
        let mut n = 0u32;
        for seed in population {
            n += 1;
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            for (token, _) in seed.weighted() {
                seen.insert(token);
            }
            for token in seen {
                *df.entry(token.to_owned()).or_insert(0) += 1;
            }
        }
        Self { df, n }
    }

    /// The weight of one token. A token no document carries is worth its own
    /// weight of 1.0 rather than infinity: it cannot be shared, so its exact value
    /// never reaches a score.
    #[must_use]
    pub fn weight(&self, token: &str) -> f64 {
        let df = self.df.get(token).copied().unwrap_or(0);
        let ratio = (f64::from(self.n) + 1.0) / (f64::from(df) + 1.0);
        ratio.ln() + 1.0
    }

    /// How many documents the statistics were built from.
    #[must_use]
    pub fn documents(&self) -> u32 {
        self.n
    }
}

/// One neighbour: how similar, and the tokens that make it so.
#[derive(Debug, Clone, PartialEq)]
pub struct SimHit {
    /// The catalogue row's id.
    pub id: String,
    /// 0..=1, rounded to four places so the order is reproducible.
    pub score: f64,
    /// Shared tokens, most informative first — the `--explain` line.
    pub shared: Vec<String>,
}

/// Cosine similarity between two seeds, plus the tokens that carried it.
///
/// Returns `(0.0, [])` when nothing is shared: an unshared row is not "slightly
/// similar", it is unrelated, and the gate counts exactly those.
#[must_use]
pub fn similarity(seed: &Seed, other: &Seed, idf: &Idf) -> (f64, Vec<String>) {
    let (left, right) = (seed.weighted(), other.weighted());
    let right_index: HashMap<&str, f64> = right.iter().copied().collect();

    let mut dot = 0.0;
    let mut left_norm = 0.0;
    let mut right_norm = 0.0;
    let mut shared: Vec<(String, f64)> = Vec::new();

    for (token, weight) in &left {
        let w = idf.weight(token);
        let contribution = weight * w;
        left_norm += contribution * contribution;
        if let Some(other_weight) = right_index.get(token) {
            let shared_contribution = contribution * other_weight * w;
            dot += shared_contribution;
            shared.push(((*token).to_owned(), shared_contribution));
        }
    }
    for (token, weight) in &right {
        let w = idf.weight(token);
        let contribution = weight * w;
        right_norm += contribution * contribution;
    }
    if left_norm <= 0.0 || right_norm <= 0.0 || dot <= 0.0 {
        return (0.0, Vec::new());
    }

    let mut score = dot / (left_norm.sqrt() * right_norm.sqrt());
    if !seed.author.is_empty() && seed.author == other.author {
        score = (score + AUTHOR_BONUS).min(1.0);
    }
    shared.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    shared.truncate(8);
    (
        // Four places: the same ids in the same order on every run, which is what
        // lets a saved page be compared with a fresh one.
        (score * 10_000.0).round() / 10_000.0,
        shared.into_iter().map(|(token, _)| token).collect(),
    )
}

/// Rank `population` against `seed`, best first.
///
/// Rows with no shared token are dropped, not ranked at the bottom: `ranked N of
/// M` has to mean "N rows are actually similar", and a page padded with unrelated
/// rows is the failure this returns to avoid. `ties` are broken by id so paging is
/// stable.
#[must_use]
pub fn rank(seed: &Seed, population: &[Seed], idf: &Idf) -> Vec<SimHit> {
    let mut hits: Vec<SimHit> = population
        .iter()
        .filter(|other| other.id != seed.id)
        .filter_map(|other| {
            let (score, shared) = similarity(seed, other, idf);
            (score > 0.0).then_some(SimHit {
                id: other.id.clone(),
                score,
                shared,
            })
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(id: &str, title: &str, summary: &str, author: &str) -> Seed {
        Seed::of(id, title, summary, author)
    }

    #[test]
    fn tokens_drop_stopwords_short_words_and_repetition() {
        let got = tokens("The Gamer of the Gamer System, a fic");
        assert_eq!(got, vec!["gamer", "system"]);
    }

    #[test]
    fn a_shared_title_token_outweighs_a_shared_summary_token() {
        // Same population, two candidates: one shares the title's rare word, one
        // shares only a summary word. The title one must win, which is what the
        // 3.0 weight buys.
        let target = seed("a", "Christchurch Protocol", "a worm story", "ann");
        let title_match = seed("b", "Christchurch Rising", "nothing else", "bob");
        let body_match = seed("c", "Something Else", "a worm story", "carl");
        let population = vec![target.clone(), title_match.clone(), body_match.clone()];
        let idf = Idf::build(&population);
        let (title_score, _) = similarity(&target, &title_match, &idf);
        let (body_score, _) = similarity(&target, &body_match, &idf);
        assert!(
            title_score > body_score,
            "title match {title_score} must beat summary match {body_score}"
        );
    }

    #[test]
    fn the_same_author_breaks_a_similarity_tie() {
        // Two candidates with identical text, one by the seed's author (in a
        // different case, because a board spells the pseud however it likes). The
        // seed carries one extra token so neither candidate sits at 1.0 — a
        // clamped score would hide the bonus.
        let target = seed(
            "a",
            "Ghost in the City",
            "cyberpunk gamer neon grid runner implant",
            "ZFighter18",
        );
        let same_author = seed("b", "Ghost in the City", "cyberpunk gamer", "zfighter18");
        let other_author = seed("c", "Ghost in the City", "cyberpunk gamer", "someone");
        let population = vec![target.clone(), same_author.clone(), other_author.clone()];
        let idf = Idf::build(&population);
        let (a, _) = similarity(&target, &same_author, &idf);
        let (b, _) = similarity(&target, &other_author, &idf);
        assert!(a > b, "same author {a} must beat other author {b}");
        assert!(
            (a - b - AUTHOR_BONUS).abs() < 1e-4,
            "the bonus is exactly {AUTHOR_BONUS}, got {}",
            a - b
        );
    }

    #[test]
    fn a_ubiquitous_token_cannot_outweigh_a_rare_one() {
        // "gamer" in every document, "christchurch" in two: raw overlap would rate
        // the gamer-only rows as unrelated-but-close; idf must collapse that word.
        let target = seed("a", "Christchurch Gamer", "a gamer", "ann");
        let generic = seed("b", "Another Gamer", "just a gamer", "bob");
        let rare = seed("c", "Christchurch Gamer", "a gamer", "carl");
        let filler: Vec<Seed> = (0..40)
            .map(|i| seed(&format!("f{i}"), &format!("Gamer {i}"), "a gamer system", "x"))
            .collect();
        let mut population = vec![target.clone(), generic.clone(), rare.clone()];
        population.extend(filler);
        let idf = Idf::build(&population);
        let (rare_score, _) = similarity(&target, &rare, &idf);
        let (generic_score, _) = similarity(&target, &generic, &idf);
        assert!(
            rare_score > generic_score,
            "rare shared token {rare_score} must beat ubiquitous {generic_score}"
        );
    }

    #[test]
    fn nothing_shared_is_zero_and_the_row_is_dropped() {
        let target = seed("a", "Christchurch", "harbour fog", "ann");
        let unrelated = seed("b", "Battletech", "mechwarrior", "bob");
        let population = vec![target.clone(), unrelated];
        let idf = Idf::build(&population);
        let (score, shared) = similarity(&target, &population[1], &idf);
        assert_eq!(score, 0.0);
        assert!(shared.is_empty());
        assert!(rank(&target, &population, &idf).is_empty());
    }

    #[test]
    fn the_seed_never_ranks_itself_and_the_order_is_stable() {
        let target = seed("a", "Ghost in the City", "cyberpunk gamer", "ann");
        let near = seed("b", "Ghost in the City", "cyberpunk gamer", "bob");
        let far = seed("c", "Ghost of a Chance", "cyberpunk", "carl");
        let population = vec![target.clone(), near.clone(), far.clone()];
        let idf = Idf::build(&population);
        let hits = rank(&target, &population, &idf);
        assert!(hits.iter().all(|h| h.id != "a"), "the seed must not rank itself");
        assert_eq!(hits[0].id, "b");
        // Determinism: the same call twice.
        assert_eq!(
            hits.iter().map(|h| (h.id.clone(), h.score)).collect::<Vec<_>>(),
            rank(&target, &population, &idf)
                .iter()
                .map(|h| (h.id.clone(), h.score))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_explanation_names_the_token_that_carried_the_score() {
        let target = seed("a", "Worm Gamer", "a story", "ann");
        let other = seed("b", "Worm Gamer", "another", "bob");
        let population = vec![target.clone(), other.clone()];
        let idf = Idf::build(&population);
        let (_, shared) = similarity(&target, &other, &idf);
        assert_eq!(shared, vec!["gamer", "worm"]);
    }
}
