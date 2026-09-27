//! M11-17d — the shared lore-metadata contract (spec §2.3.1, §11.17, §19.14).
//!
//! Two things are pinned here, and they are not the same kind of thing.
//!
//! **The shape of the wire types** — round-tripping, defaults, and the version
//! handshake. Ordinary serde tests.
//!
//! **The privacy prohibition, as an executable property.** §11.17 requires that
//! reading history, progress, ratings, pseud linkage, credentials and so on
//! *cannot be sent*, and §0.3 requires that a sender which tries be "refused
//! with a named error, not silently truncated". Those are behavioural claims
//! about a type, and a type is exactly the thing that can be checked. The
//! prohibited-field test below walks a list of names the spec forbids and
//! asserts that each one is refused at deserialization and absent from the
//! serialized form. If a future field is added under one of those names, or a
//! `deny_unknown_fields` is dropped, this fails — which is the point. A privacy
//! boundary that is only described in a doc comment is a comment.
#![forbid(unsafe_code)]

use lorehaven_lore_metadata::{
    CanonicalBatch, CanonicalWork, Completion, EntityRef, ExchangeVersion, ReviewStatus,
    SignalBatch, WorkSignal,
};

/// Every name §11.17 and §0.3 forbid in a signal.
///
/// The list is the spec's, transcribed. A new prohibited concept means adding it
/// here, which is the point: the check is only as good as its enumeration, and
/// an enumeration that lives in a test is reviewable.
const FORBIDDEN: &[&str] = &[
    "reader_id",
    "account_id",
    "pseud_id",
    "reading_history",
    "read_history",
    "progress",
    "position",
    "read_status",
    "reading_status",
    "rating",
    "reader_rating",
    "my_rating",
    "notes",
    "kudos",
    "library",
    "library_membership",
    "shelves",
    "drafts",
    "draft_content",
    "credentials",
    "source_credentials",
    "cookie",
    "session_id",
    "session_token",
    "ip",
    "ip_address",
    "file_path",
    "path",
];

#[test]
fn every_forbidden_field_is_refused_rather_than_ignored() {
    for field in FORBIDDEN {
        let payload =
            format!(r#"{{"version":1,"signals":[{{"title":"a work","{field}":"anything"}}]}}"#);
        let parsed = serde_json::from_str::<SignalBatch>(&payload);
        assert!(
            parsed.is_err(),
            "a signal carrying {field:?} was accepted. §0.3 makes this a fact about a \
             reader, not a work, and §11.17 requires it refused *by name* — a lenient \
             deserializer would drop the field and leave the sender believing it was sent"
        );
    }
}

#[test]
fn the_serialized_form_carries_nothing_prohibited() {
    // The other half of the same property: even a batch built in Rust cannot
    // grow one of these fields, because there is nowhere to put it.
    let json = serde_json::to_string(&SignalBatch::new(vec![WorkSignal::titled("a work")]))
        .expect("serialize");
    for field in FORBIDDEN {
        assert!(
            !json.contains(&format!("\"{field}\"")),
            "a serialized signal carries {field:?}: {json}"
        );
    }
}

#[test]
fn a_minimal_signal_round_trips() {
    let batch = SignalBatch::new(vec![WorkSignal::titled("A Lighthouse in Winter")
        .on_site("ao3", "12345")
        .on_site("ffn", "67890")]);
    let json = serde_json::to_string(&batch).expect("serialize");
    let back: SignalBatch = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(batch, back);
    assert_eq!(back.version, ExchangeVersion::CURRENT);
    assert_eq!(back.signals[0].site_ids.len(), 2);
}

#[test]
fn optional_fields_are_omitted_rather_than_sent_as_null() {
    // A signal with nothing but a title must not carry a wall of nulls: the
    // fields are absent, which is the difference between "unknown" and "known
    // to be nothing" for a downstream canonicaliser.
    let json = serde_json::to_string(&WorkSignal::titled("Bare")).expect("serialize");
    for absent in [
        "fandom",
        "word_count",
        "content_hash",
        "language",
        "completion",
    ] {
        assert!(
            !json.contains(absent),
            "{absent} should be omitted, not sent as null: {json}"
        );
    }
    assert!(json.contains(r#""title":"Bare""#), "{json}");
}

#[test]
fn a_work_signal_needs_a_title() {
    // Without a title the signal cannot be matched to anything, so the required
    // field is genuinely required rather than merely conventional.
    assert!(serde_json::from_str::<WorkSignal>(r#"{"author_names":["x"]}"#).is_err());
}

#[test]
fn version_negotiation_agrees_inside_the_range() {
    let supported = ExchangeVersion::supported_range();
    assert!(supported.contains(ExchangeVersion::CURRENT));
    assert_eq!(
        ExchangeVersion::negotiate(ExchangeVersion::CURRENT),
        Ok(ExchangeVersion::CURRENT)
    );
}

#[test]
fn version_negotiation_refuses_with_the_supported_range() {
    // The refusal must carry the range. A client told only "no" has to make a
    // second round trip to find out what is possible; §2.3.1 asks for the
    // refusal *with* the range precisely so it does not have to.
    let too_new = ExchangeVersion(ExchangeVersion::CURRENT.0 + 1);
    let refused = ExchangeVersion::negotiate(too_new).expect_err("refused");
    assert_eq!(refused.requested, too_new);
    assert_eq!(refused.supported, ExchangeVersion::supported_range());
    assert!(
        refused.to_string().contains("not supported"),
        "the message names the problem: {refused}"
    );
    assert!(
        refused
            .to_string()
            .contains(&ExchangeVersion::CURRENT.to_string()),
        "and names what is possible: {refused}"
    );

    // Zero is below the floor and must be refused even though the shapes would
    // parse — a type that parses is not a contract that is honoured.
    let too_old = ExchangeVersion(0);
    assert!(
        ExchangeVersion::negotiate(too_old).is_err(),
        "a version below the supported floor is refused, not tolerated"
    );
}

#[test]
fn the_supported_range_is_inclusive_at_both_ends() {
    let range = ExchangeVersion::supported_range();
    assert!(range.contains(range.min));
    assert!(range.contains(range.max));
    assert!(!range.contains(ExchangeVersion(range.min.0 - 1)));
    assert!(!range.contains(ExchangeVersion(range.max.0 + 1)));
}

#[test]
fn a_canonical_work_round_trips_and_carries_its_review_status() {
    let work = CanonicalWork {
        entity: EntityRef {
            kind: "work".into(),
            id: "work-1".into(),
            aliases: vec!["A Lighthouse in Winter".into(), "Lighthouse, A".into()],
        },
        title: "A Lighthouse in Winter".into(),
        author_names: vec!["A. Writer".into()],
        fandom: Some("Example fandom".into()),
        tags: vec!["hurt/comfort".into()],
        characters: vec![],
        relationships: vec![],
        content_rating: Some("general".into()),
        word_count: Some(48_000),
        completion: Some(Completion::Complete),
        signal_count: 3,
        curated_at: Some("2026-09-27T00:00:00Z".into()),
        review_status: ReviewStatus::Unverified,
    };
    let json = serde_json::to_string(&CanonicalBatch::new(vec![work.clone()])).expect("serialize");
    let back: CanonicalBatch = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.works[0], work);
    assert_eq!(back.works[0].review_status, ReviewStatus::Unverified);
}

#[test]
fn canonical_metadata_never_carries_a_holder_or_submitter_count() {
    // §11.17: the response "never reveals who submitted a signal, and it never
    // reveals how many accounts hold a work". A holder count is not computable
    // from what the instance holds, so an endpoint offering one would be
    // offering a guess.
    let json = serde_json::to_string(&CanonicalBatch::new(vec![CanonicalWork {
        entity: EntityRef {
            kind: "work".into(),
            id: "w".into(),
            aliases: vec![],
        },
        title: "t".into(),
        author_names: vec![],
        fandom: None,
        tags: vec![],
        characters: vec![],
        relationships: vec![],
        content_rating: None,
        word_count: None,
        completion: None,
        signal_count: 7,
        curated_at: None,
        review_status: ReviewStatus::Unverified,
    }]))
    .expect("serialize");

    for forbidden in [
        "holder",
        "holders",
        "readers",
        "reader_count",
        "submitters",
        "submitted_by",
        "accounts",
        "follower",
        "library_size",
    ] {
        assert!(
            !json.contains(forbidden),
            "canonical metadata carries {forbidden:?}: {json}"
        );
    }
    // `signal_count` is present and is a review priority. Its presence is the
    // point: it is the *only* count, and it counts signals, not people.
    assert!(json.contains(r#""signal_count":7"#), "{json}");
}

#[test]
fn the_completion_enum_uses_wire_names() {
    // Pinned because a rename here is a silent contract change: the JSON is the
    // thing a third party depends on, and the Rust variant name is not it.
    for (rust, json) in [
        (Completion::Complete, "\"complete\""),
        (Completion::InProgress, "\"in_progress\""),
        (Completion::Abandoned, "\"abandoned\""),
    ] {
        let encoded = serde_json::to_string(&rust).expect("serialize");
        assert_eq!(encoded, json);
        assert_eq!(
            serde_json::from_str::<Completion>(json).expect("parse"),
            rust
        );
    }
}

#[test]
fn the_review_status_enum_uses_wire_names() {
    assert_eq!(
        serde_json::to_string(&ReviewStatus::Verified).expect("serialize"),
        "\"verified\""
    );
    assert_eq!(
        serde_json::to_string(&ReviewStatus::Unverified).expect("serialize"),
        "\"unverified\""
    );
}

#[test]
fn an_empty_batch_is_valid() {
    // A client with nothing to contribute must be able to say so. Refusing an
    // empty batch would push senders toward submitting a filler signal, which
    // is worse than an empty batch.
    let batch = SignalBatch::new(vec![]);
    let json = serde_json::to_string(&batch).expect("serialize");
    assert_eq!(
        serde_json::from_str::<SignalBatch>(&json).expect("parse"),
        batch
    );
}

#[test]
fn entity_kinds_are_open_so_a_third_party_can_add_one() {
    // Closed enums mean a version bump for every new kind of lore thing, which
    // defeats the point of a shared contract.
    let ref_ = EntityRef {
        kind: "motif".into(),
        id: "m1".into(),
        aliases: vec![],
    };
    let json = serde_json::to_string(&ref_).expect("serialize");
    assert!(json.contains(r#""kind":"motif""#), "{json}");
    assert_eq!(
        serde_json::from_str::<EntityRef>(&json).expect("parse"),
        ref_
    );
}

#[test]
fn aliases_default_to_empty() {
    let ref_: EntityRef = serde_json::from_str(r#"{"kind":"work","id":"w1"}"#).expect("parse");
    assert!(ref_.aliases.is_empty());
}
