//! M45-22 — §54's session selector, and the budget cut.
//!
//! This is **not** a second ranker, and the type design is what makes that true
//! rather than a promise in a comment. `apply_budget` takes an already-ordered
//! `ranked` slice and cuts it; it never sorts, never scores, and never asks what
//! a work is worth. §54.1's argument is that a concierge-specific ordering would
//! be a second place for §43's ordering contract to be forgotten, so there is
//! nowhere in this file for that opinion to live.
//!
//! Three rules from §54.4 shape the cut, and each exists because the alternative
//! is easy to ship by accident:
//!
//! 1. **A work with no estimate is kept and marked.** Dropping it would make the
//!    queue length depend on how complete the archive's metadata is, so a
//!    thin-metadata instance would silently serve a shorter queue for the same
//!    reader.
//! 2. **The cut is a prefix.** Two renders of one session agree, or "the
//!    20-minute queue" is not a thing the reader can have an opinion about.
//! 3. **The boundary work is included when it fits exactly.** `<=` not `<`. This
//!    is the off-by-one that decides whether "I have 20 minutes" returns the work
//!    that ends at minute 20.

use serde::{Deserialize, Serialize};

/// What the reader asked for. Both fields are optional and neither is required.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionSelector {
    /// A §15.8 mood key, or a §36.11 custom label. `None` means no mood filter,
    /// which §54.6 makes the plain §16 blend rather than an error.
    pub mood: Option<String>,
    /// Minutes the reader has. `None` means no budget, and then nothing is cut.
    pub budget_minutes: Option<u32>,
}

impl SessionSelector {
    /// The no-selector selector: the §16 blend, unfiltered.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Validate the mood against what the taxonomy actually carries.
    ///
    /// §54.2: an unrecognised mood is refused, not guessed. Falling back to the
    /// unfiltered queue would answer a different question than the one asked, and
    /// the reader would have no way to tell — which is the failure the refusal
    /// exists to prevent. `available` is the caller's list of real mood keys.
    ///
    /// The error **names the moods that exist**, because a refusal a reader
    /// cannot act on is the same defect as a bare 403.
    pub fn validate(&self, available: &[String]) -> Result<(), SessionSelectorError> {
        let Some(mood) = self.mood.as_deref() else {
            return Ok(());
        };
        let wanted = mood.trim();
        if wanted.is_empty() {
            return Err(SessionSelectorError::EmptyMood {
                available: available.to_vec(),
            });
        }
        // Case-insensitive, because §15.8's keys are author-assigned free text and
        // a reader typing "Comfort" means the same thing as "comfort". The
        // comparison is the lenient one; the value STORED is the reader's own
        // spelling, so the session records what they said.
        let matched = available
            .iter()
            .any(|m| m.trim().eq_ignore_ascii_case(wanted));
        if matched {
            Ok(())
        } else {
            Err(SessionSelectorError::UnknownMood {
                requested: mood.to_owned(),
                available: available.to_vec(),
            })
        }
    }
}

/// Why a selector was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionSelectorError {
    /// The mood was blank. Distinct from unknown: "here are the moods we have"
    /// is no help for an empty string.
    EmptyMood { available: Vec<String> },
    /// The taxonomy carries no such mood.
    UnknownMood {
        requested: String,
        available: Vec<String>,
    },
}

impl SessionSelectorError {
    /// The message a reader sees.
    ///
    /// Carries the available moods in both arms. The empty case could get away
    /// without them, and a caller that forgets to append them is the kind of
    /// omission that only shows up in production, so the list is not optional in
    /// one arm and required in the other.
    #[must_use]
    pub fn message(&self) -> String {
        let list = match self {
            Self::EmptyMood { available } | Self::UnknownMood { available, .. } => available,
        };
        let available = if list.is_empty() {
            "this instance carries no moods yet".to_owned()
        } else {
            format!("the moods on this instance are: {}", list.join(", "))
        };
        match self {
            Self::EmptyMood { .. } => {
                format!("a mood was given but it was blank; {available}")
            }
            Self::UnknownMood { requested, .. } => {
                format!("no work on this instance carries the mood {requested:?}; {available}")
            }
        }
    }
}

/// Why a queue item is in it — §50's `reason` field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueueReason {
    /// Selected by the mood the reader named.
    Mood { mood: String },
    /// Came back from the §16 blend with no mood selector, or carrying no mood
    /// the reader named.
    Blend,
    /// Included despite having no duration estimate (§54.4's `duration_unknown`).
    ///
    /// A *reason* rather than a flag on the item, because a reader told "comfort"
    /// and shown a work with `reason: Blend` can see the mood did not do what they
    /// asked. §50's transparency rule applied to the queue itself.
    DurationUnknown,
}

/// One entry in the queue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueueItem {
    pub work_id: String,
    pub reason: QueueReason,
    /// Minutes estimated for this work. `None` when unknown, which §54.4 says is
    /// rendered rather than dropped.
    pub estimated_minutes: Option<f64>,
}

/// Which reading speed the estimate used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateSource {
    /// From §36.11's progress sync — the reader has an observation.
    Observed,
    /// The instance default, because the reader has no observation yet.
    ///
    /// Stated rather than inferred: §54.4 says guessing a personal rate is not
    /// available without an observation, and inventing one would make the same
    /// request mean different things on a reader's first day and their fortieth.
    Default,
}

/// A rendered queue, and the facts a reader is entitled to about how it was made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConciergeQueue {
    pub session_id: String,
    pub items: Vec<QueueItem>,
    /// Total estimated minutes of the returned items.
    pub estimated_minutes: f64,
    /// Index the budget bound at. `None` when nothing was cut.
    ///
    /// `None` and `Some(0)` are different facts — "everything fit" against
    /// "nothing fit" — and a reader who asked for zero minutes deserves to be
    /// told so rather than handed an empty list with no explanation.
    pub truncated_at: Option<usize>,
    /// Which rate the estimate used.
    pub rate_source: RateSource,
    /// Whether the queue is empty because a selector matched nothing, rather than
    /// because there is nothing to show (§54.6's explained empty queue).
    ///
    /// `None` when the queue is not empty. An empty queue with a selector is an
    /// answer; an empty queue with no selector is a fact about the archive.
    pub explained_empty: Option<String>,
}

impl ConciergeQueue {
    /// An empty queue carrying an explanation, for §54.6's "matches nothing is
    /// not a fallback".
    #[must_use]
    pub fn explained_empty(session_id: &str, why: impl Into<String>, rate: RateSource) -> Self {
        Self {
            session_id: session_id.to_owned(),
            items: Vec::new(),
            estimated_minutes: 0.0,
            truncated_at: None,
            rate_source: rate,
            explained_empty: Some(why.into()),
        }
    }

    /// Whether anything was cut (§54.4's third bullet).
    #[must_use]
    pub fn was_truncated(&self) -> bool {
        self.truncated_at.is_some()
    }
}

/// Cut `ranked` to `budget_minutes`, in order (§54.4).
///
/// `ranked` is `(work_id, estimated_minutes)` in **blend order** — this function
/// does not reorder it, which is §54.1's "time constrains the tail, never the
/// ranking".
///
/// Returns the items, the index the budget bound at, and the total estimated
/// minutes of what was returned.
///
/// `budget_minutes: None` cuts nothing and returns `truncated_at: None`. `Some(0.0)`
/// returns an empty queue with `truncated_at: Some(0)` — not a fallback to the
/// unfiltered queue, and not an error.
///
/// The estimate for an unknown-duration work is the midpoint of the queue's own
/// estimate range, so a budget is still applied to it (§54.4) rather than the work
/// being excluded. The range comes from the items actually present, which means a
/// queue where *everything* is unknown has no range and falls back to a duration
/// of zero — recorded in the doc comment on [`midpoint_estimate`] because it is the
/// one case where the mark is the whole of the item's meaning.
#[must_use]
pub fn apply_budget(
    ranked: &[(String, Option<f64>)],
    budget_minutes: Option<f64>,
) -> (Vec<QueueItem>, Option<usize>, f64) {
    let midpoint = midpoint_estimate(ranked);

    let mut items: Vec<QueueItem> = Vec::with_capacity(ranked.len());
    let mut total = 0.0_f64;
    // The index the budget bound at, and the index of the first item that did not
    // fit. `None` until one does not.
    let mut truncated_at: Option<usize> = None;

    for (index, (work_id, estimate)) in ranked.iter().enumerate() {
        let minutes = estimate.unwrap_or(midpoint);
        let fits = budget_minutes.is_none_or(|budget| total + minutes <= budget);

        if !fits {
            // First item that does not fit ends the queue, and the queue is a
            // PREFIX: `break`, not `continue`. Continuing would re-introduce order
            // as a selection criterion — a shorter work later in the ranking would
            // squeeze past a longer one that came first, and §54.4's determinism
            // would be gone.
            truncated_at = Some(index);
            break;
        }

        total += minutes;
        items.push(QueueItem {
            work_id: work_id.clone(),
            reason: if estimate.is_some() {
                QueueReason::Blend
            } else {
                QueueReason::DurationUnknown
            },
            estimated_minutes: *estimate,
        });
    }

    (items, truncated_at, total)
}

/// The estimate to charge a work of unknown duration: the midpoint of the queue's
/// own range (§54.4's "placed at the natural midpoint of the queue's estimate
/// range").
///
/// Zero when every item is unknown or the list is empty — there is no range to
/// take a midpoint of. That is not a fallback to any rate: it means the budget
/// could not be applied to an unknown-duration work, and the item is marked
/// `duration_unknown` so the reader can see that.
#[must_use]
pub fn midpoint_estimate(ranked: &[(String, Option<f64>)]) -> f64 {
    let known: Vec<f64> = ranked.iter().filter_map(|(_, m)| *m).collect();
    if known.is_empty() {
        return 0.0;
    }
    let min = known.iter().copied().fold(f64::INFINITY, f64::min);
    let max = known.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (min + max) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(ids: &[&str], minutes: &[Option<f64>]) -> Vec<(String, Option<f64>)> {
        ids.iter()
            .zip(minutes)
            .map(|(id, m)| ((*id).to_owned(), *m))
            .collect()
    }

    fn moods(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    // -- the selector --------------------------------------------------------

    #[test]
    fn an_unknown_mood_is_refused_by_name_and_lists_what_exists() {
        let selector = SessionSelector {
            mood: Some("catharsis".to_owned()),
            budget_minutes: None,
        };
        let available = moods(&["comfort", "wistful", "catharsis-adjacent"]);

        let error = selector
            .validate(&available)
            .expect_err("catharsis is not on the list");

        match &error {
            SessionSelectorError::UnknownMood { requested, .. } => {
                assert_eq!(requested, "catharsis");
            }
            other => panic!("expected UnknownMood, got {other:?}"),
        }
        // §54.2's rule is that the response names the moods the taxonomy has. An
        // error a reader cannot act on is the defect the refusal exists to avoid.
        let message = error.message();
        assert!(message.contains("catharsis"), "{message}");
        assert!(message.contains("comfort"), "{message}");
        assert!(message.contains("wistful"), "{message}");
    }

    #[test]
    fn a_mood_the_taxonomy_carries_is_accepted_case_insensitively() {
        let selector = SessionSelector {
            mood: Some("  Comfort ".to_owned()),
            budget_minutes: None,
        };
        assert_eq!(selector.validate(&moods(&["comfort"])), Ok(()));
    }

    #[test]
    fn no_selector_is_never_an_error() {
        // §54.6: an empty intent is the plain §16 blend.
        assert_eq!(
            SessionSelector::none().validate(&moods(&["comfort"])),
            Ok(())
        );
        assert_eq!(SessionSelector::none().validate(&[]), Ok(()));
    }

    #[test]
    fn a_blank_mood_is_its_own_refusal_not_an_unknown_one() {
        // "here are the moods we have" is no help for an empty string, so the two
        // are separate variants rather than one with a special case in the message.
        let selector = SessionSelector {
            mood: Some("   ".to_owned()),
            budget_minutes: None,
        };
        assert!(matches!(
            selector.validate(&moods(&["comfort"])),
            Err(SessionSelectorError::EmptyMood { .. })
        ));
    }

    // -- the budget ----------------------------------------------------------

    #[test]
    fn no_budget_cuts_nothing() {
        let ranked = q(&["a", "b", "c"], &[Some(90.0), Some(30.0), Some(45.0)]);
        let (items, truncated_at, total) = apply_budget(&ranked, None);
        assert_eq!(items.len(), 3);
        assert_eq!(truncated_at, None, "no budget means nothing was cut");
        assert!((total - 165.0).abs() < f64::EPSILON, "{total}");
    }

    #[test]
    fn the_boundary_work_is_included_when_it_fits_exactly() {
        // `<=` not `<`. This decides whether "I have 20 minutes" returns the work
        // that ends at minute 20, which is the reader's own reading of the number.
        let ranked = q(&["a", "b"], &[Some(12.0), Some(8.0)]);
        let (items, truncated_at, total) = apply_budget(&ranked, Some(20.0));
        assert_eq!(items.len(), 2, "{items:?}");
        assert_eq!(truncated_at, None);
        assert!((total - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn the_cut_is_a_prefix_and_names_the_index() {
        let ranked = q(
            &["a", "b", "c", "d"],
            &[Some(10.0), Some(10.0), Some(10.0), Some(10.0)],
        );
        let (items, truncated_at, total) = apply_budget(&ranked, Some(25.0));

        let ids: Vec<&str> = items.iter().map(|i| i.work_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"], "a prefix of the blend, in order");
        assert_eq!(truncated_at, Some(2), "the index the budget bound at");
        assert!((total - 20.0).abs() < f64::EPSILON, "{total}");
    }

    #[test]
    fn a_short_work_later_in_the_ranking_cannot_squeeze_past_a_longer_one_before_it() {
        // §54.4's determinism. A `continue`-instead-of-`break` cut passes the
        // prefix test above and fails this one, which is why both exist: the
        // budget is a TAIL operation and may not become an eligibility filter
        // (§54.2's table).
        let ranked = q(&["a", "b", "c"], &[Some(50.0), Some(50.0), Some(1.0)]);
        let (items, truncated_at, _) = apply_budget(&ranked, Some(60.0));

        let ids: Vec<&str> = items.iter().map(|i| i.work_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["a"],
            "`c` is one minute and would fit, and must not appear: {items:?}"
        );
        assert_eq!(truncated_at, Some(1));
    }

    #[test]
    fn a_zero_budget_returns_an_empty_queue_with_a_reason() {
        let ranked = q(&["a", "b"], &[Some(10.0), Some(10.0)]);
        let (items, truncated_at, total) = apply_budget(&ranked, Some(0.0));
        assert!(items.is_empty(), "{items:?}");
        // §54.4: "cut at index 0" and "nothing was cut" are different facts.
        assert_eq!(
            truncated_at,
            Some(0),
            "a zero budget cut everything, including the first"
        );
        assert_eq!(total, 0.0);
    }

    #[test]
    fn a_work_with_no_estimate_is_kept_and_marked() {
        // The budget covers all three, so the assertion is about MEMBERSHIP and the
        // mark — not about how many happen to fit. The first version used
        // `[10, None, 90]` with a 100-minute budget and asserted three items; it
        // failed, and correctly: 10 + the 50 midpoint = 60, so the third work's 90
        // crosses 100 and the cut is right. The arithmetic was wrong, not the code.
        let ranked = q(&["a", "b", "c"], &[Some(10.0), None, Some(30.0)]);
        let (items, truncated_at, _) = apply_budget(&ranked, Some(100.0));

        assert_eq!(
            items.len(),
            3,
            "§54.4: an unknown duration is not an exclusion: {items:?}"
        );
        assert_eq!(truncated_at, None);
        assert_eq!(
            items[1].reason,
            QueueReason::DurationUnknown,
            "and it must say so, or the reader cannot tell why it was included"
        );
        assert_eq!(items[1].estimated_minutes, None);
        // The known-duration items keep the Blend reason, so a report can tell
        // the two apart rather than marking everything unknown.
        assert_eq!(items[0].reason, QueueReason::Blend);
        assert_eq!(items[2].reason, QueueReason::Blend);
    }

    #[test]
    fn an_unknown_duration_work_is_charged_the_midpoint_of_the_range() {
        // Known range is 10..90, so the midpoint is 50. The queue's total must
        // include the unknown work's charge, or the budget is not being applied to
        // it at all.
        let ranked = q(&["a", "b", "c"], &[Some(10.0), None, Some(90.0)]);
        let (items, truncated_at, total) = apply_budget(&ranked, Some(100.0));

        assert!(
            (total - 60.0).abs() < 1e-9,
            "10 + 50 + (cut at 90): {total}"
        );
        assert_eq!(truncated_at, Some(2), "the third work's 90 would cross 100");
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn a_queue_of_only_unknown_durations_has_no_range_to_midpoint() {
        // Every item unknown: there is no range, so the midpoint is zero and the
        // mark is the whole of the item's meaning. Asserted so the zero is a
        // decision rather than an accident waiting for a fixture to hide it.
        let ranked = q(&["a", "b"], &[None, None]);
        assert_eq!(midpoint_estimate(&ranked), 0.0);
        let (items, truncated_at, _) = apply_budget(&ranked, Some(5.0));
        assert_eq!(items.len(), 2, "a zero charge fits any budget: {items:?}");
        assert_eq!(truncated_at, None);
        assert!(items
            .iter()
            .all(|i| i.reason == QueueReason::DurationUnknown));
    }

    #[test]
    fn two_renders_of_one_session_are_identical() {
        // §54.7: two renders with no intervening writes return identical lists. The
        // function is pure, so this asserts the property the caller depends on
        // rather than the purity itself.
        let ranked = q(
            &["a", "b", "c", "d", "e"],
            &[Some(12.0), None, Some(7.5), Some(30.0), Some(4.0)],
        );
        let first = apply_budget(&ranked, Some(40.0));
        let second = apply_budget(&ranked, Some(40.0));
        assert_eq!(first, second, "{first:?} vs {second:?}");
    }

    #[test]
    fn an_empty_ranking_produces_an_empty_queue_and_cuts_nothing() {
        let (items, truncated_at, total) = apply_budget(&[], Some(20.0));
        assert!(items.is_empty());
        // Nothing was cut, because nothing was there to cut. `Some(0)` would say
        // the budget excluded the first work, which is a different fact and a
        // misleading one.
        assert_eq!(truncated_at, None);
        assert_eq!(total, 0.0);
    }

    #[test]
    fn an_explained_empty_queue_is_distinguishable_from_a_cut_one() {
        let matched_nothing = ConciergeQueue::explained_empty(
            "s1",
            "no work on this instance carries the mood \"catharsis\"",
            RateSource::Default,
        );
        assert_eq!(matched_nothing.items.len(), 0);
        assert!(
            !matched_nothing.was_truncated(),
            "a selector matching nothing is not a cut"
        );

        // The EXPLANATION must survive, not merely be `Some`. Measured: a mutation
        // that discarded the `why` argument kept every test green, because the only
        // assertion was on the `Option` being present. `Some(None)` is the shape
        // that bug produces — a queue that claims to be explained and says nothing.
        assert_eq!(
            matched_nothing.explained_empty.as_deref(),
            Some("no work on this instance carries the mood \"catharsis\""),
            "§54.6's explained empty queue has to say WHAT matched nothing"
        );
        assert_eq!(matched_nothing.rate_source, RateSource::Default);

        // And a cut-to-nothing queue says nothing of the kind.
        let cut = ConciergeQueue {
            session_id: "s2".to_owned(),
            items: Vec::new(),
            estimated_minutes: 0.0,
            truncated_at: Some(0),
            rate_source: RateSource::Default,
            explained_empty: None,
        };
        assert!(cut.explained_empty.is_none());
        assert!(cut.was_truncated());
        // The two are different facts about different causes, so neither may be
        // readable as the other.
        assert_ne!(
            matched_nothing.truncated_at, cut.truncated_at,
            "a selector that matched nothing and a budget that cut everything are \
             different answers and must not serialize alike"
        );
    }

    #[test]
    fn the_rate_source_is_stated_and_not_guessed() {
        // §54.4 forbids inventing a personal rate. The type carries the two
        // possibilities and nothing else, so "did we guess" has no third answer.
        let observed: RateSource = RateSource::Observed;
        assert_eq!(observed, RateSource::Observed);
        assert_ne!(RateSource::Observed, RateSource::Default);
        // And it survives the wire, because a client renders it.
        let json = serde_json::to_string(&observed).expect("serialize");
        assert_eq!(json, "\"observed\"");
    }
}
