//! The capability registry and the gate that guards it.
//!
//! The design principle this file exists to enforce: a dashboard renders by
//! iterating the capabilities a viewer is *allowed*, never by iterating
//! metrics and filtering them. The difference is that an unimplemented
//! capability is then invisible rather than an unfiltered leak, and a test can
//! assert the whole surface without reading any UI code.
//!
//! Every test here is a privacy claim. If one fails, something is showing a
//! reader data they should not see.

use lorehaven_domain::analytics::*;

// --- the gate ----------------------------------------------------------------

#[test]
fn tl0_sees_its_own_basics_and_the_public_counters() {
    assert!(allowed(Scope::OwnReadingBasic, TL_NEW));
    assert!(allowed(Scope::OwnWorkBasic, TL_NEW));
    assert!(allowed(Scope::PublicWorkBasic, TL_NEW));
    assert!(allowed(Scope::PublicInstanceCounts, TL_NEW));
}

#[test]
fn tl0_does_not_see_anyone_elses_numbers() {
    // The whole point of the ladder. A new account has the same public view as
    // everyone and no comparative context.
    for scope in [Scope::CommunityTrending, Scope::CommunityFandomDash] {
        assert!(!allowed(scope, TL_NEW), "{scope:?} leaked to TL0");
    }
    assert!(!allowed(Scope::OwnReadingDistribution, TL_NEW));
    assert!(!allowed(Scope::OwnWorkRetention, TL_NEW));
}

#[test]
fn depth_increases_with_trust_and_never_reverses() {
    // Monotonicity is a real property worth testing rather than trusting: a
    // capability granted at TL2 and withheld at TL3 would be a bug that reads
    // as a feature in a spot check.
    for scope in ALL_SCOPES {
        // `allowed` before the first level is the negative case: nothing is
        // allowed below level 0, which is what makes the loop start clean.
        assert!(!allowed(*scope, -1), "{scope:?} is allowed below TL0");
        for level in 0..=TRUST_MAX {
            let now = allowed(*scope, level);
            let before = if level == 0 {
                false
            } else {
                allowed(*scope, level - 1)
            };
            assert!(
                !before || now,
                "{scope:?} is allowed at TL{} but not TL{level}",
                level - 1
            );
        }
    }
}

#[test]
fn an_author_own_work_capability_never_appears_at_a_lower_level() {
    // Retention and mood inference are author-facing but trust-gated, because
    // a reader who has not been reviewed is a reader who can be a fan of a
    // number rather than a reader of a chart.
    assert!(!allowed(Scope::OwnWorkRetention, TL_NEW));
    assert!(allowed(Scope::OwnWorkRetention, TL_ESTABLISHED));
    assert!(allowed(Scope::OwnWorkMood, TL_REGULAR));
    assert!(!allowed(Scope::OwnWorkMood, TL_ESTABLISHED));
}

#[test]
fn operational_capabilities_require_their_role_and_not_merely_a_level() {
    // TL6 is a trustee; an ordinary account at level 6 obtained through a
    // mistake should still not see the financial dashboard. This is why the
    // gate takes a role as well as a level.
    assert!(!allowed_with_role(
        Scope::TrusteeFinancials,
        TL_TRUSTEE,
        Role::Reader,
        Preset::Archive
    ));
    assert!(allowed_with_role(
        Scope::TrusteeFinancials,
        TL_TRUSTEE,
        Role::Trustee,
        Preset::Archive
    ));
}

#[test]
fn admin_capabilities_need_the_role_and_the_level_together() {
    // A role without a level is a misconfiguration; a level without a role is
    // a reader. Requiring both is what makes the admin panel a role grant
    // rather than a score.
    assert!(!allowed_with_role(
        Scope::AdminTasteProfile,
        TL_TRUSTEE,
        Role::Trustee,
        Preset::Archive
    ));
    assert!(!allowed_with_role(
        Scope::AdminTasteProfile,
        0,
        Role::Admin,
        Preset::Archive
    ));
    assert!(allowed_with_role(
        Scope::AdminTasteProfile,
        TRUST_MAX,
        Role::Admin,
        Preset::Archive
    ));
}

#[test]
fn an_admin_also_gets_everything_a_reader_gets() {
    // Otherwise every route needs an `|| is_admin`, and the first one that
    // forgets is the leak.
    for scope in ALL_SCOPES {
        if matches!(scope.authority(), Authority::Admin) {
            continue;
        }
        assert!(
            allowed_with_role(*scope, TRUST_MAX, Role::Admin, Preset::Archive),
            "{scope:?} is withheld from an admin"
        );
    }
}

// --- the k-anonymity floor ----------------------------------------------------

#[test]
fn the_floor_is_ten_for_anything_about_other_people() {
    // §36.12's value. Five is the floor for your own data and is not enough
    // for someone else's: a reader whose dashboard shows a six-person
    // breakdown must not meet the same six people as author analytics.
    assert_eq!(floor_for(Subject::Other), 10);
    assert_eq!(floor_for(Subject::Self_), 5);
}

#[test]
fn a_count_below_the_floor_is_coarsened_to_a_true_statement() {
    // "Between 1 and 9" is a range a reader can narrow by asking again. "Fewer
    // than 10" is a floor, and the floor is what is actually true.
    let shown = coarsen(7, floor_for(Subject::Other));
    assert_eq!(shown, Coarsened::Below(10));
    assert!(!shown.is_numeric());
}

#[test]
fn a_count_at_or_above_the_floor_is_exact() {
    assert_eq!(coarsen(10, floor_for(Subject::Other)), Coarsened::Exact(10));
    assert_eq!(
        coarsen(10_000, floor_for(Subject::Other)),
        Coarsened::Exact(10_000)
    );
    assert_eq!(coarsen(5, floor_for(Subject::Self_)), Coarsened::Exact(5));
}

#[test]
fn coarsening_is_monotone_so_repeated_queries_cannot_narrow_it() {
    // Asking twice must not leak more than asking once. Every count below the
    // floor produces the identical answer, so there is nothing to average.
    let floor = floor_for(Subject::Other);
    assert_eq!(coarsen(1, floor), coarsen(9, floor));
    assert_eq!(coarsen(0, floor), coarsen(9, floor));
}

// --- the forbidden set -------------------------------------------------------

#[test]
fn the_forbidden_scopes_do_not_exist() {
    // The source document's "never shown" list is a list of claims about
    // behaviour. This makes it a list of facts: these names are absent from
    // the registry, so code referencing one does not compile, and this test
    // fails if anyone adds one.
    for (name, clause) in FORBIDDEN_SCOPE_NAMES {
        assert!(
            ALL_SCOPES.iter().all(|s| s.as_str() != *name),
            "{name} ({clause}) must not be a capability"
        );
    }
}

#[test]
fn the_forbidden_names_cover_the_specs_own_rules() {
    // Each of these is a specific clause. If the list shrinks, the test
    // fails -- which is the point: dropping one is a decision someone has to
    // make deliberately, in a diff, rather than by omission.
    for (name, clause) in [
        ("ab.variant_assignment", "§24.6"),
        ("ab.signal_weights", "§0.3, §9.7.1"),
        ("ab.resonance_numeric", "own-only, label form only"),
        ("ab.pseud_linkage", "§7.2"),
        ("ab.other_reading_history", "§9.5"),
        ("ab.other_credit_balance", "§20.1"),
        ("ab.other_earnings", "aggregate only"),
        ("ab.shadowban_state", "§19.6"),
        ("ab.vanguard_reason", "§16.18"),
        ("ab.session_identifiers", "§24.3"),
        ("ab.feature_flags", "instance config"),
        ("ab.individual_queries", "§24.3"),
        ("ab.comment_scores", "aggregate performance only"),
    ] {
        let entry = FORBIDDEN_SCOPE_NAMES
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap_or_else(|| panic!("{name} ({clause}) dropped from the forbidden list"));
        assert_eq!(entry.1, clause, "{name} changed its justification");
    }
}

#[test]
fn no_capability_name_starts_with_the_forbidden_prefix() {
    // A prefix is the cheapest way to make a new leak structurally impossible
    // rather than merely unlisted.
    for scope in ALL_SCOPES {
        assert!(
            !scope.as_str().starts_with("ab."),
            "{scope:?} is in the forbidden namespace"
        );
    }
}

// --- the registry ------------------------------------------------------------

#[test]
fn every_capability_has_a_minimum_trust_level() {
    for scope in ALL_SCOPES {
        assert!(
            (0..=TRUST_MAX).contains(&scope.minimum_trust()),
            "{scope:?} names a level outside the ladder"
        );
    }
}

#[test]
fn every_capability_has_a_computed_method_that_names_its_denominator() {
    // §24.2: each metric documents its definition. A metric whose method does
    // not name what it counts is not a definition.
    for scope in ALL_SCOPES {
        let method = scope.method();
        assert!(!method.definition.is_empty(), "{scope:?} has no definition");
        assert!(
            method.definition.chars().count() >= 20,
            "{scope:?}'s definition is not a definition: {:?}",
            method.definition
        );
        assert!(
            !method.freshness.as_str().is_empty(),
            "{scope:?} does not say how fresh it is"
        );
        assert!(
            !method.approximation.is_empty(),
            "{scope:?} does not say whether it is estimated"
        );
    }
}

#[test]
fn a_capability_about_other_people_cannot_use_the_self_floor() {
    // The two floors only differ because the two subjects differ. A capability
    // about someone else that resolves to the self floor would hand a reader
    // another person's five-person breakdown.
    for scope in ALL_SCOPES {
        if scope.subject() == Subject::Other {
            assert_ne!(
                scope.floor(),
                Some(5),
                "{scope:?} about other people uses the self floor"
            );
        }
    }
}

#[test]
fn a_ttl_is_required_for_everything_marked_near_real_time() {
    // `freshness` promising minutes while `requires_ttl` is false would let a
    // cache serve a personal number to the wrong reader.
    for scope in ALL_SCOPES {
        if scope.method().freshness == Freshness::NearRealTime && !scope.is_personal() {
            assert!(
                scope.requires_ttl(),
                "{scope:?} is near-real-time and cacheable"
            );
        }
    }
}

#[test]
fn the_registry_is_ordered_so_a_client_can_render_without_sorting() {
    // The three assertions below assume a stable order. Sorting in the client
    // means two clients can disagree about what a capability list is.
    let levels: Vec<i64> = ALL_SCOPES.iter().map(|s| s.minimum_trust()).collect();
    let mut sorted = levels.clone();
    sorted.sort_unstable();
    assert_eq!(levels, sorted, "the registry is not in trust order");
}

// --- instance presets --------------------------------------------------------

#[test]
fn a_preset_can_only_remove_capabilities() {
    // `sandbox` is documented as "everything visible". If a preset could add,
    // it would be a way to bypass the ladder, and the trust floor would be
    // advisory.
    for scope in ALL_SCOPES {
        if matches!(scope.authority(), Authority::Admin) {
            continue;
        }
        assert!(
            allowed_under(*scope, TL_TRUSTEE, Role::Trustee, Preset::Sandbox),
            "{scope:?} is withheld by the most permissive preset"
        );
    }
}

#[test]
fn gallery_hides_the_community_surface() {
    // A curated instance does not want a trending dashboard on its front page.
    // The personal surface stays, because a reader's own history is theirs.
    assert!(!allowed_under(
        Scope::CommunityTrending,
        TL_TRUSTEE,
        Role::Trustee,
        Preset::Gallery
    ));
    assert!(!allowed_under(
        Scope::CommunityFandomDash,
        TL_TRUSTEE,
        Role::Trustee,
        Preset::Gallery
    ));
    assert!(allowed_under(
        Scope::OwnReadingBasic,
        TL_TRUSTEE,
        Role::Trustee,
        Preset::Gallery
    ));
    assert!(allowed_under(
        Scope::PublicWorkBasic,
        TL_TRUSTEE,
        Role::Trustee,
        Preset::Gallery
    ));
}

#[test]
fn showcase_keeps_author_analytics_and_drops_the_community_dashboards() {
    // An instance that exists to showcase authors wants the author dashboard
    // and not a per-fandom activity feed.
    assert!(allowed_under(
        Scope::OwnWorkRetention,
        TL_ESTABLISHED,
        Role::Reader,
        Preset::Showcase
    ));
    assert!(!allowed_under(
        Scope::CommunityFandomDash,
        TL_TRUSTEE,
        Role::Trustee,
        Preset::Showcase
    ));
}

#[test]
fn a_preset_never_raises_a_trust_ceiling() {
    // The dangerous direction. `commons` widens who sees a *lower* trust level's
    // capabilities; it must not let TL0 see what needs TL3.
    assert!(!allowed_under(
        Scope::OwnWorkRetention,
        TL_NEW,
        Role::Reader,
        Preset::Commons
    ));
    assert!(!allowed_under(
        Scope::OwnWorkMood,
        TL_ESTABLISHED,
        Role::Reader,
        Preset::Commons
    ));
    assert!(allowed_under(
        Scope::OwnWorkMood,
        TL_REVIEWED,
        Role::Reader,
        Preset::Commons
    ));
}
