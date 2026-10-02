//! §50.2's canon-agnostic class, and §50.3's "reproducible on both engines".
//!
//! The class answers "can this work be read cold?" using only the text — §50.3
//! forbids a franchise database, a model, and any cross-reader pooling — so the
//! tests are mostly about which tokens are and are not counted as unexplained
//! proper nouns.

use lorehaven_domain::canon::{classify, CanonClass, CANON_DENSITY_CEILING, MIN_CANON_WORDS};

/// Build a text of `words` tokens, so a test can sit above `MIN_CANON_WORDS`
/// without pasting prose.
///
/// The tokens are TERMINATED, which matters more than it looks: a capital is
/// excluded when it starts a sentence, so a fixture with no terminators has no
/// sentence-initial tokens and every capital in it counts as a name. Real prose
/// is punctuated, so the fixture must be too.
fn plain(words: usize) -> String {
    (0..words).map(|i| format!("word{i}. ")).collect()
}

/// The same, with `names` mid-sentence proper nouns sprinkled through.
fn with_names(words: usize, names: usize) -> String {
    let mut out = String::new();
    let mut left = names;
    for i in 0..words {
        // Every fourth word is capitalised and mid-sentence: preceded by a space
        // and followed by a space, never after a terminator.
        // Slot 0 carries NO terminator, which is what makes slot 1 the
        // mid-sentence slot: the tokenizer marks the token FOLLOWING a terminator
        // as sentence-initial, so a name placed straight after `word{i-1}.` is
        // excluded and the density comes out as 0.0.
        //
        // Getting this backwards is silent. The helper still emits plausible
        // prose, the fixture still has a hundred capitalised tokens in it, and
        // only the count is wrong -- which is why this helper spells out the slot
        // instead of leaving it to be worked out from the output.
        out.push_str(&match i % 4 {
            0 => format!("word{i} "),
            1 if left > 0 => {
                left -= 1;
                format!("Name{i}. ")
            }
            _ => format!("word{i}. "),
        });
    }
    // A terminator-free blob, so no Name ever lands sentence-initial.
    out
}

#[test]
fn plain_prose_is_canon_agnostic() {
    let text = plain(1000);
    let (class, measures) = classify(&text, 1000).expect("1000 words is classifiable");
    assert_eq!(class, CanonClass::CanonAgnostic);
    assert_eq!(
        measures.unexplained_names, 0,
        "lowercase prose has no unexplained proper nouns"
    );
    assert_eq!(measures.density, 0.0);
}

#[test]
fn dense_proper_nouns_make_it_canon_dependent() {
    let text = with_names(1000, 100);
    let (class, measures) = classify(&text, 1000).expect("classifiable");
    assert_eq!(
        class,
        CanonClass::CanonDependent,
        "100 unexplained names in 1000 words is {} density, well over the {CANON_DENSITY_CEILING} ceiling",
        measures.density
    );
    assert!(measures.density > CANON_DENSITY_CEILING);
}

#[test]
fn a_repeated_name_stops_counting_once_the_text_establishes_it() {
    // "Spock" mid-sentence, then "spock" later: the text has introduced the name,
    // so a reader who has not met the franchise can still follow it.
    let mut text = plain(1000);
    // Spock twice mid-sentence, then once in lowercase. A counter with no memory
    // would report 3; one that remembers reports 1. A fixture with a name
    // appearing only ONCE cannot tell those two apart, which is the trap this
    // replaces -- the first version of this test had a single `Spock` and passed
    // against the mutation that removed the `established` set entirely.
    text.push_str(" Then Spock met Spock and spock left. ");
    let (class, measures) = classify(&text, 1000).expect("classifiable");
    assert_eq!(
        measures.unexplained_names, 1,
        "two mid-sentence Spocks share one lowercase form, so only the first is \
         unexplained; the third is established by the lowercase mention"
    );
    assert_eq!(
        class,
        CanonClass::CanonAgnostic,
        "one unexplained name is well inside the ceiling"
    );
}

#[test]
fn sentence_initial_capitals_are_not_proper_nouns() {
    // Without this exclusion every work scores as canon-dependent, because every
    // sentence in English starts with a capital.
    let text = plain(1000)
        .replace("word1 ", "Every ")
        .replace("word2 ", "Reader ")
        .replace("word3 ", "Knows ");
    let (_, measures) = classify(&text, 1000).expect("classifiable");
    assert_eq!(
        measures.unexplained_names, 0,
        "a capital that is just orthography carries no information"
    );
}

#[test]
fn every_terminator_ends_a_sentence_not_just_the_full_stop() {
    // `!` and `?` are sentence ends in §49.3's splitter and must be here too.
    // An implementation that recognised only `.` treats a capital after an
    // exclamation as a name, which silently penalises dialogue-heavy prose --
    // exactly the register where capitals are most likely.
    let cases = [
        ("She stopped. Rowan had already gone.", "a full stop"),
        ("She stopped! Rowan had already gone.", "an exclamation"),
        ("She stopped? Rowan had already gone.", "a question"),
        ("She stopped\nRowan had already gone.", "a newline"),
    ];
    for (text, label) in cases {
        let mut fixture = plain(1000);
        fixture.push_str(&format!(" {text} "));
        let (_, measures) = classify(&fixture, 1000).expect("classifiable");
        assert_eq!(
            measures.unexplained_names, 0,
            "{label} must end a sentence, or Rowan reads as an unexplained name"
        );
    }
}

#[test]
fn shouted_capitals_and_initialisms_are_not_names() {
    // "NO" and "FBI" are far more often emphasis or an initialism than a name,
    // and an initialism stalls only readers who already know it.
    let mut text = plain(1000);
    text.push_str(" She said NO and then the FBI arrived. ");
    let (_, measures) = classify(&text, 1000).expect("classifiable");
    assert_eq!(measures.unexplained_names, 0);
}

#[test]
fn text_below_the_threshold_is_unclassified_rather_than_either_answer() {
    // §49.3's rule, restated: an absent coordinate is not a zero. A 200-word
    // story is not "canon-agnostic", it is unmeasured.
    let text = with_names(200, 50);
    assert_eq!(
        classify(&text, 200),
        None,
        "200 words is below {MIN_CANON_WORDS} and must not be classified"
    );
    assert!(
        classify(&plain(MIN_CANON_WORDS), MIN_CANON_WORDS).is_some(),
        "exactly at the threshold IS classifiable"
    );
}

#[test]
fn the_boundary_is_inclusive() {
    // A work exactly at the ceiling is canon-agnostic. `at` rather than `over`
    // because §50.2 wants eligibility decided generously: the cost of offering a
    // canon-blind work to a reader who did know its parent is small, and the cost
    // of hiding a standalone work is that it is never found.
    let words = 1000;
    let allowed = (words as f64 * CANON_DENSITY_CEILING) as usize;
    let text = with_names(words, allowed);
    let (class, measures) = classify(&text, words).expect("classifiable");
    assert!(
        measures.unexplained_names <= allowed,
        "the fixture should not exceed the ceiling, got {}",
        measures.unexplained_names
    );
    assert_eq!(
        class,
        CanonClass::CanonAgnostic,
        "exactly at the ceiling must count as eligible"
    );
}

#[test]
fn the_same_text_always_gives_the_same_answer() {
    // §50.3: "Coordinates and canon-agnostic flags are reproducible on both
    // engines." The classifier keeps a `HashSet`, and a hash set iterated in a
    // different order per process would make the answer drift -- so this is the
    // test that would catch the set leaking into the result.
    let text = with_names(1000, 40);
    let first = classify(&text, 1000).expect("classifiable");
    for _ in 0..16 {
        assert_eq!(
            classify(&text, 1000),
            Some(first),
            "the same text must classify identically every time"
        );
    }
}

#[test]
fn an_empty_text_is_unclassified_rather_than_a_zero() {
    // §49.3's rule again: an absent value is not a zero. Zero words is below the
    // threshold, so it is `None` -- not "canon-agnostic", and not a division by
    // zero producing a NaN density either.
    assert_eq!(
        classify("", 0),
        None,
        "zero words is below the threshold and must not be classified"
    );

    // The same holds for text that EXISTS with a word_count below the threshold:
    // the classification is absent, not zero, which is why `classify` has no
    // divide-by-zero branch to get wrong.
    assert_eq!(
        classify(&plain(100), 100),
        None,
        "100 real words is still below the threshold"
    );
}

#[test]
fn non_latin_text_does_not_panic_or_miscount() {
    // §49.3's measures are Unicode-aware for the same reason. Japanese has no
    // upper case at all, so every work in the language is canon-agnostic here --
    // which is a real limitation of a capitalisation heuristic, asserted rather
    // than left to be discovered in production.
    let text = plain(1000).replace("word1", "コーヒー");
    let (class, _) = classify(&text, 1000).expect("classifiable");
    assert_eq!(
        class,
        CanonClass::CanonAgnostic,
        "a capitalisation heuristic cannot see canon dependence in caseless scripts"
    );
}

#[test]
fn the_class_is_a_class_not_a_score() {
    // §50.2: "Canon-agnostic works are a class, not a score." The type has two
    // variants and no f64 anywhere in it, so nothing can blend it into a ranking
    // even by accident.
    let (class, _) = classify(&plain(1000), 1000).expect("classifiable");
    let as_str = format!("{class:?}");
    assert!(matches!(
        as_str.as_str(),
        "CanonAgnostic" | "CanonDependent"
    ));
    assert!(
        !as_str.contains('.'),
        "the class must not carry a fractional value: {as_str}"
    );
}
