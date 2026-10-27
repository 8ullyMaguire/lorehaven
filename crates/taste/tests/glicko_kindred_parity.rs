//! Kindred parity — T-M16-01 step 4: *"the Rust implementation is checked against kindred's on
//! kindred's corpus, case by case, before it is believed"* (R-6).
//!
//! The fixture in `fixtures/kindred/glicko_parity.json` was **produced by kindred's own
//! `internal/arena/glicko.go`** (`@8da833f`), by copying that file verbatim into a throwaway
//! Go module, importing it, and recording what it returned. It is an oracle, not a
//! transcription: if the two implementations disagree, this test says so, and neither side
//! gets to be right by assertion.
//!
//! # Why the cases are shaped the way they are
//!
//! - the paper's worked example, because it is the only case with a published answer;
//! - one win, one loss and one draw from the initial rating, because a draw must move the
//!   rating and shrink the RD rather than being skipped;
//! - the inverted-bracket upset, because that is the branch a guard gets wrong silently;
//! - weighted and unweighted influence on the same comparison, because `0` must mean *unset*
//!   and not *silence*;
//! - a five-opponent period, because a per-event update gets a period wrong and a
//!   single-opponent test cannot see it;
//! - thirty seeded periods, because a scale error that is invisible once is a different
//!   leaderboard after thirty comparisons.
//!
//! The tolerance is tight on purpose. Both sides are IEEE-754 doubles performing the same
//! operations in the same order, so a divergence above ~1e-12 relative is a *different
//! calculation*, not accumulated floating-point noise. (The comparison is per case, from the
//! `before` the fixture records, so error cannot compound between cases and a loose tolerance
//! would not be hiding drift.)

use darkhold_taste::arena::glicko::{delta, Opponent, Outcome, Rating};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Fixture {
    /// Where the oracle came from: the reference implementation and its revision.
    source: String,
    cases: Vec<UpdateCase>,
    dormant: Vec<InactivityCase>,
}

#[derive(Debug, Deserialize)]
struct UpdateCase {
    name: String,
    before: RatingJson,
    opponents: Vec<OpponentJson>,
    after: RatingJson,
    delta: f64,
}

#[derive(Debug, Deserialize)]
struct InactivityCase {
    name: String,
    before: RatingJson,
    after: RatingJson,
}

#[derive(Debug, Deserialize, Clone, Copy)]
struct RatingJson {
    mu: f64,
    phi: f64,
    sigma: f64,
}

#[derive(Debug, Deserialize)]
struct OpponentJson {
    mu: f64,
    phi: f64,
    score: f64,
    weight: f64,
}

impl RatingJson {
    fn to_rating(self) -> Rating {
        Rating {
            mu: self.mu,
            phi: self.phi,
            sigma: self.sigma,
        }
    }
}

impl OpponentJson {
    fn to_opponent(&self) -> Opponent {
        // The fixture carries the residual kindred used (1, 0.5, 0). Mapping it back to the
        // enum here rather than accepting a bare float keeps the port's type safety and still
        // compares against the oracle's numbers.
        let outcome = match self.score {
            s if s == 1.0 => Outcome::Win,
            s if s == 0.5 => Outcome::Draw,
            s if s == 0.0 => Outcome::Loss,
            other => panic!("the fixture holds a residual that is not an outcome: {other}"),
        };
        Opponent::weighted(self.mu, self.phi, outcome, self.weight)
    }
}

fn fixture() -> Fixture {
    // `include_str!` rather than a runtime path: a fixture that cannot be found must fail the
    // build, not skip a test.
    serde_json::from_str(include_str!("fixtures/kindred/glicko_parity.json"))
        .expect("the kindred parity fixture must parse")
}

/// Relative tolerance: `|a - b| <= 1e-12 * max(1, |a|)`.
fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() <= 1e-12 * actual.abs().max(1.0).max(expected.abs())
}

#[test]
fn the_oracle_is_the_reference_implementation() {
    let fixture = fixture();
    assert!(
        fixture.source.contains("kindred") && fixture.source.contains("glicko.go"),
        "the fixture must name where it came from: {}",
        fixture.source
    );
    // And it must contain enough cases to mean something. A fixture trimmed to two cases
    // would still pass every assertion below while covering almost none of the branches.
    assert!(
        fixture.cases.len() >= 30,
        "the fixture has only {} update cases",
        fixture.cases.len()
    );
    assert!(!fixture.dormant.is_empty(), "no inactivity cases recorded");
    assert!(
        fixture.cases.iter().any(|case| case.name == "paper-example"),
        "the paper's worked example must be in the oracle"
    );
    assert!(
        fixture.cases.iter().any(|case| case.name == "inverted-bracket-upset"),
        "the inverted-bracket case must be in the oracle"
    );
}

#[test]
fn every_update_case_matches_kindred() {
    let fixture = fixture();
    let mut mismatches = Vec::new();

    for case in &fixture.cases {
        let before = case.before.to_rating();
        let opponents: Vec<Opponent> = case.opponents.iter().map(OpponentJson::to_opponent).collect();
        let after = before.update(&opponents);

        for (label, actual, expected) in [
            ("mu", after.mu, case.after.mu),
            ("phi", after.phi, case.after.phi),
            ("sigma", after.sigma, case.after.sigma),
        ] {
            if !close(actual, expected) {
                mismatches.push(format!(
                    "{}: {label} = {actual:.17} but kindred has {expected:.17}",
                    case.name
                ));
            }
        }

        let our_delta = delta(before.mu, &opponents);
        if !close(our_delta, case.delta) {
            mismatches.push(format!(
                "{}: delta = {our_delta:.17} but kindred has {:.17}",
                case.name, case.delta
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "{} of {} cases disagree with kindred:\n  {}",
        mismatches.len(),
        fixture.cases.len(),
        mismatches.join("\n  ")
    );
}

#[test]
fn every_dormant_case_matches_kindred() {
    let fixture = fixture();
    for case in &fixture.dormant {
        let after = case.before.to_rating().inactivity();
        assert!(
            close(after.mu, case.after.mu),
            "{}: inactive mu = {:.17} but kindred has {:.17}",
            case.name,
            after.mu,
            case.after.mu
        );
        assert!(
            close(after.phi, case.after.phi),
            "{}: inactive phi = {:.17} but kindred has {:.17}",
            case.name,
            after.phi,
            case.after.phi
        );
        assert!(
            close(after.sigma, case.after.sigma),
            "{}: inactive sigma = {:.17} but kindred has {:.17}",
            case.name,
            after.sigma,
            case.after.sigma
        );
    }
}
