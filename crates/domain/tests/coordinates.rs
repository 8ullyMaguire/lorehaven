//! Coordinates: §49.3's four deterministic measures, and §49.7's invariants.
//!
//! | clause | test |
//! |---|---|
//! | same text in, same coordinates out, no model, no randomness | `the_same_text_gives_the_same_coordinates_every_time` |
//! | four works of identical tags, different prose, differ | `prose_style_separates_works_that_tags_cannot` |
//! | a work under the minimum measurable length has no coordinates | `a_work_under_the_minimum_length_has_no_coordinates` |
//! | an absent coordinate is not a zero | `an_absent_coordinate_is_not_a_zero` |
//! | four measures, each separating what it claims | `each_measure_separates_what_it_claims_to` |
//! | coordinates are dimensions the ranker scores against | `coordinates_are_rankable_dimensions` |
//!
//! The determinism test is the load-bearing one. §49.8 asks for "byte for byte"
//! reproducibility, and the cheapest way to be sure a measure has no hidden
//! ordering dependency is to compute it many times over and require equality —
//! which is what the first test does, including a serialised comparison because
//! §49.8's wording is about bytes rather than about approximate numbers.

use lorehaven_domain::coordinates::{
    coordinates, ChapterText, Coordinates, Corpus, Unmeasurable, WorkCoordinates,
    DIM_CHAPTER_LENGTH_SPREAD, DIM_DIALOGUE_RATIO, DIM_SENTENCE_LENGTH_VARIANCE,
    DIM_VOCABULARY_RICHNESS, MIN_MEASURABLE_WORDS,
};

/// The measured coordinates for a corpus, as an owned value.
///
/// A helper rather than `coordinates(&c).measured().unwrap()` at every call site
/// because that pattern borrows a temporary and fails to compile, and the failure
/// is a wall of lifetime diagnostics rather than anything to do with coordinates.
/// It is the same fix for the same mistake, so it is written once.
fn coords_of(corpus: &Corpus) -> WorkCoordinates {
    let result = coordinates(corpus);
    *result.measured().expect("the fixture must be measurable")
}

/// A sentence of `n` distinct words, so a fixture's length is arithmetic rather
/// than something to count by hand.
fn sentence(words: usize) -> String {
    (0..words)
        .map(|i| format!("w{i}"))
        .collect::<Vec<_>>()
        .join(" ")
        + "."
}

/// A chapter of `sentences` sentences, each `words` long, with `words` words in
/// total — so the stored `word_count` and the real text agree, which is what a
/// caller at ingest would hand over.
fn chapter(sentences: usize, words: usize) -> ChapterText {
    let text = (0..sentences)
        .map(|_| sentence(words))
        .collect::<Vec<_>>()
        .join(" ");
    ChapterText {
        word_count: sentences * words,
        plain_text: text,
    }
}

/// A measurable multi-chapter work. The chapter mix varies a little so the text
/// is not one string repeated — a corpus of identical chapters has degenerate
/// measures and would let a bug pass.
fn work_of(chapters: usize, words_each: usize, dialogue: usize) -> Corpus {
    Corpus {
        chapters: (0..chapters)
            .map(|c| ChapterText {
                word_count: words_each,
                // `format!("marker{c}")` inside the args of another `format!` is
                // just `c`: the inner call exists only to turn an integer into text,
                // and the outer one already does that for every argument it is given.
                // The marker is not cosmetic, though -- it is what keeps two chapters
                // from being byte-identical, which is the degenerate corpus this
                // fixture exists to avoid.
                plain_text: format!("{} marker{c}", chapter(2, words_each / 4).plain_text),
            })
            .collect(),
        dialogue_words: dialogue,
    }
}

#[test]
fn the_same_text_gives_the_same_coordinates_every_time() {
    let corpus = work_of(4, 800, 120);
    let first = coordinates(&corpus);
    assert!(first.is_measured(), "{first:?}");

    // Recomputed many times. If any measure depended on a HashMap's iteration
    // order, a float accumulation, or a clock, this is where it would show.
    for i in 0..64 {
        assert_eq!(
            coordinates(&corpus),
            first,
            "iteration {i} disagreed with the first computation — §49.7 requires \\
             same text in, same coordinates out"
        );
    }

    // A structurally-equal corpus built independently is the case that catches
    // an accidental dependence on `HashMap` seeding: two processes have different
    // hash seeds, so equal inputs in two builds must still agree.
    let twin = Corpus {
        chapters: corpus.chapters.clone(),
        dialogue_words: corpus.dialogue_words,
    };
    assert_eq!(
        coordinates(&twin),
        first,
        "a fresh but equal corpus disagreed"
    );

    // Byte-for-byte, as §49.8 words it: the serialised form must be identical,
    // not merely numerically close.
    let a = serde_json::to_string(first.measured().unwrap()).expect("serialise");
    let b = serde_json::to_string(coordinates(&corpus).measured().unwrap()).expect("serialise");
    assert_eq!(a, b, "coordinates must reproduce byte for byte");
}

#[test]
fn prose_style_separates_works_that_tags_cannot() {
    // §49.8: "Four works of identical tags and different prose styles produce
    // different coordinates." Identical tags are the premise, so only the prose
    // varies here.
    let flat = Corpus {
        chapters: vec![chapter(200, 20)],
        dialogue_words: 0,
    };
    let varied = Corpus {
        chapters: vec![ChapterText {
            word_count: 1900,
            plain_text: (0..40)
                .map(|i| sentence(if i % 2 == 0 { 5 } else { 90 }))
                .collect::<Vec<_>>()
                .join(" "),
        }],
        dialogue_words: 0,
    };

    assert!(
        coordinates(&flat).is_measured() && coordinates(&varied).is_measured(),
        "both fixtures must clear the minimum measurable length"
    );
    let a = coords_of(&flat);
    let b = coords_of(&varied);

    assert!(
        b.sentence_length_variance > a.sentence_length_variance,
        "varied prose must score higher on sentence-length variance: {} vs {}",
        b.sentence_length_variance,
        a.sentence_length_variance
    );
    assert!(
        a.vocabulary_richness != b.vocabulary_richness,
        "two texts with different prose must not have identical vocabulary richness"
    );
}

#[test]
fn a_work_under_the_minimum_length_has_no_coordinates() {
    // §49.8: "A work under the minimum measurable length has no coordinates and
    // does not rank against measured works."
    let short = Corpus {
        chapters: vec![chapter(5, 20)],
        dialogue_words: 0,
    };
    assert!(
        short.word_count() < MIN_MEASURABLE_WORDS,
        "the fixture must actually be under the threshold: {} words",
        short.word_count()
    );
    let result = coordinates(&short);
    assert_eq!(
        result,
        Coordinates::Unmeasurable(Unmeasurable::TooShort),
        "a work below the minimum must be unmeasurable, not measured as zero"
    );
    assert!(!result.is_measured());

    // A work of exactly the threshold IS measurable, so the boundary is where it
    // says it is rather than one word inside it.
    let at_threshold = Corpus {
        chapters: vec![chapter(MIN_MEASURABLE_WORDS / 20, 20)],
        dialogue_words: 0,
    };
    assert_eq!(at_threshold.word_count(), MIN_MEASURABLE_WORDS);
    assert!(
        coordinates(&at_threshold).is_measured(),
        "a work of exactly MIN_MEASURABLE_WORDS must be measurable"
    );
}

#[test]
fn an_absent_coordinate_is_not_a_zero() {
    // §49.3: "A zero would mean 'uniformly flat prose' and would rank against
    // short-but-sharp works." So the two must be distinguishable, and the reason
    // must travel with the absence.
    let empty = Corpus::default();
    let short = Corpus {
        chapters: vec![chapter(3, 20)],
        dialogue_words: 0,
    };

    assert_eq!(
        coordinates(&empty),
        Coordinates::Unmeasurable(Unmeasurable::NoText)
    );
    assert_eq!(
        coordinates(&short),
        Coordinates::Unmeasurable(Unmeasurable::TooShort)
    );
    assert!(coordinates(&empty).measured().is_none());
    assert!(coordinates(&short).measured().is_none());

    // The reason is recoverable, which a bare `Option` could not do.
    assert_eq!(
        coordinates(&short).clone_reason(),
        Some(Unmeasurable::TooShort)
    );
    assert_eq!(
        coordinates(&empty).clone_reason(),
        Some(Unmeasurable::NoText)
    );
    assert!(coordinates(&work_of(3, 800, 0)).clone_reason().is_none());
    assert!(!Unmeasurable::TooShort.explain().is_empty());

    // A measured work with no dialogue at all is Some(0.0), which is a real
    // measurement — "no dialogue" is a fact about the prose, not an absence.
    let no_dialogue = work_of(3, 800, 0);
    assert_eq!(
        coordinates(&no_dialogue)
            .measured()
            .expect("measured")
            .dialogue_ratio,
        0.0
    );
}

#[test]
fn each_measure_separates_what_it_claims_to() {
    // Dialogue ratio: quoted words against none.
    let mut with = work_of(2, 800, 0);
    with.dialogue_words = 800;
    let without = work_of(2, 800, 0);
    let with = coords_of(&with);
    let without = coords_of(&without);
    assert_eq!(without.dialogue_ratio, 0.0, "no dialogue is exactly zero");
    assert!(
        with.dialogue_ratio > without.dialogue_ratio,
        "a work with dialogue must score higher: {} vs {}",
        with.dialogue_ratio,
        without.dialogue_ratio
    );
    assert!(
        (0.0..=1.0).contains(&with.dialogue_ratio),
        "a ratio must be normalised: {}",
        with.dialogue_ratio
    );

    // Vocabulary richness: one word repeated, against all words distinct.
    let repetitive = Corpus {
        chapters: vec![ChapterText {
            word_count: 500,
            plain_text: (0..500).map(|_| "same").collect::<Vec<_>>().join(" "),
        }],
        dialogue_words: 0,
    };
    let all_distinct = Corpus {
        chapters: vec![ChapterText {
            word_count: 500,
            plain_text: (0..500)
                .map(|i| format!("word{i}"))
                .collect::<Vec<_>>()
                .join(" "),
        }],
        dialogue_words: 0,
    };
    let rep = coords_of(&repetitive);
    let dis = coords_of(&all_distinct);
    assert_eq!(
        rep.vocabulary_richness, 0.002,
        "one distinct word in 500 is a type-token ratio of 1/500"
    );
    assert!(
        dis.vocabulary_richness > rep.vocabulary_richness,
        "distinct words must score higher than one repeated word: {} vs {}",
        dis.vocabulary_richness,
        rep.vocabulary_richness
    );
    // Case-folding, because "The" and "the" are one word to a reader.
    let cased = Corpus {
        chapters: vec![ChapterText {
            word_count: 500,
            plain_text: (0..500)
                .map(|i| if i % 2 == 0 { "The" } else { "the" })
                .collect::<Vec<_>>()
                .join(" "),
        }],
        dialogue_words: 0,
    };
    assert_eq!(
        coordinates(&cased)
            .measured()
            .expect("measured")
            .vocabulary_richness,
        0.002,
        "case must not inflate the type count"
    );

    // Chapter-length spread: even chapters against uneven ones, and a
    // single-chapter work has no distribution at all.
    let even = Corpus {
        chapters: vec![
            chapter(40, 20),
            chapter(40, 20),
            chapter(40, 20),
            chapter(40, 20),
        ],
        dialogue_words: 0,
    };
    let uneven = Corpus {
        chapters: vec![
            chapter(40, 20),
            chapter(40, 20),
            chapter(4, 20),
            chapter(5, 20),
        ],
        dialogue_words: 0,
    };
    let e = coords_of(&even);
    let u = coords_of(&uneven);
    assert_eq!(
        e.chapter_length_spread,
        Some(0.0),
        "equal chapters have no spread"
    );
    assert!(
        u.chapter_length_spread.unwrap() > 0.0,
        "uneven chapters must have spread"
    );

    // A single-chapter work: no distribution, so None — not 0.0, which would say
    // "evenly sized chapters" about a work that has exactly one. The same
    // absent-is-not-a-zero mistake, one level down.
    let single = Corpus {
        chapters: vec![chapter(60, 20)],
        dialogue_words: 0,
    };
    assert_eq!(
        coords_of(&single).chapter_length_spread,
        None,
        "one chapter has no distribution to measure; absent is not zero"
    );
}

#[test]
fn coordinates_are_rankable_dimensions() {
    // §49.3's measures are "dimensions the ranker scores against", so they have
    // to arrive as dimension keys the arena already understands.
    let five = coords_of(&work_of(5, 800, 100));
    let dims = five.as_dimensions();
    let keys: Vec<&str> = dims.iter().map(|(k, _)| k.as_str()).collect();

    assert!(keys.contains(&DIM_SENTENCE_LENGTH_VARIANCE), "{keys:?}");
    assert!(keys.contains(&DIM_DIALOGUE_RATIO), "{keys:?}");
    assert!(keys.contains(&DIM_VOCABULARY_RICHNESS), "{keys:?}");
    assert!(keys.contains(&DIM_CHAPTER_LENGTH_SPREAD), "{keys:?}");

    // Every coordinate is a position on the same 0.0..=1.0 scale a reader's
    // weights live on, so they can be compared without rescaling.
    for (key, value) in &dims {
        assert!(
            (0.0..=1.0).contains(value),
            "{key} is {value}, outside the scale the weights use"
        );
    }

    // A single-chapter work contributes three dimensions, not four — the missing
    // one is genuinely missing rather than reported as zero.
    let single_corpus = Corpus {
        chapters: vec![chapter(60, 20)],
        dialogue_words: 0,
    };
    let three = coords_of(&single_corpus).as_dimensions();
    assert_eq!(three.len(), 3, "{three:?}");
    assert!(
        !three.iter().any(|(k, _)| k == DIM_CHAPTER_LENGTH_SPREAD),
        "an unmeasurable dimension must not appear as a weight target"
    );
}

#[test]
fn a_dialogue_count_larger_than_the_work_is_clamped() {
    // A caller whose stored dialogue count disagrees with the word count must not
    // produce a coordinate outside the scale. This is a data-integrity guard, and
    // it is here because "a ratio above 1.0 cannot be compared against a
    // normalised weight" is a silent corruption otherwise.
    let over = coords_of(&work_of(3, 800, 9999));
    assert_eq!(
        over.dialogue_ratio, 1.0,
        "an over-count must clamp to 1.0, not exceed the scale"
    );
}

#[test]
fn sentence_variance_stays_inside_its_range_without_clamping() {
    // The divisor is the longest sentence, not the mean, and the mutation harness
    // is why: swapping in the mean left the suite green, because the earlier
    // flat-vs-varied test only asserted that two works differ, and both
    // normalisations make them differ.
    //
    // The real defect is at the TOP of the range. A textbook coefficient of
    // variation (sd / mean) exceeds 1.0 whenever the spread is wider than the
    // average sentence -- so it only lands in 0.0..=1.0 by being clamped, and
    // every work more extreme than the clamp collapses onto the same 1.0. The
    // readers who most need to be told apart are exactly the ones at that end.
    //
    // So the assertions are about resolution across the range, not about one
    // fixture looking big. `clamp` is the thing under test: these fixtures are
    // chosen so a clamped implementation reports a visibly wrong number.
    let two_scale = Corpus {
        chapters: vec![ChapterText {
            word_count: 2000,
            // Half very short, half long: a real spread with a real ceiling.
            plain_text: (0..60)
                .map(|i| sentence(if i < 30 { 8 } else { 80 }))
                .collect::<Vec<_>>()
                .join(" "),
        }],
        dialogue_words: 0,
    };
    let mild = Corpus {
        chapters: vec![ChapterText {
            word_count: 2000,
            plain_text: (0..100)
                .map(|i| sentence(if i % 2 == 0 { 15 } else { 25 }))
                .collect::<Vec<_>>()
                .join(" "),
        }],
        dialogue_words: 0,
    };
    let flat = Corpus {
        chapters: vec![ChapterText {
            word_count: 4000,
            plain_text: (0..200).map(|_| sentence(20)).collect::<Vec<_>>().join(" "),
        }],
        dialogue_words: 0,
    };

    let t = coords_of(&two_scale).sentence_length_variance;
    let m = coords_of(&mild).sentence_length_variance;
    let f = coords_of(&flat).sentence_length_variance;

    // Ordering holds, and it is the ordering a reader would describe.
    assert!(f < m, "flat {} must score below mild {}", f, m);
    assert!(m < t, "mild {} must score below two-scale {}", m, t);

    // And the top of the range is not pinned. Under sd/mean the two-scale
    // fixture computes to > 1.0 and clamps to exactly 1.0; under the
    // longest-sentence divisor it lands well inside. The gap between the two
    // normalisations is what this number records.
    assert!(
        t < 0.75,
        "a two-scale work must not saturate the measure; got {t} — a sd/mean \
         implementation clamps here to 1.0 and loses all resolution above it"
    );
    assert!(
        t > 0.3,
        "a two-scale work is clearly varied and must not read as flat; got {t}"
    );

    // The fixture must really be the shape the test claims, or the assertion
    // above is vacuous.
    let t_sentences = coords_of(&two_scale).sentence_count;
    assert_eq!(t_sentences, 60, "the fixture must contain 60 sentences");
    let f_sentences = coords_of(&flat).sentence_count;
    assert_eq!(
        f_sentences, 200,
        "the flat fixture must contain 200 sentences"
    );
}

#[test]
fn a_chapter_boundary_is_never_inside_a_sentence() {
    // `Corpus::text` joins chapters with a blank line specifically so the last
    // sentence of one chapter and the first of the next are not counted as one
    // sentence. Joining with a single newline leaves the text technically
    // different but the sentence count the same, because the sentinel counts both
    // boundaries -- so the mutation harness's newline swap survived, and the real
    // defect it hides is different and worse.
    //
    // Here: a chapter whose text does NOT end in terminal punctuation. The next
    // chapter starts mid-thought. With a blank line the segment ends at the
    // break, so the trailing fragment is its own (short) sentence; with a single
    // newline the two halves merge into one long sentence and the fragment
    // disappears into its neighbour. The sentence COUNT is therefore the
    // observable that distinguishes them.
    let unterminated = ChapterText {
        word_count: 1200,
        // No full stop: the sentence continues into the next chapter.
        plain_text: (0..20)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" "),
    };
    let following = chapter(60, 20);
    let corpus = Corpus {
        chapters: vec![unterminated, following],
        dialogue_words: 0,
    };

    let text = corpus.text();
    assert!(
        text.contains("\n\n"),
        "chapters must be joined with a blank line, not a single newline: {:?}",
        &text[..text.len().min(60)]
    );

    let measured = coords_of(&corpus);
    // 1 fragment (20 words) + 60 sentences of 20 words. If the boundary merged,
    // the fragment and the first real sentence would be one, giving 61.
    assert_eq!(
        measured.sentence_count, 61,
        "a chapter boundary must end a sentence; got {} sentences",
        measured.sentence_count
    );
    assert_eq!(
        measured.sentence_count,
        // The second chapter's own count, plus the first chapter's fragment.
        60 + 1,
        "the fragment before the break counts as its own sentence"
    );
}

#[test]
fn a_soft_wrap_inside_a_chapter_is_not_a_sentence_break() {
    // §49.3's splitter must treat a paragraph break as a boundary and an interior
    // newline as nothing at all. `chapter_revisions.plain_text` is stored prose,
    // and stored prose is soft-wrapped -- a chapter routinely contains newlines
    // that are not paragraph marks. Counting them as breaks would split every
    // wrapped line into its own "sentence" and, on a narrow-wrapped chapter, make
    // the measure report high variance for perfectly flat prose.
    //
    // Found by the mutation harness: relaxing the rule to "any newline is a break"
    // left all nine tests green, because every fixture joined chapters with "\n\n"
    // and none contained an interior newline. Measured on the two candidate rules:
    //
    //     text                        blank-line rule   any-newline rule
    //     "abc.\n\ndef."             [4, 4]            [4, 4]     same
    //     "abc\ndef."                 [7]               [3, 4]     DIFFERENT
    //     "ab\ncd ef."                [8]               [2, 6]     DIFFERENT
    //
    // So the mutant is not equivalent; the fixtures were simply incomplete.
    let wrapped = ChapterText {
        word_count: 1200,
        // One long sentence, wrapped at 40 characters, as a store would save it.
        plain_text: {
            let long = sentence(600);
            let mut out = String::new();
            let mut line = String::new();
            for word in long.split(' ') {
                if line.len() + word.len() + 1 > 40 {
                    out.push_str(&line);
                    out.push('\n');
                    line.clear();
                }
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(word);
            }
            out.push_str(&line);
            out
        },
    };

    let corpus = Corpus {
        chapters: vec![wrapped],
        dialogue_words: 0,
    };
    let measured = coords_of(&corpus);

    assert_eq!(
        measured.sentence_count, 1,
        "a soft-wrapped single sentence is still one sentence; got {}",
        measured.sentence_count
    );
    assert_eq!(
        measured.sentence_length_variance, 0.0,
        "one sentence has no spread, however it is wrapped"
    );

    // And the control: hard-wrapped at the same width, the same number of
    // sentences results. Wrapping must not change a coordinate.
    let unwrapped = ChapterText {
        word_count: 1200,
        plain_text: sentence(600),
    };
    let flat_text = coords_of(&Corpus {
        chapters: vec![unwrapped],
        dialogue_words: 0,
    });
    assert_eq!(
        measured.sentence_count, flat_text.sentence_count,
        "line wrapping must not change the sentence count"
    );
    assert_eq!(
        measured.sentence_length_variance, flat_text.sentence_length_variance,
        "line wrapping must not change the variance measure"
    );
}
