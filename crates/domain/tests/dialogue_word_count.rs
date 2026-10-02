#[cfg(test)]
mod dialogue_tests {
    use lorehaven_domain::coordinates::dialogue_word_count;

    const OPEN: char = '\u{201C}';
    const CLOSE: char = '\u{201D}';

    fn quoted(inner: &str) -> String {
        format!("{OPEN}{inner}{CLOSE}")
    }

    #[test]
    fn counts_only_the_words_inside_quotes() {
        let text = format!(
            "She said {} and then she left the room.",
            quoted("I am going")
        );
        assert_eq!(
            dialogue_word_count(&text),
            3,
            "only the three quoted words are dialogue"
        );
    }

    #[test]
    fn text_with_no_quotes_has_no_dialogue() {
        // §49.3 is explicit that a dialogue ratio of exactly 0.0 is a real
        // measurement and NOT the same as unmeasured, so the all-zero case has
        // to be a count of zero rather than a refusal.
        let text = "The door opened. Nobody was there. The room was cold.";
        assert_eq!(dialogue_word_count(text), 0);
    }

    #[test]
    fn straight_quotes_are_not_dialogue_marks() {
        // A straight quote is also an inch mark. Counting `6"` as a quoted word
        // is worse than counting nothing, so the parser only accepts curly ones.
        let text = "The shelf was 6\" deep, and the box was 12\" wide.";
        assert_eq!(
            dialogue_word_count(text),
            0,
            "inch marks must not be read as dialogue"
        );
    }

    #[test]
    fn an_unclosed_quote_is_closed_at_the_end_not_discarded() {
        // Text pasted from a word processor often has one unmatched quote.
        // Discarding the rest of the chapter would under-report the ratio in a
        // way that looks like a deliberate measurement.
        let text = format!("Then she said {} and after that everything changed", OPEN);
        assert_eq!(dialogue_word_count(&text), 5);
    }

    #[test]
    fn a_closing_quote_ends_the_run() {
        let text = quoted("hello");
        assert_eq!(dialogue_word_count(&text), 1);
    }

    #[test]
    fn counts_across_several_quoted_spans() {
        let text = format!("{} then {}", quoted("one two"), quoted("three"));
        assert_eq!(dialogue_word_count(&text), 3);
    }

    #[test]
    fn two_spans_touching_do_not_merge_into_one_word() {
        // No whitespace between the closing and opening quote. The word run has
        // to be broken by the quote marks themselves, or the two spans read as one
        // word and the count comes out one short -- and only for dialogue that
        // happens to abut, which is why it needs its own test.
        let text = format!("{}{}", quoted("alpha"), quoted("beta"));
        assert_eq!(
            dialogue_word_count(&text),
            2,
            "two quoted spans are two words even with nothing between them"
        );
    }

    #[test]
    fn hyphenated_and_apostrophised_words_count_once() {
        // Punctuation inside a quoted run must not split a word, or a
        // characteristically hyphenated style reads as very talkative.
        let text = quoted("well-known and don't");
        assert_eq!(
            dialogue_word_count(&text),
            3,
            "three words: `well-known`, `and`, `don't`. A hyphen and an apostrophe \
             are INSIDE a word, so neither splits one -- an implementation that \
             split on them would report five."
        );
        // And each joiner on its own, with no ambiguity about the count.
        assert_eq!(dialogue_word_count(&quoted("well-known")), 1);
        assert_eq!(dialogue_word_count(&quoted("don't")), 1);
    }

    #[test]
    fn a_cjk_run_counts_as_one_contiguous_run() {
        // CJK is written WITHOUT spaces, so a word count is a fiction here. What
        // the counter can honestly report -- and what §49.3 needs, since it wants
        // a reproducible number rather than a linguistically correct one -- is the
        // number of contiguous non-punctuation runs inside the quotation marks.
        // A CJK ideographic comma therefore ENDS a run, which is why the earlier
        // `is_ascii_punctuation` implementation counted this as one long word
        // and a Latin implementation counted it as several.
        // Curly quotes, NOT the CJK brackets: the counter looks for \u{201C} and
        // \u{201D} only, and a \u{300C}\u{300D} span is not one of them. Using
        // the wrong brackets here reads like a word-boundary bug when it is
        // really the opening mark.
        let text = format!("{OPEN}\u{3053}\u{3093}\u{306B}\u{3061}\u{306F}{CLOSE}");
        assert_eq!(
            dialogue_word_count(&text),
            1,
            "an unpunctuated CJK run is one run"
        );

        let punctuated = format!("{OPEN}\u{3053}\u{3093}\u{3001}\u{306B}\u{3061}\u{306F}{CLOSE}");
        assert_eq!(
            dialogue_word_count(&punctuated),
            2,
            "an ideographic comma ends a run, as it does visually"
        );
    }

    #[test]
    fn the_same_text_always_gives_the_same_count() {
        let text = format!(
            "A said {}, B said {}, and C said nothing.",
            quoted("one"),
            quoted("two three")
        );
        let first = dialogue_word_count(&text);
        for _ in 0..8 {
            assert_eq!(
                dialogue_word_count(&text),
                first,
                "§49.3: the same text gives the same answer, every time"
            );
        }
    }

    #[test]
    fn a_quote_containing_no_words_counts_nothing() {
        let text = format!("She said {} and left.", quoted("..."));
        assert_eq!(
            dialogue_word_count(&text),
            0,
            "a run of punctuation is not a word"
        );
    }
}
