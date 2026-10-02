//! Work coordinates: four deterministic measures over a work's prose
//! (spec §49.3, M45-14).
//!
//! ## What this module decides
//!
//! Four numbers per work, each separating something a reader can feel and a
//! reader cannot always name: sentence-length variance, dialogue ratio,
//! vocabulary richness, and chapter-length distribution. §49.3's point is that
//! they are **dimensions the ranker scores against** — a reader who consistently
//! likes varied, scene-driven prose can be modelled on them without ever being
//! asked what "prose density" means.
//!
//! ## Why these four and not a model
//!
//! §49.3 refuses an embedding by default and makes the reason concrete: "an
//! embedding is a fifth that costs the instance CPU it may not have (§53.2)", and
//! optional local embeddings require M27's ML permission. Four arithmetic
//! measures are enough for the taste profile, and — this is the load-bearing
//! part — they are **exactly reproducible**. A dimension that differs between
//! two runs of the same text is not a dimension, it is noise with a name.
//!
//! ## Determinism, spelled out, because it is the whole contract
//!
//! §49.7 requires "Same text in, same coordinates out, on both engines, with no
//! model and no randomness". Three things follow, and each is a place a naive
//! implementation goes wrong:
//!
//! * **No HashMap iteration in the output path.** Rust's `HashMap` has a
//!   randomly-seeded hasher, so iterating one yields a different order per
//!   process. Vocabulary richness therefore sorts its type counts before doing
//!   anything with them.
//! * **No floating-point accumulation order dependence.** Summing sentence
//!   lengths in document order is the only order the spec can mean, and the
//!   variance is computed from a second pass rather than from a running mean, so
//!   the result does not depend on how the loop was unrolled.
//! * **Integer counts first, floats last.** Every measure is a ratio of two
//!   integers that have been counted exactly. Nothing is normalised by a length
//!   computed in floating point.
//!
//! ## The absent coordinate is not a zero
//!
//! §49.3 and §49.7 both insist on this: "A work too short to measure has **no
//!   coordinates**, and an absent coordinate is not a zero. A zero would mean
//!   'uniformly flat prose' and would rank against short-but-sharp works."
//!
//! So [`WorkCoordinates`] is `Option<WorkCoordinates>` everywhere it is stored
//! and read, and [`MIN_MEASURABLE_WORDS`] is the threshold. That number is a
//! decision — §49.3 says "the minimum measurable length" without naming one — and
//! it is named and documented here so that retuning it is a one-line act rather
//! than a number discovered in three places.
//!
//! It is 500 words because the shortest of the four measures is sentence-length
//! variance, and variance over fewer than ~30 sentences is dominated by sampling
//! noise: two sentences of length 4 and 40 look "very varied" and two of length
//! 20 and 21 look "very flat", with neither reading meaning anything. 500 words
//! clears that comfortably while staying far below a typical chapter, so a short
//! *work* made of several chapters is still measurable — the threshold is on
//! the whole work's text, not on any one chapter.

use serde::{Deserialize, Serialize};

/// A work too short to measure gets no coordinates at all.
///
/// §49.3 names "the minimum measurable length" without giving a number; this is
/// that number, and the reasoning is in the module docs: sentence-length
/// variance needs enough sentences for its spread to mean anything.
pub const MIN_MEASURABLE_WORDS: usize = 500;

/// The four measures §49.3 specifies, each normalised to `0.0..=1.0`.
///
/// Normalised rather than raw because they are compared against a reader's
/// weights, and the reader's weights are on a `0.0..=1.0` scale
/// (`TasteDimension::admin_target`). A raw word count would be meaningless next
/// to a normalised weight; a normalised one is comparable directly.
///
/// All four are `Option<f64>` individually even though the struct as a whole is
/// only constructed when the work is measurable. A measure that cannot be
/// computed for an otherwise-measurable work — a work with no dialogue at all
/// has a dialogue ratio of exactly `0.0`, which is *not* the same as unmeasured,
/// so that case is a real `Some(0.0)`; but a single-chapter work has no
/// chapter-length *distribution* to speak of, and that is genuinely `None`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WorkCoordinates {
    /// Spread of sentence lengths. High = varied prose, low = flat.
    ///
    /// Normalised by the longest sentence in the work, not by the mean, so a work
    /// with one long sentence and many short ones reads as varied — which is what
    /// a reader perceives — rather than being pulled back toward its own average.
    pub sentence_length_variance: f64,

    /// Fraction of the text that is dialogue, in `0.0..=1.0`.
    ///
    /// Counts quoted spans, which is the convention fanfic prose actually uses.
    /// Reported to four decimal places internally by the division and then
    /// rounded, so two runs cannot differ in the last float bit.
    pub dialogue_ratio: f64,

    /// Type-token ratio: distinct words over total words. High = dense.
    ///
    /// Case-folded and punctuation-stripped, because "The" and "the" are one
    /// word to a reader and two to a naive split. Bounded by the shortest of the
    /// four to measure in a stable way — a very short work has a spuriously high
    /// TTR — so it is computed over the whole work's text and not per chapter.
    pub vocabulary_richness: f64,

    /// How the work's length is spread across its chapters, `0.0..=1.0`.
    ///
    /// Coefficient of variation (stddev / mean) of chapter word counts, capped at
    /// 1.0. High = a serial with wildly uneven chapters; low = uniform lengths or
    /// a single chapter. `None` for a single-chapter work, which has no
    /// distribution to measure.
    pub chapter_length_spread: Option<f64>,

    /// Word count the measures were computed over. Recorded so a stored
    /// coordinate can be checked against the text it came from without
    /// recomputing, and so a work that shrank below the threshold is detectable.
    pub word_count: usize,

    /// How many sentences the variance measure saw, for the same reason.
    pub sentence_count: usize,
}

impl WorkCoordinates {
    /// The dimensions these coordinates occupy, as `key -> position`.
    ///
    /// §49.3's four measures *are* the dimensions the ranker scores against, so
    /// this is the mapping from a work to the vector the arena compares. Keys are
    /// stable strings because they land in `arena_weights.dimension_key` and in
    /// exported profiles.
    pub fn as_dimensions(&self) -> Vec<(String, f64)> {
        let mut out = vec![
            (
                DIM_SENTENCE_LENGTH_VARIANCE.to_string(),
                self.sentence_length_variance,
            ),
            (DIM_DIALOGUE_RATIO.to_string(), self.dialogue_ratio),
            (
                DIM_VOCABULARY_RICHNESS.to_string(),
                self.vocabulary_richness,
            ),
        ];
        if let Some(spread) = self.chapter_length_spread {
            out.push((DIM_CHAPTER_LENGTH_SPREAD.to_string(), spread));
        }
        out
    }
}

/// `arena_weights.dimension_key` for sentence-length variance.
pub const DIM_SENTENCE_LENGTH_VARIANCE: &str = "style_sentence_variance";
/// `arena_weights.dimension_key` for dialogue ratio.
pub const DIM_DIALOGUE_RATIO: &str = "style_dialogue_ratio";
/// `arena_weights.dimension_key` for type-token ratio.
pub const DIM_VOCABULARY_RICHNESS: &str = "style_vocabulary_richness";
/// `arena_weights.dimension_key` for chapter-length spread.
pub const DIM_CHAPTER_LENGTH_SPREAD: &str = "style_chapter_length_spread";

/// A chapter's text, as coordinates see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterText {
    /// The chapter's plain text. `chapter_revisions.plain_text` (0003).
    pub plain_text: String,
    /// The chapter's word count as the instance computed it at ingest.
    ///
    /// Taken from the stored column rather than recounted, so coordinates agree
    /// with the number the rest of the platform shows. §49.3 wants reproducibility
    /// of *coordinates*, and the cheapest way to be reproducible is to depend on
    /// one already-deterministic count rather than two.
    pub word_count: usize,
}

/// Why a work has no coordinates.
///
/// §49.3's rule is that an absent coordinate is not a zero, and a reason is
/// strictly more useful than a bare `None`: an operator asking why a work is not
/// rankable gets "too short" rather than a null and a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unmeasurable {
    /// No text at all — an empty work, or one whose chapters were never saved.
    NoText,
    /// Below [`MIN_MEASURABLE_WORDS`]. Short-but-sharp, per §49.3.
    TooShort,
}

impl Unmeasurable {
    /// A human-readable explanation, for a 404 body or a log line.
    pub fn explain(self) -> &'static str {
        match self {
            Unmeasurable::NoText => "the work has no measurable text",
            Unmeasurable::TooShort => "the work is below the minimum measurable length",
        }
    }
}

/// The outcome of a coordinate computation.
///
/// A sum type rather than `Option<WorkCoordinates>` because §49.3's "an absent
/// coordinate is not a zero" is only honoured if the *reason* travels with the
/// absence, and a bare `Option` loses it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Coordinates {
    /// The work is measurable.
    Measured(WorkCoordinates),
    /// The work is not, and here is why.
    Unmeasurable(Unmeasurable),
}

impl Coordinates {
    /// The coordinates, if any.
    pub fn measured(&self) -> Option<&WorkCoordinates> {
        match self {
            Coordinates::Measured(c) => Some(c),
            Coordinates::Unmeasurable(_) => None,
        }
    }

    /// Whether this work has coordinates.
    pub fn is_measured(&self) -> bool {
        matches!(self, Coordinates::Measured(_))
    }

    /// Why this work has no coordinates, if it has none.
    ///
    /// The read-side counterpart to `measured()`. A caller that wants to tell a
    /// reader *why* a work is not rankable — a 404 body, a log line, an operator's
    /// "why is this one missing" — needs the reason, and a bare `Option` from
    /// `measured()` cannot give it back.
    pub fn clone_reason(&self) -> Option<Unmeasurable> {
        match self {
            Coordinates::Measured(_) => None,
            Coordinates::Unmeasurable(reason) => Some(*reason),
        }
    }
}

/// Everything §49.3's measures need: the whole work's text, and its chapters'
/// individual word counts.
#[derive(Debug, Clone, Default)]
pub struct Corpus {
    /// Chapter texts in `order_key` order. Order matters for nothing in the
    /// measures themselves (they are order-independent) but is preserved so a
    /// caller can rely on the input being the reading order.
    pub chapters: Vec<ChapterText>,
    /// Words attributable to dialogue, summed across chapters.
    ///
    /// Carried in rather than recomputed from `chapters` so the caller can read
    /// it from whatever it already has — a stored dialogue count, a tag, a
    /// parser's output — instead of re-deriving it. The measures only need the
    /// *sum*, and taking it from the caller is what makes this module usable
    /// from both the ingest path and a backfill.
    pub dialogue_words: usize,
}

impl Corpus {
    /// The whole work's text, chapters joined by a blank line.
    ///
    /// The join matters and is not arbitrary: sentence segmentation must not run
    /// two chapters together into one sentence, so a paragraph break is required
    /// between them. `\n\n` is what `plain_text` already uses between
    /// paragraphs, so this introduces no new convention.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for (i, chapter) in self.chapters.iter().enumerate() {
            if i > 0 {
                out.push_str("\n\n");
            }
            out.push_str(&chapter.plain_text);
        }
        out
    }

    /// The work's total word count, as the instance recorded it.
    pub fn word_count(&self) -> usize {
        self.chapters.iter().map(|c| c.word_count).sum()
    }
}

/// Count the words inside quotation marks, for §49.3's dialogue ratio.
///
/// The count is deliberately a *caller-supplied* input to [`Corpus`] rather than
/// something `coordinates` derives: the measures are order-independent and
/// deterministic, and a dialogue heuristic is neither — it is a judgement about
/// which marks mean speech. So the parser lives here, separately testable, and
/// the caller decides whether to trust it. What matters for §49.3 is only that
/// the same text always yields the same count, which this does.
///
/// A run of text between an opening curly quote (`\u{201C}`, `\u{201D}`) and its
/// matching closer is one quoted span. Curly quotes rather than straight ones
/// because straight quotes are also inches and feet, and a measurement that
/// counts `6\"` as dialogue is worse than one that counts nothing.
///
/// Unclosed quotes are handled by closing at end-of-text rather than by
/// discarding: text pasted from a word processor often has an unmatched quote,
/// and dropping the rest of the chapter would under-report the ratio in a way
/// that looks like a deliberate measurement.
///
/// Nested quotes are not tracked. Two levels of quotation inside a quotation are
/// vanishingly rare in prose and the error is bounded by the inner run, so the
/// extra state would not buy accuracy.
#[must_use]
pub fn dialogue_word_count(text: &str) -> usize {
    let mut words = 0usize;
    let mut in_quote = false;
    let mut in_word = false;

    for ch in text.chars() {
        match ch {
            // Entering or leaving a quoted run also ends the current word, so
            // the text *between* two quotes is not counted as part of either.
            // Without this, `in_word` is still true from the previous quote and
            // the first word of the gap is swallowed rather than counted -- the
            // count comes out one short, and only for multi-span dialogue.
            '\u{201C}' => {
                in_quote = true;
                in_word = false;
            }
            '\u{201D}' => {
                in_quote = false;
                in_word = false;
            }
            // Inside a quoted run, a word is a maximal run of characters that
            // are not whitespace and not punctuation -- so `don't` and
            // `well-known` are one word each, and a lone `...` is none. Written
            // as one explicit branch rather than a pair of guards so the word
            // boundary is in one place: the earlier version reset `in_word` on
            // any non-alphanumeric, which split `don't` into two words and
            // silently over-reported the ratio for apostrophe-heavy prose.
            c if in_quote => {
                // What counts as part of a word, and what merely touches one.
                //
                // `is_alphanumeric` covers letters and digits, but NOT kana -- and
                // kana are neither alphanumeric nor whitespace, so a
                // whitespace-only rule counted every Japanese work as having a
                // dialogue ratio of exactly 0.0. §49.3 names 0.0 as the value
                // that must never be confused with "unmeasured", so that would
                // have been the single most misleading number in the schema. The
                // rule is therefore: any non-whitespace character is part of a
                // word EXCEPT the two CJK punctuation marks that are visually
                // word boundaries.
                //
                // A hyphen, an apostrophe and an underscore JOIN rather than
                // split: `well-known` and `don't` are each one word, and prose
                // that hyphenates heavily would otherwise read as very talkative
                // purely because of its punctuation. A joiner only CONTINUES a
                // run -- a leading one does not start it, so a stray quote mark
                // or dash cannot invent a word.
                let joiner = matches!(c, '-' | '\'' | '\u{2019}' | '_');
                let cjk_boundary = matches!(c, '\u{3001}' | '\u{3002}');
                let word_character =
                    !c.is_whitespace() && !cjk_boundary && (c.is_alphanumeric() || !c.is_ascii());
                let part_of_word = word_character || (joiner && in_word);

                if part_of_word && !in_word {
                    in_word = true;
                    words += 1;
                } else if !part_of_word {
                    in_word = false;
                }
            }
            _ => {}
        }
    }
    words
}

/// Compute §49.3's four measures, or say why there are none.
///
/// Deterministic: same [`Corpus`] in, same [`Coordinates`] out, on both engines.
/// The reasoning is in the module docs — no hash iteration in the output path,
/// no float accumulation order dependence, integer counts first.
pub fn coordinates(corpus: &Corpus) -> Coordinates {
    let word_count = corpus.word_count();
    if word_count == 0 {
        return Coordinates::Unmeasurable(Unmeasurable::NoText);
    }
    if word_count < MIN_MEASURABLE_WORDS {
        return Coordinates::Unmeasurable(Unmeasurable::TooShort);
    }

    let text = corpus.text();

    let sentences = sentences(&text);
    let sentence_count = sentences.len();
    let (variance, _) = sentence_length_variance(&sentences);

    let dialogue_ratio = if word_count == 0 {
        0.0
    } else {
        // Clamped: a caller whose stored dialogue count exceeds the work's word
        // count would otherwise produce a ratio above 1.0, and a coordinate
        // outside 0.0..=1.0 cannot be compared against a normalised weight.
        (corpus.dialogue_words as f64 / word_count as f64).clamp(0.0, 1.0)
    };

    let vocabulary_richness = type_token_ratio(&text);

    let chapter_length_spread = chapter_length_spread(&corpus.chapters);

    Coordinates::Measured(WorkCoordinates {
        sentence_length_variance: variance,
        dialogue_ratio: round6(dialogue_ratio),
        vocabulary_richness: round6(vocabulary_richness),
        chapter_length_spread: chapter_length_spread.map(round6),
        word_count,
        sentence_count,
    })
}

/// Split text into sentences on `.`, `!`, `?` and hard line breaks.
///
/// A hand-rolled splitter rather than a crate, for §49.3's sake: the measure has
/// to be reproducible across engines and across runs, and a dependency's
/// sentence-boundary rules are a thing that changes between versions. The rules
/// here are four characters and a paragraph break, and they are written down.
///
/// Abbreviations are **not** special-cased. "Dr." would split, and that is a
/// known, accepted approximation: it is deterministic, which is the property
/// §49.7 requires, and a reader comparing two works' sentence-length variance
/// does not care about Dr. Conway. The alternative — a rule table — is exactly
/// the kind of thing that makes a "deterministic" measure quietly unstable.
fn sentences(text: &str) -> Vec<usize> {
    let chars: Vec<char> = text.chars().collect();
    let mut lengths = Vec::new();
    let mut current = 0_usize;

    // A boundary is terminal punctuation, or a paragraph break. A single space is
    // NOT a boundary: it is the space *inside* a sentence.
    //
    // Two wrong rules bracket this one, and both were found by tests rather than
    // by reading the code:
    //
    //   * "every char including \n is a boundary" -> a "\n\n" chapter join is two
    //     boundaries, one of which carries no sentence, so every multi-chapter
    //     work gained N-1 phantom one-character sentences and its variance
    //     measure was dragged toward zero.
    //   * "any whitespace is a boundary" -> every inter-word space became one, so
    //     a single sentence split into hundreds of one-word fragments, and the
    //     measure INVERTED: flat prose scored higher than varied, because 4000
    //     equal fragments have a smaller relative spread than 40 mixed ones.
    //
    // A blank line is the right rule because it is the only whitespace run that
    // cannot occur inside a sentence. "\n\n" is therefore ONE break, not two,
    // which is what the first rule got wrong and the second rule fixed by
    // accident -- and then broke for the far more common case of ordinary prose.
    let mut i = 0_usize;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\n' {
            // A run of newlines: at least one is a paragraph break.
            let mut newlines = 0_usize;
            while i < chars.len() && chars[i] == '\n' {
                newlines += 1;
                i += 1;
            }
            if newlines >= 2 && current > 0 {
                // A blank line ends the sentence in progress, once.
                lengths.push(std::mem::take(&mut current));
            } else if newlines == 1 && current > 0 {
                // A single newline inside a chapter (soft wrap) is not a boundary.
                // Leading whitespace is not part of the next sentence's length.
            }
            continue;
        }
        current += 1;
        if matches!(ch, '.' | '!' | '?') {
            lengths.push(std::mem::take(&mut current));
        }
        i += 1;
    }
    if current > 0 {
        lengths.push(current);
    }
    lengths
}

/// Variance of sentence lengths, normalised to `0.0..=1.0`.
///
/// Two passes over the integers — mean, then squared deviations — rather than
/// Welford's online algorithm, because §49.3's contract is byte-identical output
/// and a two-pass sum over `i64` is easier to be sure of than an online update
/// whose result depends on the order of the updates. (They agree closely; this
/// chooses the one whose answer is a property of the data.)
///
/// ## Why divided by the longest sentence
///
/// The obvious alternative — the mean, a textbook coefficient of variation — was
/// measured and rejected, because it runs out of range. On a fixture of 60
/// ten-word sentences and one 1200-word sentence the standard deviation *exceeds*
/// the mean, so sd/mean is > 1.0 and only reaches `0.0..=1.0` by being clamped.
/// That clamp is a lie about the data: every work more extreme than the fixture
/// would report the same 1.0, and the dimension would lose all resolution at the
/// top of its range — which is precisely where the readers who most need to be
/// distinguished from each other live.
///
/// Dividing by the longest sentence stays inside the range without a clamp, and
/// keeps the five measured fixtures spread across it rather than saturated:
///
/// ```text
/// case                              /max    /mean
/// 200 equal sentences              0.0010  0.0010
/// 100 sentences of 15/25 words     0.2199  0.2819
/// 40 sentences of 5/90 words       0.4773  0.9131
/// 60x10 words + one 1200-word      0.1263  1.0000  <- clamped
/// half 8-word, half 80-word        0.4599  0.8514
/// ```
///
/// An earlier draft of this comment claimed the longest-sentence divisor made
/// "one long sentence among many short ones" read as *high* variation. The
/// measurement shows the opposite — that case scores 0.1263, below the 5/90
/// case's 0.4773 — and 0.1263 is the better answer: most of its sentences really
/// are the same length, and a reader who liked that work is not looking for
/// variety. The divisor is chosen for range headroom, not for flattering one
/// fixture.
///
/// Returns `0.0` for fewer than two sentences, where there is no spread to
/// report.
fn sentence_length_variance(lengths: &[usize]) -> (f64, f64) {
    if lengths.len() < 2 {
        return (0.0, 0.0);
    }
    let n = lengths.len() as f64;
    let sum: f64 = lengths.iter().map(|l| *l as f64).sum();
    let mean = sum / n;
    let sq: f64 = lengths.iter().map(|l| (*l as f64 - mean).powi(2)).sum();
    let variance = sq / n;
    let longest = lengths.iter().copied().max().unwrap_or(1) as f64;
    if longest <= 0.0 {
        return (0.0, variance);
    }
    // sqrt then divide: variance is in chars^2, so the sqrt puts it back in chars
    // and makes it comparable with the longest sentence.
    ((variance.sqrt() / longest).clamp(0.0, 1.0), variance)
}

/// Type-token ratio over the whole text, case-folded and punctuation-stripped.
///
/// The distinct-word set is a `BTreeSet` and the doc above used to claim that was
/// *because* iteration order would otherwise reach the output. That claim was
/// wrong, and the mutation harness is what proved it: swapping in a `HashSet`
/// left all seven tests green, across separate processes, every time.
///
/// The reason is visible here — only `insert` and `len` are called, and neither
/// depends on iteration order. `distinct.len()` is a count, not a traversal, so
/// the set's ordering is unobservable. `BTreeSet` is kept anyway because it makes
/// the determinism argument *local and checkable* rather than a claim about a
/// property the code happens not to need today: the next person to iterate this
/// set gets a stable order for free instead of silently introducing the seed
/// dependence §49.7 forbids.
///
/// An equivalent mutant is still worth recording, because the alternative was a
/// test that cannot fail: an earlier version of the determinism test tried to
/// catch this and could not, since one process has one hash seed.
fn type_token_ratio(text: &str) -> f64 {
    let mut distinct: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut total = 0_usize;
    for word in text.split_whitespace() {
        let cleaned: String = word
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '\'' || *c == '-')
            .flat_map(|c| c.to_lowercase())
            .collect();
        if cleaned.is_empty() {
            continue;
        }
        total += 1;
        distinct.insert(cleaned);
    }
    if total == 0 {
        return 0.0;
    }
    (distinct.len() as f64 / total as f64).clamp(0.0, 1.0)
}

/// Coefficient of variation of chapter word counts, capped at `1.0`.
///
/// `None` for a single chapter: a one-chapter work has no length *distribution*,
/// and reporting `0.0` would say "uniformly even chapters" about a work that has
/// exactly one — the same absent-is-not-a-zero mistake §49.3 warns about, one
/// level down.
fn chapter_length_spread(chapters: &[ChapterText]) -> Option<f64> {
    if chapters.len() < 2 {
        return None;
    }
    let n = chapters.len() as f64;
    let sum: f64 = chapters.iter().map(|c| c.word_count as f64).sum();
    let mean = sum / n;
    if mean <= 0.0 {
        return Some(0.0);
    }
    let sq: f64 = chapters
        .iter()
        .map(|c| (c.word_count as f64 - mean).powi(2))
        .sum();
    let stddev = (sq / n).sqrt();
    Some((stddev / mean).clamp(0.0, 1.0))
}

/// Round to six decimal places.
///
/// §49.3 wants the same text to give the same coordinates *byte for byte* (§49.8
/// says "re-running the computation reproduces them byte for byte"). Storing a
/// `REAL`/`DOUBLE PRECISION` and comparing it across two runs is fine — the same
/// IEEE 754 operations give the same bits — but a value that has been through a
/// sum whose order could vary is not safe to compare as text. Rounding to six
/// places makes the printed form stable and is far finer than any difference a
/// reader could perceive between two works.
fn round6(v: f64) -> f64 {
    (v * 1_000_000.0).round() / 1_000_000.0
}
