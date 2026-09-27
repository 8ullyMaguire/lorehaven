//! The metadata exchange's own rules (spec §11.17, §15.17, §19.14, §16.16.1).
//!
//! Pure functions, no I/O. The endpoint's logic that can be stated without a
//! database lives here so it can be tested directly rather than only through a
//! router, and so the two rules that are easy to get subtly wrong — what a
//! signal is allowed to carry, and what a signal is allowed to *mean* — are
//! stated once.
//!
//! The distinction this module exists to protect: a signal is **evidence**, and
//! evidence has a use that is not authority. Receiving a signal never writes
//! canonical metadata; it creates an unverified entity and counts it. Everything
//! that turns evidence into a canonical value is a curation act, and §19.14
//! holds that to TL3.

use std::collections::{BTreeMap, BTreeSet};

use lorehaven_lore_metadata::{EntityRef, ReviewStatus, SignalBatch, WorkSignal};

/// §19.14: a TL1 account may submit a signal.
pub const TRUST_LEVEL_TO_SUBMIT: i64 = 1;

/// §19.14: curating canonical metadata requires TL3, plus §19.4's review.
///
/// Neither bar is configuration. §0.3 makes trust non-purchasable, so a config
/// key that lowered this would be a purchased moderation authority wearing a
/// different name. The constants are `const`, not config, for that reason — the
/// point is that there is no code path that reads a value from anywhere.
pub const TRUST_LEVEL_TO_CURATE: i64 = 3;

/// §19.14: submitting requires TL1, and the bar is a floor, not a ceiling.
pub fn may_submit(trust_level: i64) -> bool {
    trust_level >= TRUST_LEVEL_TO_SUBMIT
}

/// §19.14: curating requires TL3.
pub fn may_curate(trust_level: i64) -> bool {
    trust_level >= TRUST_LEVEL_TO_CURATE
}

/// The entity kinds a signal can name (§15.17).
///
/// A closed list because each kind has its own display rules in the tag
/// browser, and a kind with no rules would be a name nobody can curate. New
/// kinds are a migration, not a data change — the alternative is a `kind` string
/// that anything can write and a UI that has to cope with the unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntityKind {
    Fandom,
    Tag,
    Character,
    Relationship,
}

impl EntityKind {
    /// The wire/storage spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            EntityKind::Fandom => "fandom",
            EntityKind::Tag => "tag",
            EntityKind::Character => "character",
            EntityKind::Relationship => "relationship",
        }
    }

    /// Parse a stored or submitted kind. `None` for an unknown spelling.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "fandom" => Some(EntityKind::Fandom),
            "tag" => Some(EntityKind::Tag),
            "character" => Some(EntityKind::Character),
            "relationship" => Some(EntityKind::Relationship),
            _ => None,
        }
    }
}

/// Normalise a name for identity: the form two spellings of the same thing
/// collapse to.
///
/// Case-folded, whitespace-collapsed, and trimmed. Deliberately *not* more
/// aggressive than that: stripping punctuation or de-accenting would merge
/// distinct tag names (`A/B` and `A-B` are different ships, and `BDSM` and
/// `B.D.S.M` are not a tag anyone files), and an aggressive normaliser makes the
/// alias table unmanageable because the "obvious" duplicates are gone but the
/// real ones are not.
pub fn normalise(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut pending_space = false;
    for ch in raw.trim().chars() {
        if ch.is_whitespace() {
            // Collapse runs, and do not emit a leading or trailing space.
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        for lower in ch.to_lowercase() {
            out.push(lower);
        }
    }
    out
}

/// Every entity a signal names, as `(kind, normalised, original)`.
///
/// A `BTreeSet` on `(kind, norm)` so a signal that names `Slow Burn` twice, or
/// `slow burn` and `Slow  Burn`, contributes one entity rather than two. That
/// matters for §15.17: `signal_count` is review priority, and a submitter who
/// repeats a tag in one signal should not have their signal weighted higher than
/// one who lists it once.
///
/// The original spelling is kept alongside the normalised form because §15.17
/// requires a canonical value to have a *canonical form* to display, and the
/// first spelling a curator sees should be the one the submitter used. Sorting
/// is what makes the choice deterministic: two identical signals produce the
/// same set in the same order regardless of hash seed.
pub fn named_entities(signal: &WorkSignal) -> BTreeSet<(EntityKind, String, String)> {
    // Keyed by `(kind, norm)` with the *first* spelling seen as the value. The
    // first obvious implementation put the original spelling in the set key,
    // which silently defeats the deduplication: `Horror`, `horror` and `HORROR`
    // are one tag, and keying on the spelling made them three rows and three
    // increments of `signal_count` — the exact over-weighting §15.17 forbids. A
    // map keyed by identity is what makes "one signal names an entity once"
    // structural rather than a convention.
    let mut out: BTreeMap<(EntityKind, String), String> = BTreeMap::new();
    let mut push = |kind: EntityKind, values: &[String]| {
        for value in values {
            let norm = normalise(value);
            // An empty name is not evidence of anything. Skipping it here rather
            // than storing a blank row is the difference between a tag browser
            // that can show an unnamed tag and one that cannot.
            if norm.is_empty() {
                continue;
            }
            out.entry((kind, norm))
                // First spelling wins, and "first" is the order the signal
                // listed them in, so the display form is the submitter's own.
                .or_insert_with(|| value.trim().to_string());
        }
    };
    if let Some(f) = &signal.fandom {
        push(EntityKind::Fandom, std::slice::from_ref(f));
    }
    push(EntityKind::Tag, &signal.tags);
    push(EntityKind::Character, &signal.characters);
    push(EntityKind::Relationship, &signal.relationships);
    out.into_iter()
        .map(|((kind, norm), original)| (kind, norm, original))
        .collect()
}

/// The content hash §11.17 deduplicates by.
///
/// Hashed over the *canonical JSON* of the signal rather than over the fields in
/// declaration order, so two clients that build the same signal with their
/// fields in a different order produce the same hash. Hashing a serialisation
/// that is not canonical would make deduplication depend on a client's map
/// ordering, which is not a property the spec asks for but is exactly the kind
/// of thing that makes a "re-import costs nothing" promise false in practice.
///
/// Note this is a *content* hash in §11.17's sense: it covers the metadata
/// claims, not the submitter, not the source instance, and not the account. Two
/// instances submitting the same work deduplicate to one stored signal, which is
/// the intent — the signal is a fact about a work, and the fact is the same
/// whoever reports it.
pub fn content_hash(signal: &WorkSignal) -> String {
    use sha2::{Digest, Sha256};
    // BTreeMap-ordered serialisation, via the wire type's own `Serialize`, so
    // the hash is over the contract rather than over a private field list that
    // could drift from it.
    let canonical = serde_json::to_string(signal).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// A batch-level content hash, for the rate limiter's dedup and for logging.
pub fn batch_content_hash(batch: &SignalBatch) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for signal in &batch.signals {
        hasher.update(content_hash(signal).as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

/// The outcome of validating a submitted batch.
///
/// A refusal is a named reason, not a boolean: §11.17 requires a non-conforming
/// client to be "rejected by name rather than having its payload quietly
/// trimmed", and an error a client cannot read the reason out of is not a
/// rejection by name, it is a rejection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchRejection {
    /// The batch carried no signals.
    Empty,
    /// The contract version is outside the supported range.
    UnsupportedVersion(String),
    /// A signal had no title. The title is the one field with no default: a
    /// signal with a site id and a tag list but no name cannot be attached to
    /// anything a reader will see, so accepting it creates a record that can
    /// never be rendered.
    SignalWithoutTitle { index: usize },
    /// A signal named an entity kind this instance does not know.
    UnknownEntityKind { index: usize, kind: String },
    /// The batch was larger than the instance will accept in one call.
    TooManySignals { count: usize, limit: usize },
}

/// The largest batch §11.17's endpoint accepts in one call.
///
/// A batch limit and not only a rate limit: 1000 submissions per hour is a
/// per-hour budget, and a single call carrying every one of them would be a
/// single transaction large enough to hold a write lock on `exchange_signals`
/// for the length of the whole insert on SQLite. The limit is a constant rather
/// than configuration because §11.17 does not make it configurable — only the
/// rate is — and a knob nobody specified is a knob nobody has reasoned about.
pub const MAX_BATCH_SIGNALS: usize = 200;

/// Validate a batch for submission. Trust is checked separately, by the route.
pub fn validate_batch(batch: &SignalBatch) -> Result<(), BatchRejection> {
    if batch.signals.is_empty() {
        return Err(BatchRejection::Empty);
    }
    if batch.signals.len() > MAX_BATCH_SIGNALS {
        return Err(BatchRejection::TooManySignals {
            count: batch.signals.len(),
            limit: MAX_BATCH_SIGNALS,
        });
    }
    for (index, signal) in batch.signals.iter().enumerate() {
        if signal.title.trim().is_empty() {
            return Err(BatchRejection::SignalWithoutTitle { index });
        }
    }
    Ok(())
}

/// What the exchange learned from one accepted signal.
///
/// This is the whole of §15.17's response, and it is deliberately small. It
/// carries no submitter and no holder count (§11.17), and `signal_count` is the
/// *resulting* count for review ordering, which is the one number §15.17
/// permits — because it counts signals, not accounts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SignalOutcome {
    /// The signal's content hash, which is also its identity.
    pub content_hash: String,
    /// The entities this signal created or reinforced, newest count last.
    pub entities: Vec<EntityRef>,
    /// `true` when the signal was already stored — §11.17's deduplication, so a
    /// re-import costs the submitter nothing and creates no second record.
    pub duplicate: bool,
}

/// The acknowledgement returned by `POST /api/v1/exchange/signals`.
///
/// §11.17: the response reveals neither who submitted a signal nor how many
/// accounts hold a work. Neither is a field here, so there is nothing to leak —
/// the omission is structural rather than a field deliberately left blank at the
/// edge. `results` carries `content_hash`, the entities the signal touched, and
/// whether it was already stored, which is all a submitter needs to know that
/// their re-import cost them nothing.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SignalAck {
    /// How many signals were accepted, duplicates included.
    pub accepted: usize,
    /// How many of those were already stored (§11.17's deduplication).
    pub duplicates: usize,
    /// Per-signal detail.
    pub results: Vec<SignalOutcome>,
}

/// The review status a newly-seen entity starts in.
///
/// §15.17's load-bearing claim: an entity created from a signal is *usable
/// immediately*, carrying `review_status = unverified`. The alternative —
/// holding every new name behind quorum — makes a new tag invisible until
/// enough people independently file it, which on a small instance is never, and
/// makes the queue so large that reviewing it becomes the whole job.
pub fn initial_review_status() -> ReviewStatus {
    ReviewStatus::Unverified
}

#[cfg(test)]
mod tests {
    use super::*;
    use lorehaven_lore_metadata::{Completion, ExchangeVersion, SignalBatch, WorkSignal};

    fn signal() -> WorkSignal {
        WorkSignal {
            site_ids: vec![],
            title: "A Study in Emerald".into(),
            author_names: vec!["M. R. James".into()],
            fandom: Some("Sherlock Holmes".into()),
            tags: vec!["Horror".into()],
            characters: vec!["Sherlock Holmes".into()],
            relationships: vec![],
            content_rating: Some("general".into()),
            word_count: Some(12_000),
            chapter_count: None,
            completion: Some(Completion::Complete),
            content_hash: None,
            language: Some("en".into()),
            source_url: Some("https://example.org/story/1".into()),
            extracted_at: Some("2026-09-27T00:00:00Z".into()),
        }
    }

    fn batch(signals: Vec<WorkSignal>) -> SignalBatch {
        SignalBatch {
            version: ExchangeVersion::CURRENT,
            signals,
        }
    }

    #[test]
    fn normalise_collapses_case_and_whitespace() {
        assert_eq!(normalise("Slow  Burn"), "slow burn");
        assert_eq!(normalise("  SLOW BURN "), "slow burn");
        assert_eq!(normalise("slow burn"), "slow burn");
    }

    #[test]
    fn normalise_keeps_punctuation_that_distinguishes_names() {
        // `A/B` and `A-B` are different ships. An aggressive normaliser that
        // stripped punctuation would merge them and the alias table could never
        // be reasoned about.
        assert_ne!(normalise("A/B"), normalise("A-B"));
    }

    #[test]
    fn normalise_does_not_deaccent_or_casefold_beyond_lowercase() {
        assert_eq!(normalise("Amélie"), "amélie");
        assert_eq!(normalise("Amelie"), "amelie");
    }

    #[test]
    fn named_entities_collects_every_kind() {
        let got = named_entities(&signal());
        let kinds: Vec<_> = got.iter().map(|(k, _, _)| k.as_str()).collect();
        assert!(kinds.contains(&"fandom"));
        assert!(kinds.contains(&"tag"));
        assert!(kinds.contains(&"character"));
        assert!(!kinds.contains(&"relationship"), "none was named");
    }

    #[test]
    fn named_entities_deduplicates_within_one_signal() {
        // §15.17: `signal_count` is review priority, so a submitter who repeats a
        // tag in one signal must not be weighted above one who lists it once.
        let mut s = signal();
        s.tags = vec!["Horror".into(), "horror".into(), "HORROR".into()];
        s.characters = vec![];
        let got = named_entities(&s);
        let tags: Vec<_> = got
            .iter()
            .filter(|(k, _, _)| *k == EntityKind::Tag)
            .collect();
        assert_eq!(tags.len(), 1, "three spellings, one entity: {tags:?}");
    }

    #[test]
    fn named_entities_skips_empty_names() {
        let mut s = signal();
        s.tags = vec!["".into(), "   ".into(), "Real".into()];
        let got = named_entities(&s);
        let tags: Vec<_> = got
            .iter()
            .filter(|(k, _, _)| *k == EntityKind::Tag)
            .collect();
        assert_eq!(tags.len(), 1, "a blank name is not evidence: {got:?}");
    }

    #[test]
    fn named_entities_keeps_the_original_spelling() {
        let mut s = signal();
        s.tags = vec!["Slow Burn".into()];
        s.characters = vec![];
        let got = named_entities(&s);
        let (_, norm, original) = got.iter().find(|(k, _, _)| *k == EntityKind::Tag).unwrap();
        assert_eq!(norm, "slow burn");
        assert_eq!(
            original, "Slow Burn",
            "the curator sees what the submitter wrote"
        );
    }

    #[test]
    fn content_hash_is_stable_across_field_order() {
        // Two clients building the same signal in a different field order must
        // produce one hash, or "a re-import costs nothing" is false in practice.
        let a = signal();
        let mut b = WorkSignal {
            title: a.title.clone(),
            tags: a.tags.clone(),
            author_names: a.author_names.clone(),
            fandom: a.fandom.clone(),
            characters: a.characters.clone(),
            site_ids: vec![],
            relationships: vec![],
            content_rating: a.content_rating.clone(),
            word_count: a.word_count,
            chapter_count: None,
            completion: a.completion,
            content_hash: None,
            language: a.language.clone(),
            source_url: a.source_url.clone(),
            extracted_at: a.extracted_at.clone(),
        };
        b.title = a.title.clone();
        assert_eq!(content_hash(&a), content_hash(&b));
    }

    #[test]
    fn content_hash_ignores_the_submitter() {
        // The signal is a fact about a work; the fact is the same whoever reports
        // it. So the hash covers no account field — there is none on the type.
        assert_eq!(content_hash(&signal()), content_hash(&signal()));
    }

    #[test]
    fn content_hash_differs_when_a_fact_differs() {
        let a = signal();
        let mut b = signal();
        b.word_count = Some(12_001);
        assert_ne!(content_hash(&a), content_hash(&b));
    }

    #[test]
    fn an_empty_batch_is_refused_by_name() {
        let err = validate_batch(&batch(vec![])).expect_err("empty is refused");
        assert_eq!(err, BatchRejection::Empty);
    }

    #[test]
    fn a_signal_with_no_title_is_refused_by_index() {
        let mut s = signal();
        s.title = "   ".into();
        let err = validate_batch(&batch(vec![signal(), s])).expect_err("no title is refused");
        assert_eq!(err, BatchRejection::SignalWithoutTitle { index: 1 });
    }

    #[test]
    fn an_oversized_batch_is_refused_by_count() {
        let signals = (0..=MAX_BATCH_SIGNALS).map(|_| signal()).collect();
        let err = validate_batch(&batch(signals)).expect_err("oversized is refused");
        assert_eq!(
            err,
            BatchRejection::TooManySignals {
                count: MAX_BATCH_SIGNALS + 1,
                limit: MAX_BATCH_SIGNALS
            }
        );
    }

    #[test]
    fn a_well_formed_batch_is_accepted() {
        validate_batch(&batch(vec![signal()])).expect("a good batch is accepted");
    }

    #[test]
    fn trust_bars_are_asymmetric() {
        // §19.14: cheap to participate, expensive to be believed.
        assert!(may_submit(1), "TL1 may submit");
        assert!(!may_curate(1), "TL1 may not curate");
        assert!(!may_curate(2), "TL2 may not curate");
        assert!(may_curate(3), "TL3 may curate");
    }

    #[test]
    fn a_new_entity_starts_unverified() {
        // §15.17: usable immediately, visibly so.
        assert_eq!(initial_review_status(), ReviewStatus::Unverified);
    }

    #[test]
    fn the_ack_carries_no_submitter_and_no_holder_count() {
        let ack = SignalAck {
            accepted: 1,
            duplicates: 0,
            results: vec![SignalOutcome {
                content_hash: "abc".into(),
                entities: vec![],
                duplicate: false,
            }],
        };
        let json = serde_json::to_string(&ack).expect("serialises");
        for forbidden in ["account", "holder", "reader", "submitter", "readers"] {
            assert!(
                !json.contains(forbidden),
                "the acknowledgement must not carry `{forbidden}`: {json}"
            );
        }
    }

    #[test]
    fn entity_kinds_round_trip() {
        for kind in [
            EntityKind::Fandom,
            EntityKind::Tag,
            EntityKind::Character,
            EntityKind::Relationship,
        ] {
            assert_eq!(EntityKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(EntityKind::parse("mood"), None, "unknown kinds are refused");
    }
}
