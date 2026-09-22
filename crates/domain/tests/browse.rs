//! M43 — Browse ordering vocabulary (spec §43).
//!
//! One `Sort` enum serves every browse surface. Tests cover the vocabulary
//! itself, the per-surface preference API, and the default-sort contract.

use lorehaven_domain::browse::Sort;

#[test]
fn sort_parse_all_vocabulary() {
    for sort in Sort::ALL {
        assert_eq!(Sort::parse(sort.as_str()), Some(*sort));
    }
}

#[test]
fn sort_parse_rejects_unknown() {
    assert!(Sort::parse("random").is_none());
    assert!(Sort::parse("").is_none());
    assert!(Sort::parse("for_you").is_none());
    assert!(Sort::parse("For-You").is_none());
}

#[test]
fn sort_from_str_error_names_accepted_set() {
    let result: Result<Sort, String> = "bogus".parse();
    assert!(result.is_err());
    let msg = result.unwrap_err();
    for sort in Sort::ALL {
        assert!(
            msg.contains(sort.as_str()),
            "error should list `{}`: {msg}",
            sort.as_str()
        );
    }
}

#[test]
fn sort_taste_steered_flag() {
    assert!(Sort::ForYou.is_taste_steered());
    assert!(Sort::Trending.is_taste_steered());
    assert!(!Sort::New.is_taste_steered());
    assert!(!Sort::Updated.is_taste_steered());
    assert!(!Sort::Top.is_taste_steered());
    assert!(!Sort::BestMatch.is_taste_steered());
    assert!(!Sort::Az.is_taste_steered());
}

#[test]
fn sort_exact_flag() {
    assert!(Sort::New.is_exact());
    assert!(Sort::Updated.is_exact());
    assert!(Sort::Top.is_exact());
    assert!(Sort::BestMatch.is_exact());
    assert!(Sort::Az.is_exact());
    assert!(!Sort::ForYou.is_exact());
    assert!(!Sort::Trending.is_exact());
}

#[test]
fn sort_default_is_new() {
    assert_eq!(Sort::default(), Sort::New);
}

#[test]
fn sort_display_round_trips() {
    for sort in Sort::ALL {
        let s = sort.to_string();
        let parsed: Sort = s.parse().unwrap();
        assert_eq!(*sort, parsed);
    }
}

#[test]
fn sort_serde_round_trips() {
    for sort in Sort::ALL {
        let json = serde_json::to_string(sort).unwrap();
        let parsed: Sort = serde_json::from_str(&json).unwrap();
        assert_eq!(*sort, parsed);
    }
}

#[test]
fn sort_serde_rejects_unknown() {
    let result: Result<Sort, _> = serde_json::from_str("\"bogus\"");
    assert!(result.is_err());
}

#[test]
fn sort_error_message_format() {
    let result: Result<Sort, String> = "xyz".parse();
    let msg = result.unwrap_err();
    assert!(msg.contains("unknown sort"));
    assert!(msg.contains("xyz"));
    assert!(msg.contains("accepted:"));
}
