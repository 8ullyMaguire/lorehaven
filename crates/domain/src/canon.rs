//! Canon-agnostic classification (spec §50.2, M45-31).
//!
//! ## What §50.2 asks for, and what it forbids
//!
//! §50.2: "A work whose fandom is unknown to the reader is eligible... Canon-
//! agnostic works are a class, not a score. A work is eligible when its own text
//! is readable without having read a parent canon."
//!
//! And §50.3 forbids the obvious implementation twice over:
//!
//! * "**No automatic canon detection.** §50.2's class is computed from the text
//!   by the deterministic measures §49.3 already defines, not from a franchise
//!   database that would go stale and would need to be corrected by hand."
//! * "**No cross-reader pooling of what 'canon-agnostic' means.**" §49.9 refuses
//!   cross-reader taste pooling, and a per-work flag computed from one instance's
//!   corpus is that reader's corpus, not the platform's.
//!
//! So: no franchise list, no embedding, no model, no cross-reader anything. What
//! is left is a property of the words themselves, which is the one signal that
//! actually answers the question being asked.
//!
//! ## The signal: unexplained proper nouns
//!
//! A reader meets "She handed the Data Chip to Spock" and stalls, because the
//! word carries a referent the text has not established. That is exactly what
//! "readable without having read a parent canon" means, felt. So the measure is
//! the share of tokens that are *unexplained proper nouns*: capitalised, not at a
//! sentence start, not a word the text itself has already introduced in lowercase.
//!
//! The three exclusions are each load-bearing:
//!
//! * **Sentence-initial capitals** are English orthography, not information.
//!   Counting them would make every work score as canon-dependent.
//! * **A name the text has already used in lowercase** is established by the
//!   text. `spock` appears after `Spock`, so the reader learned it here.
//! * **A name the text establishes in lowercase** stops counting. So does a name
//!   that never appears mid-sentence, which is the same rule as the first bullet.
//!
//! The direction of the error is deliberate: an unexplained name counts even on
//! its first appearance, because §50.2 asks whether THIS text can be read cold.
//! That errs toward canon-agnostic — never toward hiding a work that turns out to
//! need its parent.
//!
//! ## Why a threshold and not a score
//!
//! §50.2 says "a class, not a score". A score invites a ranker to blend it, and
//! §50.3's last invariant says canon-blind discovery "changes eligibility, never
//! ranking". So [`CANON_DENSITY_CEILING`] is a boolean boundary and nothing reads
//! the fraction for ordering.
//!
//! The ceiling is 2%. It is a decision, documented here so retuning it is a
//! one-line act: at 2%, a text with 1000 words may carry 20 unexplained proper
//! nouns, which is about one every fifty words — sparse enough to be incidental
//! naming rather than a reader stalling every paragraph.

use std::collections::HashSet;

/// Share of tokens that may be unexplained proper nouns before a work is treated
/// as canon-dependent. See the module docs for why 2%.
pub const CANON_DENSITY_CEILING: f64 = 0.02;

/// Below this many words there is not enough signal to classify, and §50.2's
/// class is then *absent* rather than either answer — the same rule §49.3 states
/// for coordinates: an absent value is not a zero.
pub const MIN_CANON_WORDS: usize = 500;

/// What §50.2's class came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonClass {
    /// Readable without its parent canon: eligible for fandom-blind discovery.
    CanonAgnostic,
    /// Carries unexplained proper nouns a reader may stall on.
    CanonDependent,
}

/// The measurement behind a [`CanonClass`], so a stored flag can be audited.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanonMeasures {
    /// Unexplained proper nouns found.
    pub unexplained_names: usize,
    /// Tokens the density was taken over.
    pub word_count: usize,
    /// `unexplained_names / word_count`.
    pub density: f64,
}

/// Classify a work's text, or say there is too little of it to tell.
///
/// Deterministic in the §50.3 sense: same text, same answer, on both engines.
/// The only state is a `HashMap` used for membership, never iterated — so no hash
/// seed can reach the result, which is the same rule §49.3's module follows for
/// the vocabulary measure.
#[must_use]
pub fn classify(text: &str, word_count: usize) -> Option<(CanonClass, CanonMeasures)> {
    if word_count < MIN_CANON_WORDS {
        return None;
    }

    // Lowercase forms the text has established, so a name the reader learns here
    // never counts as unexplained twice. Membership only — the map is never
    // iterated, so no hash seed can reach the result. That is the same rule
    // §49.3's vocabulary measure follows, and it is what makes the answer
    // reproducible across runs and engines.
    let mut established: HashSet<String> = HashSet::new();
    let mut unexplained = 0usize;

    for (token, sentence_initial) in tokenize(text) {
        let lower = token.to_lowercase();

        if is_proper_noun(&token) && !sentence_initial {
            if established.insert(lower.clone()) {
                unexplained += 1;
            }
        } else {
            established.insert(lower);
        }
    }

    // No zero-guard needed: the threshold above already returned for any
    // `word_count` below `MIN_CANON_WORDS`, so the divisor cannot be zero here.
    let density = unexplained as f64 / word_count as f64;
    let class = if density <= CANON_DENSITY_CEILING {
        CanonClass::CanonAgnostic
    } else {
        CanonClass::CanonDependent
    };
    Some((
        class,
        CanonMeasures {
            unexplained_names: unexplained,
            word_count,
            density,
        },
    ))
}

/// Split into `(token, sentence_initial)` pairs.
///
/// The flag is decided HERE, where the punctuation is still in hand: a capital
/// at the start of a sentence is English orthography rather than information, and
/// recovering "was this sentence-initial?" after the fact needs a second scan over
/// the source. One pass, and the classifier below holds no state at all.
///
/// Apostrophes stay inside a word, so `don't` stays one token. A terminator is
/// `.`, `!`, `?` or a newline — the same set §49.3's sentence splitter uses, so
/// the two measures agree about where sentences are.
fn tokenize(text: &str) -> Vec<(String, bool)> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    // True when the previous character ended a sentence, so the NEXT token is a
    // sentence-initial one.
    let mut next_is_initial = true;

    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '\'' {
            current.push(ch);
            continue;
        }
        if !current.is_empty() {
            tokens.push((std::mem::take(&mut current), next_is_initial));
            next_is_initial = false;
        }
        if matches!(ch, '.' | '!' | '?' | '\n') {
            next_is_initial = true;
        }
    }
    if !current.is_empty() {
        tokens.push((current, next_is_initial));
    }
    tokens
}

/// A token is a proper noun if it starts with an uppercase letter and is not
/// entirely uppercase.
///
/// Entirely-uppercase tokens are excluded because they are far more often
/// shouted emphasis ("NO", "STOP") or an initialism ("FBI") than a name, and
/// §50.2 wants the words a reader *stalls* on — an initialism stalls only for
/// readers who already know it.
fn is_proper_noun(token: &str) -> bool {
    let Some(first) = token.chars().next() else {
        return false;
    };
    if !first.is_uppercase() {
        return false;
    }
    token.chars().filter(|c| c.is_uppercase()).count() != token.chars().count()
}
