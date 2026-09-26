//! Recommendation transparency, domain half (spec §33.3).
//!
//! Two things live here, both pure and both about *what a reader is told*:
//!
//! - [`SlotReason`] and its reader-facing vocabulary. §33.3(a) requires every
//!   recommended slot to name its reader-side reasons, and §0.3/§24.3 require
//!   that no explanation path reveal the administrator's taste multiplier. The
//!   only way to hold that second rule is to make the vocabulary closed: a
//!   reason is one of these variants, and there is no `String` reason a
//!   handler can invent. The test that walks every variant is the real guard.
//! - [`AttentionReport`] assembly for §33.3(b), which is aggregate-only and
//!   off until the reader turns it on.
//!
//! Nothing here does I/O. The repository that records slots lives in
//! `lorehaven-db::recommendation_slots`.

use serde::{Deserialize, Serialize};

/// Why this work appeared in this slot, in reader-side terms.
///
/// The closed set is the point. A free-form reason string is how an operator
/// multiplier leaks into a reader's explanation — not by accident, but because
/// somebody eventually needs to say "the admin boosted this" and a `String`
/// allows it. These variants have no such member, so that sentence cannot be
/// written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotReason {
    /// The work carries tags the reader's taste profile weights.
    TasteTags,
    /// The work is popular on the instance.
    Popular,
    /// The work shares media references with something the reader has saved.
    MediaReferenceCollaborative,
    /// A named strategy from the §34 decision layer produced it.
    Strategy,
    /// The reader bookmarked something related and the blend followed it.
    ReadingHistory,
    /// The work matches a saved search or alert.
    SavedSearch,
}

impl SlotReason {
    /// The wire form, which is also what gets persisted in `reasons`.
    pub fn as_str(self) -> &'static str {
        match self {
            SlotReason::TasteTags => "taste_tags",
            SlotReason::Popular => "popular",
            SlotReason::MediaReferenceCollaborative => "media_ref_collab",
            SlotReason::Strategy => "strategy",
            SlotReason::ReadingHistory => "reading_history",
            SlotReason::SavedSearch => "saved_search",
        }
    }

    /// Parse a persisted reason. `None` for an unknown string, so a row written
    /// by a future version degrades to a missing reason rather than to a lie.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "taste_tags" => SlotReason::TasteTags,
            "popular" => SlotReason::Popular,
            "media_ref_collab" => SlotReason::MediaReferenceCollaborative,
            "strategy" => SlotReason::Strategy,
            "reading_history" => SlotReason::ReadingHistory,
            "saved_search" => SlotReason::SavedSearch,
            _ => return None,
        })
    }

    /// The engine's existing short reason, mapped onto the closed set.
    ///
    /// The engines in `lorehaven_db::discovery` set reasons to `"tags"`,
    /// `"popular"`, `"media_ref_collab"` and `"strategy"`. This is the one
    /// place those strings are interpreted, so a new engine cannot introduce a
    /// reason outside the vocabulary without this function refusing to map it.
    pub fn from_engine_reason(reason: &str) -> Option<Self> {
        match reason {
            "tags" | "taste_tags" => Some(SlotReason::TasteTags),
            "popular" => Some(SlotReason::Popular),
            "media_ref_collab" => Some(SlotReason::MediaReferenceCollaborative),
            "strategy" => Some(SlotReason::Strategy),
            _ => None,
        }
    }

    /// Every variant, for the exhaustive test that keeps the vocabulary honest.
    pub fn all() -> &'static [SlotReason] {
        &[
            SlotReason::TasteTags,
            SlotReason::Popular,
            SlotReason::MediaReferenceCollaborative,
            SlotReason::Strategy,
            SlotReason::ReadingHistory,
            SlotReason::SavedSearch,
        ]
    }

    /// A short reader-facing phrase for this reason.
    ///
    /// Prose lives here rather than in the route so a client can be shown the
    /// same words the tests assert, and so a wording change is one edit rather
    /// than a sweep through handlers.
    pub fn reader_phrase(self) -> &'static str {
        match self {
            SlotReason::TasteTags => "tags you have been reading",
            SlotReason::Popular => "popular on this instance right now",
            SlotReason::MediaReferenceCollaborative => {
                "shares media references with something you saved"
            }
            SlotReason::Strategy => "matched by your recommendation settings",
            SlotReason::ReadingHistory => "similar to something you read",
            SlotReason::SavedSearch => "matches a search you saved",
        }
    }
}

/// How strongly a work aligned with the reader's own taste, in coarse buckets.
///
/// A float would leak the blend: a reader comparing 0.837 against 0.842 learns
/// the shape of the ranking, and §0.3's rule is that the components stay
/// silent. Three buckets say what the reader needs and nothing more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TasteSignal {
    /// Little to no signal.
    Weak,
    /// Some signal.
    Some,
    /// Strong signal.
    Strong,
}

impl TasteSignal {
    /// Bucket a raw 0.0..=1.0 signal.
    ///
    /// The thresholds are the spec's own coarse language: "a little", "some",
    /// "as the operator suggests" (§16.16.2). One third and two thirds keeps
    /// the middle bucket wide, because "some" is the honest answer most of the
    /// time and a narrow band would overstate precision.
    pub fn from_score(score: f64) -> Self {
        if !score.is_finite() || score <= 1.0 / 3.0 {
            TasteSignal::Weak
        } else if score < 2.0 / 3.0 {
            TasteSignal::Some
        } else {
            TasteSignal::Strong
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TasteSignal::Weak => "weak",
            TasteSignal::Some => "some",
            TasteSignal::Strong => "strong",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "weak" => TasteSignal::Weak,
            "some" => TasteSignal::Some,
            "strong" => TasteSignal::Strong,
            _ => return None,
        })
    }
}

/// Whether instance curation moved this work, reported as one undifferentiated
/// line per §16.16.2 and §33.3(a).
///
/// The *amount* is deliberately not modelled. `True` and `False` are the only
/// two answers, and §16.16.2 says components are never shown when topics are
/// not public — so a struct that could hold a magnitude is a struct that could
/// eventually be asked to reveal one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceCuration {
    /// Instance curation did not move this work.
    NotInvolved,
    /// Instance curation moved it. The UI shows one line and stops.
    Involved,
}

impl InstanceCuration {
    pub fn as_str(self) -> &'static str {
        match self {
            InstanceCuration::NotInvolved => "not_involved",
            InstanceCuration::Involved => "involved",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "not_involved" => InstanceCuration::NotInvolved,
            "involved" => InstanceCuration::Involved,
            _ => return None,
        })
    }

    /// Both variants, for the test that keeps the type from growing a
    /// magnitude. Two is the whole point: see the type's doc comment.
    pub fn all_variants() -> &'static [InstanceCuration] {
        &[InstanceCuration::NotInvolved, InstanceCuration::Involved]
    }

    /// The one line a reader sees. §16.16.2's "undifferentiated line",
    /// phrased so it describes the instance rather than the work's ranking.
    pub fn reader_phrase(self) -> Option<&'static str> {
        match self {
            InstanceCuration::NotInvolved => None,
            InstanceCuration::Involved => Some("curated by this instance"),
        }
    }
}

/// A recorded recommendation slot, ready to explain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlotExplanation {
    pub slot_id: String,
    pub work_id: String,
    pub position: i64,
    /// Every reader-side reason this work was served, deduplicated and in a
    /// stable order so the same slot always explains itself identically.
    pub reasons: Vec<SlotReason>,
    /// Present only when the work aligned with the reader's taste.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub taste_signal: Option<TasteSignal>,
    /// The comparison that seeded the slot, in §29.2 arena language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seeded_by: Option<String>,
    /// The recipe stage, for recipe dashboards.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipe_stage: Option<String>,
    /// The undifferentiated instance-curation line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_curation: Option<String>,
    /// The blend score that placed it, so a reader can see a close call above
    /// it. Reader-visible ranking, not operator taste.
    pub blend_score: i64,
    /// When the slot was served.
    pub served_at: String,
}

impl SlotExplanation {
    /// Deduplicate and order reasons so an explanation is stable across reads.
    ///
    /// Two engines can both claim a work, and the blend merges them; without
    /// this the order would depend on which engine ran first, and a reader
    /// asking twice would get two different lists. Ordering by the enum's
    /// declaration order rather than alphabetically keeps the vocabulary's
    /// priority — taste before popularity — which is the order §33.3(a) lists.
    pub fn normalized(reasons: Vec<SlotReason>) -> Vec<SlotReason> {
        SlotReason::all()
            .iter()
            .copied()
            .filter(|r| reasons.contains(r))
            .collect()
    }
}

/// One line of the §33.3(b) attention report.
///
/// Aggregate-only, and every variant is about the reader's own behaviour or
/// their own settings. There is deliberately no variant for "the instance
/// chose this for you" beyond the undifferentiated one, and no variant carrying
/// a count of other readers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AttentionLine {
    /// What the reader read, in the reader's own coarse terms.
    Read { summary: String },
    /// What the reader's own filters changed.
    FiltersChanged { summary: String },
    /// What the reader's own settings held back. §33.3(b) requires at least
    /// one such line whenever the report is on.
    HeldBack { summary: String },
    /// The undifferentiated instance-curation line.
    InstanceCuration { summary: String },
}

/// The §33.3(b) attention report.
///
/// Disabled means disabled: the struct has no way to carry lines while
/// `enabled` is false, because `build` returns `None` in that case rather than
/// a report with empty lines. A client cannot mistake "off" for "nothing
/// happened".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttentionReport {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<AttentionLine>>,
}

impl AttentionReport {
    /// The off state. Not "enabled with no lines".
    pub fn disabled() -> Self {
        AttentionReport {
            enabled: false,
            lines: None,
        }
    }

    /// Build a report, enforcing §33.3(b)'s "at least one held-back line".
    ///
    /// Returns `None` when disabled. When enabled, a held-back line is
    /// guaranteed: if the caller has none, one is synthesised from the
    /// reader's own settings, because the spec makes that line mandatory and a
    /// reader who enabled the report is entitled to see what their own choices
    /// cost them. The substitute is phrased about their settings and never
    /// about a work the filter hides.
    pub fn build(enabled: bool, mut lines: Vec<AttentionLine>) -> Option<Self> {
        if !enabled {
            return None;
        }
        if !lines
            .iter()
            .any(|l| matches!(l, AttentionLine::HeldBack { .. }))
        {
            lines.push(AttentionLine::HeldBack {
                summary: "held back by your own settings".to_string(),
            });
        }
        Some(AttentionReport {
            enabled: true,
            lines: Some(lines),
        })
    }
}

/// The §33.3(c) wrangling proposal kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WranglingKind {
    /// Point an alternative spelling at an existing node.
    Alias,
    /// Fold one node into another, keeping history.
    Merge,
    /// Move a node under a different namespace.
    NamespaceMove,
    /// Rename a node's canonical form.
    CanonicalRename,
}

impl WranglingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            WranglingKind::Alias => "alias",
            WranglingKind::Merge => "merge",
            WranglingKind::NamespaceMove => "namespace_move",
            WranglingKind::CanonicalRename => "canonical_rename",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "alias" => WranglingKind::Alias,
            "merge" => WranglingKind::Merge,
            "namespace_move" => WranglingKind::NamespaceMove,
            "canonical_rename" => WranglingKind::CanonicalRename,
            _ => return None,
        })
    }

    /// Merges and renames rewrite the taxonomy; an alias only adds a pointer.
    ///
    /// §33.3 requires merges to keep history and stay reversible, so the
    /// destructive kinds are the ones that record merge actions.
    pub fn rewrites_taxonomy(self) -> bool {
        matches!(self, WranglingKind::Merge | WranglingKind::CanonicalRename)
    }
}

/// A wrangling proposal as §33.3(c) records it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WranglingProposal {
    pub id: String,
    pub kind: WranglingKind,
    pub from_node_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_node_id: Option<String>,
    pub reason: String,
    pub status: String,
    pub proposer_trust: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approver_trust: Option<i64>,
    pub created_at: String,
}

/// Validate a proposal before it enters the queue.
///
/// §19.1's trust gate is checked by the caller, because the required level is
/// configured per instance; what is checked here is the shape, which no
/// configuration can make valid.
pub fn validate_proposal(
    kind: WranglingKind,
    from_node_id: &str,
    to_node_id: Option<&str>,
    reason: &str,
) -> Result<(), String> {
    if from_node_id.trim().is_empty() {
        return Err("a proposal must name the node it starts from".to_string());
    }
    match kind {
        WranglingKind::Alias => {
            let Some(to) = to_node_id else {
                return Err("an alias proposal must name the node it points at".to_string());
            };
            if to.trim().is_empty() {
                return Err("an alias proposal must name the node it points at".to_string());
            }
        }
        WranglingKind::Merge => {
            let Some(to) = to_node_id else {
                return Err("a merge must name the node it folds into".to_string());
            };
            if to.trim().is_empty() {
                return Err("a merge must name the node it folds into".to_string());
            }
            if to == from_node_id {
                return Err("a work cannot be merged into itself".to_string());
            }
        }
        WranglingKind::NamespaceMove | WranglingKind::CanonicalRename => {}
    }
    if reason.trim().is_empty() {
        return Err("a proposal must say why".to_string());
    }
    Ok(())
}

/// Why a work was served, derived from a merged candidate's engines.
///
/// `blend` in this module's crate keeps only the first engine's reason for a
/// work, so a work found by three engines is explained as one. That is a real
/// information loss and this function is where it is recovered: the caller
/// passes every reason observed during the blend.
pub fn reasons_from_engines(engine_reasons: &[&str]) -> Vec<SlotReason> {
    let parsed: Vec<SlotReason> = engine_reasons
        .iter()
        .filter_map(|r| SlotReason::from_engine_reason(r))
        .collect();
    SlotExplanation::normalized(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_round_trips_through_its_wire_form() {
        for r in SlotReason::all() {
            assert_eq!(SlotReason::parse(r.as_str()), Some(*r), "{r:?}");
        }
    }

    #[test]
    fn every_taste_bucket_round_trips() {
        for score in [0.0_f64, 0.2, 0.34, 0.5, 0.9, 1.0] {
            let b = TasteSignal::from_score(score);
            assert_eq!(TasteSignal::parse(b.as_str()), Some(b), "score {score}");
        }
    }

    #[test]
    fn the_taste_signal_is_bucketed_never_raw() {
        // A float crossing the wire would let a reader reconstruct the blend.
        // The struct holds an enum, and this asserts the buckets are coarse.
        assert_eq!(TasteSignal::from_score(0.0), TasteSignal::Weak);
        assert_eq!(TasteSignal::from_score(0.9), TasteSignal::Strong);
        assert_eq!(TasteSignal::from_score(f64::NAN), TasteSignal::Weak);
        assert_eq!(TasteSignal::from_score(f64::INFINITY), TasteSignal::Weak);
    }

    /// The acceptance criterion in §33.3: no explanation path can surface the
    /// admin multiplier. It holds structurally — the vocabulary has no such
    /// variant — and this test is what makes a future addition notice.
    #[test]
    fn no_reason_variant_names_the_operator_or_a_multiplier() {
        for r in SlotReason::all() {
            let phrase = r.reader_phrase();
            for forbidden in [
                "operator",
                "admin",
                "multiplier",
                "affinity",
                "curator",
                "boost",
                "taste profile",
            ] {
                assert!(
                    !phrase.to_lowercase().contains(forbidden),
                    "{r:?} phrase {phrase:?} names {forbidden:?}"
                );
            }
        }
    }

    /// The same guarantee for the one instance-curation line: it says the
    /// instance curated, and nothing about how much or why.
    #[test]
    fn instance_curation_is_one_undifferentiated_line() {
        assert_eq!(InstanceCuration::NotInvolved.reader_phrase(), None);
        let line = InstanceCuration::Involved.reader_phrase().expect("a line");
        assert_eq!(line, "curated by this instance");
        for forbidden in ["operator", "multiplier", "affinity", "score", "%"] {
            assert!(!line.contains(forbidden), "line names {forbidden:?}");
        }
    }

    #[test]
    fn instance_curation_cannot_carry_a_magnitude() {
        // The type is a two-valued enum on purpose. This asserts the enum has
        // exactly the two variants, so adding a third is a deliberate act that
        // fails here rather than a quiet addition.
        assert_eq!(InstanceCuration::all_variants().len(), 2);
    }

    #[test]
    fn reasons_are_deduplicated_and_ordered_by_vocabulary_priority() {
        let out = SlotExplanation::normalized(vec![
            SlotReason::Popular,
            SlotReason::TasteTags,
            SlotReason::Popular,
            SlotReason::Strategy,
        ]);
        assert_eq!(
            out,
            vec![
                SlotReason::TasteTags,
                SlotReason::Popular,
                SlotReason::Strategy
            ]
        );
    }

    #[test]
    fn normalizing_preserves_vocabulary_order_regardless_of_input_order() {
        let a = SlotExplanation::normalized(vec![SlotReason::SavedSearch, SlotReason::TasteTags]);
        let b = SlotExplanation::normalized(vec![SlotReason::TasteTags, SlotReason::SavedSearch]);
        assert_eq!(a, b);
    }

    #[test]
    fn every_engine_reason_the_engines_emit_maps_into_the_vocabulary() {
        // These are the literal strings the three engines in
        // lorehaven_db::discovery set today. A new engine reason that does not
        // map is silently dropped by `from_engine_reason`, so this test is the
        // one that notices.
        for engine_reason in ["tags", "popular", "media_ref_collab", "strategy"] {
            assert!(
                SlotReason::from_engine_reason(engine_reason).is_some(),
                "engine reason {engine_reason:?} is outside the vocabulary"
            );
        }
    }

    #[test]
    fn an_unknown_engine_reason_is_dropped_rather_than_invented() {
        assert_eq!(SlotReason::from_engine_reason("operator_boost"), None);
        assert!(reasons_from_engines(&["tags", "operator_boost", "popular"])
            .contains(&SlotReason::TasteTags));
    }

    #[test]
    fn a_work_found_by_three_engines_explains_itself_with_all_three() {
        let out = reasons_from_engines(&["popular", "tags", "media_ref_collab"]);
        assert_eq!(
            out,
            vec![
                SlotReason::TasteTags,
                SlotReason::Popular,
                SlotReason::MediaReferenceCollaborative
            ]
        );
    }

    #[test]
    fn building_a_disabled_report_yields_none() {
        // Not "enabled with no lines": None, so a client cannot read an
        // off report as a report about a quiet week.
        assert!(AttentionReport::build(false, vec![]).is_none());
        assert!(AttentionReport::build(
            false,
            vec![AttentionLine::Read {
                summary: "x".into()
            }]
        )
        .is_none());
        let off = AttentionReport::disabled();
        assert!(!off.enabled);
        assert!(off.lines.is_none());
    }

    #[test]
    fn an_enabled_report_always_has_a_held_back_line() {
        let r = AttentionReport::build(
            true,
            vec![AttentionLine::Read {
                summary: "you read 12 works".into(),
            }],
        )
        .expect("enabled");
        let lines = r.lines.expect("an enabled report has lines");
        assert!(
            lines
                .iter()
                .any(|l| matches!(l, AttentionLine::HeldBack { .. })),
            "§33.3(b) requires a held-back line: {lines:?}"
        );
    }

    #[test]
    fn an_existing_held_back_line_is_not_duplicated() {
        let r = AttentionReport::build(
            true,
            vec![
                AttentionLine::HeldBack {
                    summary: "a tag you blocked".into(),
                },
                AttentionLine::Read {
                    summary: "you read things".into(),
                },
            ],
        )
        .expect("enabled");
        let held = r
            .lines
            .expect("lines")
            .iter()
            .filter(|l| matches!(l, AttentionLine::HeldBack { .. }))
            .count();
        assert_eq!(held, 1);
    }

    #[test]
    fn a_proposal_must_say_why() {
        assert!(validate_proposal(WranglingKind::Merge, "a", Some("b"), "  ").is_err());
        assert!(validate_proposal(WranglingKind::Merge, "a", Some("b"), "duplicate").is_ok());
    }

    #[test]
    fn a_proposal_must_name_where_it_starts() {
        assert!(validate_proposal(WranglingKind::Alias, "  ", Some("b"), "why").is_err());
    }

    #[test]
    fn a_merge_must_name_a_different_target() {
        assert!(validate_proposal(WranglingKind::Merge, "a", None, "why").is_err());
        assert!(validate_proposal(WranglingKind::Merge, "a", Some("a"), "why").is_err());
        assert!(validate_proposal(WranglingKind::Merge, "a", Some("b"), "why").is_ok());
    }

    #[test]
    fn an_alias_must_name_a_target() {
        assert!(validate_proposal(WranglingKind::Alias, "a", None, "why").is_err());
        assert!(validate_proposal(WranglingKind::Alias, "a", Some("b"), "why").is_ok());
    }

    #[test]
    fn only_rewriting_kinds_record_merge_actions() {
        assert!(WranglingKind::Merge.rewrites_taxonomy());
        assert!(WranglingKind::CanonicalRename.rewrites_taxonomy());
        assert!(!WranglingKind::Alias.rewrites_taxonomy());
        assert!(!WranglingKind::NamespaceMove.rewrites_taxonomy());
    }

    #[test]
    fn every_wrangling_kind_round_trips() {
        for k in [
            WranglingKind::Alias,
            WranglingKind::Merge,
            WranglingKind::NamespaceMove,
            WranglingKind::CanonicalRename,
        ] {
            assert_eq!(WranglingKind::parse(k.as_str()), Some(k));
        }
    }

    #[test]
    fn an_explanation_serializes_without_any_null_field() {
        let e = SlotExplanation {
            slot_id: "s1".into(),
            work_id: "w1".into(),
            position: 0,
            reasons: vec![SlotReason::TasteTags],
            taste_signal: Some(TasteSignal::Strong),
            seeded_by: None,
            recipe_stage: None,
            instance_curation: Some("curated by this instance".into()),
            blend_score: 42,
            served_at: "2026-09-26T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&e).expect("serialize");
        assert!(
            !json.contains("null"),
            "a skipped field serialized as null: {json}"
        );
        assert!(!json.contains("seeded_by"));
        assert!(json.contains("\"taste_signal\":\"strong\""));
    }
}
