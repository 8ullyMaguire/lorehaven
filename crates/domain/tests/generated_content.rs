//! §51's generated-content rules, and §51.2's vesting arithmetic.
//!
//! These are pure functions, so there is no engine and no fixture: every rule here
//! is one the write path and the credit path both depend on, and a test that only
//! exercised the SQL would let a change to the arithmetic through unnoticed.

use lorehaven_domain::generated_content::{
    credit_vested, trusted_reader_completions, CreditVesting, GeneratedContentDeclaration,
    GeneratedContentPosture, GeneratedContentPostureOnWork, TRUSTED_READERS_TO_VEST,
};

fn declared() -> CreditVesting {
    CreditVesting::ReaderCompleted { trusted: true }
}

#[test]
fn a_new_instance_forbids_generated_content() {
    // §51.1: `forbid` is the default, and that is the load-bearing default.
    // `allow` is the only value that changes what the corpus IS.
    assert_eq!(
        GeneratedContentPosture::default(),
        GeneratedContentPosture::Forbid,
        "an instance that says nothing gets the safe posture, not the permissive one"
    );
}

#[test]
fn forbid_refuses_and_the_others_accept() {
    // The one decision §51.1 turns on. Note what is NOT consulted: nothing here
    // looks at the author, because §51.5 refuses a per-author opt-out under
    // `forbid` -- a posture an author can decline is `allow` with extra steps.
    let resolutions: Vec<bool> = GeneratedContentPosture::all()
        .iter()
        .map(|p| {
            GeneratedContentPostureOnWork::resolve(
                GeneratedContentDeclaration::DeclaredGenerated,
                *p,
            )
            .is_ok()
        })
        .collect();
    assert_eq!(
        resolutions,
        vec![false, true, true],
        "forbid refuses; disclose and allow accept"
    );
}

#[test]
fn forbid_refuses_with_a_message_that_names_the_posture() {
    let refusal = GeneratedContentPostureOnWork::resolve(
        GeneratedContentDeclaration::DeclaredGenerated,
        GeneratedContentPosture::Forbid,
    )
    .expect_err("forbid refuses a declared-generated work");
    assert_eq!(refusal.posture, GeneratedContentPosture::Forbid);
    let text = refusal.to_string();
    assert!(
        text.contains("forbids generated content"),
        "the message names the policy: {text}"
    );
    assert!(
        text.contains("not created"),
        "the message says the work was NOT created -- §51.1 refuses before the row \
         exists, so 'no work' is the fact the author needs: {text}"
    );
}

#[test]
fn nothing_is_refused_when_nothing_is_declared() {
    // §51.5: `forbid` is about generated content, not about a declaration existing.
    // A work that says nothing about itself is a normal work under every posture.
    for posture in GeneratedContentPosture::all() {
        assert!(
            GeneratedContentPostureOnWork::resolve(
                GeneratedContentDeclaration::Undeclared,
                posture
            )
            .is_ok(),
            "an undeclared work is never refused under {posture}"
        );
    }
}

#[test]
fn only_disclose_puts_a_marker_on_the_work() {
    // §51.1: `allow` deliberately does not label. Making it label would turn `allow`
    // into a worse `disclose` rather than a real third option.
    // `forbid` is absent from this list on purpose, not by omission: it refuses
    // before a `PostureOnWork` exists, so there is no marker question to ask. An
    // earlier version unwrapped all three with `.expect(...)` and failed on the
    // refusal -- which is the refusal working.
    let markers: Vec<(GeneratedContentPosture, bool)> = GeneratedContentPosture::all()
        .iter()
        .filter_map(|p| {
            GeneratedContentPostureOnWork::resolve(
                GeneratedContentDeclaration::DeclaredGenerated,
                *p,
            )
            .ok()
            .map(|resolved| (*p, resolved.needs_marker()))
        })
        .collect();
    assert_eq!(
        markers,
        vec![
            (GeneratedContentPosture::Disclose, true),
            (GeneratedContentPosture::Allow, false),
        ],
        "only disclose labels; forbid has no work to label and allow declines to"
    );
}

#[test]
fn disclose_labels_nothing_when_nothing_was_declared() {
    // The clause is about LABELING, not about the instance's policy. An instance
    // under `disclose` with ordinary works shows no generated-content banner on
    // them -- a marker on every work would be a marker nobody reads.
    let resolved = GeneratedContentPostureOnWork::resolve(
        GeneratedContentDeclaration::Undeclared,
        GeneratedContentPosture::Disclose,
    )
    .expect("accepted");
    assert!(
        !resolved.needs_marker(),
        "an undeclared work carries no marker even under disclose"
    );
}

#[test]
fn a_posture_round_trips_through_its_stored_string() {
    // 0109's CHECK stores exactly these three strings, so the parser and the CHECK
    // must agree on which strings are legal.
    for posture in GeneratedContentPosture::all() {
        assert_eq!(
            posture.as_str().parse::<GeneratedContentPosture>(),
            Ok(posture),
            "{posture} must parse back to itself"
        );
    }
}

#[test]
fn an_unknown_posture_is_refused_rather_than_defaulted() {
    // A lenient parse would turn a typo into `forbid` -- the SAFE default, so a
    // typo would silently forbid generated content on an instance that allows it,
    // and the author would get a refusal naming a policy nobody set.
    for bad in ["maybe", "forbidden", "", "FORBID", "Disclose", " allow"] {
        let parsed = bad.parse::<GeneratedContentPosture>();
        assert!(parsed.is_err(), "{bad:?} must not parse as a posture");
        let err = parsed.expect_err("rejected").to_string();
        assert!(
            err.contains("forbid, disclose, allow"),
            "the error names the legal set: {err}"
        );
    }
}

#[test]
fn the_posture_parse_is_case_sensitive_on_purpose() {
    // Verified against the database, not assumed: SQLite's CHECK is case-sensitive,
    // so `DISCLOSE` would be refused at INSERT. A case-insensitive parser would
    // build a valid value the database then rejects, and the error would name the
    // database rather than the input.
    assert!("DISCLOSE".parse::<GeneratedContentPosture>().is_err());
    assert!("ALLOW".parse::<GeneratedContentPosture>().is_err());
    assert!("disclose".parse::<GeneratedContentPosture>().is_ok());
}

#[test]
fn the_three_values_are_all_there_is() {
    // Not a count for its own sake: the module must not grow a fourth posture
    // without this test and 0109's CHECK both being updated, and this is the one
    // that fails when somebody adds a variant and forgets the list.
    assert_eq!(GeneratedContentPosture::all().len(), 3);
    let strings: Vec<&str> = GeneratedContentPosture::all()
        .iter()
        .map(|p| p.as_str())
        .collect();
    assert_eq!(strings, vec!["forbid", "disclose", "allow"]);
}

// ── §51.2: credit vests on reader completion ──────────────────────────────────

#[test]
fn credit_is_not_vested_by_posting_words() {
    // The rule that answers A3's farming problem. Posting more words is the action
    // a generator can take for free, so it must not move the number.
    let events: Vec<CreditVesting> = (0..100).map(|_| CreditVesting::WordsPosted).collect();
    assert_eq!(
        trusted_reader_completions(&events),
        0,
        "posting words is not completion"
    );
    assert!(
        !credit_vested(&events),
        "100 posted word-events vest nothing"
    );
}

#[test]
fn credit_needs_three_distinct_trusted_readers() {
    // `TRUSTED_READERS_TO_VEST` is named in the spec and here, so retuning it is
    // one act rather than a number discovered in three places.
    let events: Vec<CreditVesting> = (0..TRUSTED_READERS_TO_VEST).map(|_| declared()).collect();
    assert!(
        credit_vested(&events),
        "exactly the threshold vests: {} events",
        events.len()
    );

    let short = vec![declared(), declared()];
    assert!(
        !credit_vested(&short),
        "one short of the threshold does not vest"
    );
}

#[test]
fn an_untrusted_completion_does_not_count() {
    // §51.2 names TRUSTED readers as the set. A partial credit for an untrusted
    // reader would be a weight §33's tiers do not describe.
    let events: Vec<CreditVesting> = (0..10)
        .map(|_| CreditVesting::ReaderCompleted { trusted: false })
        .collect();
    assert_eq!(trusted_reader_completions(&events), 0);
    assert!(
        !credit_vested(&events),
        "ten untrusted completions vest nothing"
    );
}

#[test]
fn mixed_events_count_only_the_trusted_completions() {
    let events = vec![
        CreditVesting::WordsPosted,
        declared(),
        CreditVesting::ReaderCompleted { trusted: false },
        declared(),
        CreditVesting::WordsPosted,
        declared(),
        CreditVesting::ReaderCompleted { trusted: false },
        CreditVesting::ReaderCompleted { trusted: false },
    ];
    assert_eq!(trusted_reader_completions(&events), 3);
    assert!(
        credit_vested(&events),
        "three trusted completions among noise is still three"
    );
}

#[test]
fn no_events_vests_nothing() {
    // §49.3's absent-is-not-a-zero rule, restated for the credit count. Zero
    // completions must not read as "some completions, but not enough".
    assert_eq!(trusted_reader_completions(&[]), 0);
    assert!(!credit_vested(&[]));
}

#[test]
fn one_completion_short_does_not_vest_but_the_threshold_does() {
    // §51.2 keeps un-vested credit VISIBLE on the author's profile, so a reader sees
    // what has actually been confirmed. That makes the boundary a published number,
    // and this pins it from both sides: an author is told "confirmed by N readers"
    // and that has to mean what it says at both N-1 and N.
    let below = vec![declared(); TRUSTED_READERS_TO_VEST - 1];
    let at = vec![declared(); TRUSTED_READERS_TO_VEST];

    assert!(
        !credit_vested(&below),
        "{} trusted completions must not read as confirmed",
        TRUSTED_READERS_TO_VEST - 1
    );
    assert!(
        credit_vested(&at),
        "{} trusted completions must read as confirmed",
        TRUSTED_READERS_TO_VEST
    );
    // And the count is what moved, not some other part of the arithmetic.
    assert_eq!(
        trusted_reader_completions(&below) + 1,
        trusted_reader_completions(&at)
    );
}

#[test]
fn an_author_who_posts_more_words_does_not_re_vest() {
    // §51.5: no credit decay or re-vesting. This is pure arithmetic, so what it
    // pins is that `credit_vested` is a function of completions alone -- there is no
    // word count anywhere in it for more posting to move.
    let vested = vec![declared(), declared(), declared()];
    let posted_more = {
        let mut e = vested.clone();
        e.extend(std::iter::repeat_n(CreditVesting::WordsPosted, 500));
        e
    };
    assert_eq!(
        credit_vested(&posted_more),
        credit_vested(&vested),
        "adding 500 posted-word events changes nothing"
    );
    assert_eq!(trusted_reader_completions(&posted_more), 3);
}
