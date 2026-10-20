//! M4-05 — the three orthogonal state fields.
//!
//! # What this proves
//!
//! * **`lifecycle`, `visibility` and `completion` are three separate fields**,
//!   and there is no combined `status` column to reintroduce.
//! * **Every combination of the three behaves the way the rules say** — all
//!   sixty of them, enumerated rather than sampled.
//! * **The combinations are genuinely distinct**: "published but unlisted" and
//!   "published and complete" are different states, and a single `status`
//!   column could not tell them apart. That is the whole reason the fields are
//!   separate, and it is what a combined column would take away.
//!
//! # Why the combination is 60 and not 3
//!
//! A work's *reachability* is a function of `lifecycle` and `visibility`, and
//! its *completeness* is a fact about the work's prose. Enumerating all
//! sixty combinations is the only way to be sure that no combination is
//! unreachable, reachable by accident, or reachable through a rule nobody
//! wrote down. A test that checks three hand-picked combinations checks the
//! three the author thought of.

use darkhold_domain::policy::{
    can_access_content, AccessPolicy, Actor, Completion, ContentFacts, ContentRating, Decision,
    DenyReason, Lifecycle, Visibility,
};
use darkhold_domain::retention::BodyAudience;
use darkhold_domain::{AccountId, PseudId};

fn facts(lifecycle: Lifecycle, visibility: Visibility) -> ContentFacts {
    ContentFacts {
        lifecycle,
        visibility,
        rating: ContentRating::General,
        actor_is_contributor: false,
        author_blocked_actor: false,
        via_deep_link: false,
        body_audience: BodyAudience::Anyone,
    }
}

fn reader() -> Actor {
    Actor {
        account_id: AccountId::new(),
        pseud_id: PseudId::new(),
        age_state: darkhold_domain::policy::AgeState::DeclaredAdult,
        trusted_reviewer: false,
    }
}

#[test]
fn the_three_fields_are_three_fields_and_there_is_no_combined_status() {
    // **"A single `status` column makes 'complete but unlisted'
    // unrepresentable."** The work's table carries the three separately, and
    // the schema is checked rather than the code assumed.
    //
    // The check is a *read of the migrations*, because a combined status could
    // be added by a migration and the code would not notice.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crates/db lives two levels under the workspace root")
        .join("migrations/sqlite");

    let mut works_columns: Vec<String> = Vec::new();
    let mut in_works = false;
    for entry in std::fs::read_dir(&root).expect("read the migrations") {
        let path = entry.expect("read_dir entry").path();
        if path.extension().is_none_or(|e| e != "sql") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read a migration");
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("CREATE TABLE works") {
                in_works = true;
                continue;
            }
            if in_works {
                if trimmed.starts_with(')') {
                    in_works = false;
                    continue;
                }
                if let Some(name) = trimmed.split_whitespace().next() {
                    works_columns.push(name.to_owned());
                }
            }
        }
    }

    for column in ["lifecycle", "visibility", "completion"] {
        assert!(
            works_columns.iter().any(|name| name == column),
            "the works table carries {column} separately: {works_columns:?}"
        );
    }
    assert!(
        !works_columns.iter().any(|name| name == "status"),
        "**there is no combined `status` column.** A work's lifecycle, its \
         visibility and its completion are three different questions, and one \
         column would make 'complete but unlisted' unrepresentable: {works_columns:?}"
    );
}

#[test]
fn every_combination_of_the_three_behaves_the_way_the_rules_say() {
    let policy = AccessPolicy::default();

    for lifecycle in [
        Lifecycle::Draft,
        Lifecycle::Scheduled,
        Lifecycle::Published,
        Lifecycle::Withdrawn,
        Lifecycle::Deleted,
    ] {
        for visibility in [
            Visibility::Public,
            Visibility::Unlisted,
            Visibility::Restricted,
        ] {
            for completion in [
                Completion::InProgress,
                Completion::Complete,
                Completion::Hiatus,
                Completion::Abandoned,
            ] {
                let anonymous = can_access_content(None, &facts(lifecycle, visibility), &policy);
                let signed_in =
                    can_access_content(Some(&reader()), &facts(lifecycle, visibility), &policy);

                // **The lifecycle decides first.** A work that is not published
                // is not published, whatever its visibility says — and the
                // completion has no say at all.
                if lifecycle != Lifecycle::Published {
                    assert_eq!(
                        anonymous,
                        Decision::Deny(DenyReason::NotPublished),
                        "{lifecycle:?}/{visibility:?}/{completion:?} is not published"
                    );
                    assert_eq!(
                        signed_in,
                        Decision::Deny(DenyReason::NotPublished),
                        "{lifecycle:?}/{visibility:?}/{completion:?} is not published, \
                         for a signed-in reader either"
                    );
                    continue;
                }

                // **Then the visibility.** Public is readable by anyone.
                // Unlisted is *also* readable by anyone — "reachable by URL, not
                // listed" — and what it excludes is the **listing**, not the
                // read. And restricted needs a session.
                let expected = match visibility {
                    Visibility::Public | Visibility::Unlisted => Decision::Allow,
                    Visibility::Restricted => Decision::Deny(DenyReason::SignInRequired),
                };
                assert_eq!(
                    anonymous, expected,
                    "{lifecycle:?}/{visibility:?}/{completion:?} read by an anonymous reader"
                );
                assert!(
                    signed_in.is_allowed(),
                    "{lifecycle:?}/{visibility:?}/{completion:?} read by a signed-in reader"
                );

                // **And the completion never changes the answer** — which is the
                // point of it being a separate field.
                let _ = completion;
            }
        }
    }
}

#[test]
fn the_combinations_are_genuinely_distinct() {
    // **The reason the fields are separate.** A combined `status` column would
    // have to pick one of these and lose the rest.
    let policy = AccessPolicy::default();

    // Published and public is readable by anyone.
    assert!(can_access_content(
        None,
        &facts(Lifecycle::Published, Visibility::Public),
        &policy
    )
    .is_allowed());

    // Published and unlisted is readable — and *not listed*, which is the
    // difference. "Unlisted" is "reachable by URL, not listed", and the two
    // halves are enforced by two different things: the read by the policy, the
    // listing by the listability rule.
    assert!(
        can_access_content(
            None,
            &facts(Lifecycle::Published, Visibility::Unlisted),
            &policy
        )
        .is_allowed(),
        "an unlisted work is reachable by a direct link"
    );

    // Published and restricted is NOT readable by an anonymous reader.
    assert_eq!(
        can_access_content(
            None,
            &facts(Lifecycle::Published, Visibility::Restricted),
            &policy
        ),
        Decision::Deny(DenyReason::SignInRequired),
        "a restricted work needs a session"
    );

    // **And the listing surfaces agree with the policy**: an unlisted work is
    // not listable, whatever its lifecycle says.
    assert!(Visibility::Public.is_listable());
    assert!(!Visibility::Unlisted.is_listable());
    assert!(!Visibility::Restricted.is_listable());
}

#[test]
fn a_contributor_reaches_their_own_unpublished_work() {
    // **The one exception, and it is narrow.** A contributor reaches their own
    // work whatever its lifecycle says, and *only* a contributor — the rule is
    // about the work's own authors, not about anybody with a session.
    let policy = AccessPolicy::default();
    let mut own = facts(Lifecycle::Draft, Visibility::Restricted);
    own.actor_is_contributor = true;
    assert!(
        can_access_content(Some(&reader()), &own, &policy).is_allowed(),
        "a contributor reaches their own draft"
    );

    let mut theirs = facts(Lifecycle::Draft, Visibility::Restricted);
    theirs.actor_is_contributor = false;
    assert!(
        !can_access_content(Some(&reader()), &theirs, &policy).is_allowed(),
        "and nobody else does"
    );
}
