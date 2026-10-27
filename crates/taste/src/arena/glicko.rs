//! Glicko-2 with draws — T-M16-01, spec §12.4 / R-6.
//!
//! Ported from `kindred/internal/arena/glicko.go`, which is the reference implementation
//! (§8.3: kindred is a reference to be matched, never a port target to be trusted). The
//! algorithm is Glickman's, revision 22 March 2022, and every constant and every branch
//! below is the published one.
//!
//! # Why not Elo
//!
//! Elo has no cold-start advantage, no confidence measure and no inactivity decay. This
//! corpus is heavily recent — a work nobody has compared in eight months should drift back
//! toward the middle, and a work compared twice should move further on its third comparison
//! than a work compared two hundred times. That *is* the rating deviation, and Elo cannot
//! express it. The archive's arena shipped Elo; this replaces it.
//!
//! # The five things the published algorithm punishes
//!
//! Each of these was wrong in a draft of the Go implementation and was caught by Glickman's
//! worked example rather than by reading the paper. They are listed because a test pins each
//! one, and a reviewer who sees only the final number cannot tell them apart:
//!
//! 1. **`v` is not `1/(1 + Σ g(φⱼ)²)`.** That is the pre-2012 draft. The published step 3
//!    weights each opponent by how decisive the comparison was:
//!    `v = [Σ g(φⱼ)² E(1−E)]⁻¹`. An expected result carries almost no information, and the
//!    draft form makes it count as much as an upset.
//! 2. **`μ′` uses `φ′²`, not `σ²/(φ²+v)`.** Step 7 is
//!    `μ′ = μ + φ′² Σ g(φⱼ)(sⱼ − E)`, with the *new* φ and no δ in it.
//! 3. **The volatility `f(x)` cannot be shortcut**, and it is discontinuous at its root —
//!    which is why step 4 is an Illinois regula-falsi iteration with an `f_A/2` damping term
//!    and not a bisection or a Newton step.
//! 4. **The bracket is asymmetric.** `A = a = ln(σ²)` always; `B = ln(δ² − φ² − v)` when
//!    `δ² > φ² + v`, and otherwise `a − k·τ` found by searching *left* in multiples of τ.
//!    A plain bisection over `[a−1, a+1]` converges to a plausible number that is not σ′.
//! 5. **The scale.** μ, φ and every [`Opponent`] field are on the **Glicko** scale; the
//!    reduced values exist only as locals inside [`Rating::update`]. Feeding a raw
//!    100-point gap into the exponent gives `E = 1.0` exactly, `δ = 0`, and a silent no-op
//!    that looks like it ran.
//!
//! # A period, not an event
//!
//! [`Rating::update`] takes **every** comparison from one rating period in a single call.
//! Applying a single-comparison rule per event inflates ratings; the old kindling notes
//! made exactly that mistake, which is why the signature takes a slice rather than one
//! opponent.

/// `400 / ln(10)` — the Glicko ↔ Glicko-2 conversion factor.
pub const SCALE: f64 = 173.7178;

/// How fast volatility may move. The paper suggests 0.3–1.2, and 0.2 for very improbable
/// outcome sets. 0.5 is what the reference implementation chose and what its corpus was
/// tuned against.
pub const TAU: f64 = 0.5;

/// Glickman's convergence tolerance for the volatility iteration.
pub const EPS: f64 = 1e-6;

/// The rating of a work nobody has compared, on the Glicko scale.
///
/// 1500/350 is the paper's own initial state: a work arrives maximally uncertain, which is
/// what makes a first comparison move it a long way and also what keeps a single comparison
/// from looking like a settled fact.
pub const MU_INIT: f64 = 1500.0;

/// The initial rating deviation, on the Glicko scale.
pub const PHI_INIT: f64 = 350.0;

/// The initial volatility.
pub const SIGMA_INIT: f64 = 0.06;

/// One comparison's outcome, from the perspective of the work being rated.
///
/// [`Outcome::Draw`] is the arena's "neither" button — a judgement that neither work is
/// preferable, not a skipped comparison. Discarding draws would bias ratings toward works
/// that only ever get compared decisively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// This work was chosen.
    Win,
    /// Neither was chosen.
    Draw,
    /// The other work was chosen.
    Loss,
}

impl Outcome {
    /// The residual the paper's formulas want: 1, 0.5 or 0.
    #[must_use]
    pub fn residual(self) -> f64 {
        match self {
            Self::Win => 1.0,
            Self::Draw => 0.5,
            Self::Loss => 0.0,
        }
    }
}

/// One opponent in a rating period.
///
/// `weight` is the *voter's* taste influence on this result, 0..1, and it rides on the
/// opponent because a result belongs to the voter who produced it: the same number has to
/// reach both sides of the comparison, or the two works would move by different amounts and
/// the pair's symmetry would be lost.
///
/// **Zero means unset, not "no influence".** A caller that wants no influence must drop the
/// opponent from the slice, because zero is what every unweighted call site already passes
/// and those call sites must keep producing the numbers they produced before weighting
/// existed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Opponent {
    /// The opponent's rating, Glicko scale.
    pub mu: f64,
    /// The opponent's rating deviation, Glicko scale.
    pub phi: f64,
    /// The result, from the rated work's point of view.
    pub outcome: Outcome,
    /// The voter's influence, 0..1, or 0 for "unset" (treated as 1).
    pub weight: f64,
}

impl Opponent {
    /// An opponent whose comparison counts at full weight.
    #[must_use]
    pub fn new(mu: f64, phi: f64, outcome: Outcome) -> Self {
        Self {
            mu,
            phi,
            outcome,
            weight: 0.0,
        }
    }

    /// The same opponent, carrying a voter's influence.
    #[must_use]
    pub fn weighted(mu: f64, phi: f64, outcome: Outcome, weight: f64) -> Self {
        Self {
            mu,
            phi,
            outcome,
            weight,
        }
    }

    /// The weight this comparison counts at: `weight` if it was set, else full.
    fn influence(&self) -> f64 {
        if self.weight <= 0.0 {
            1.0
        } else {
            self.weight
        }
    }
}

/// A Glicko-2 rating on the Glicko scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rating {
    /// Rating.
    pub mu: f64,
    /// Rating deviation: how much the rating is still in doubt.
    pub phi: f64,
    /// Volatility: how much the work's true strength is expected to move between periods.
    pub sigma: f64,
}

impl Default for Rating {
    fn default() -> Self {
        Self::initial()
    }
}

impl Rating {
    /// The rating of a work nobody has compared.
    #[must_use]
    pub fn initial() -> Self {
        Self {
            mu: MU_INIT,
            phi: PHI_INIT,
            sigma: SIGMA_INIT,
        }
    }

    /// Whether this rating is arithmetically usable.
    ///
    /// A stored rating can be damaged by a bad write, and one non-finite row poisons every
    /// later period it participates in — permanently, because its opponents then become
    /// non-finite too. Callers check this before writing.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        finite(self.mu) && finite(self.phi) && finite(self.sigma)
    }

    /// Apply one rating period.
    ///
    /// Takes every comparison from the period in one call — see the module docs. A period
    /// with no usable opponents returns the rating unchanged: the paper's step 6 says an
    /// inactive work's RD should *widen*, and that is [`Rating::inactivity`], a separate
    /// explicit call so the decay is visible at the call site rather than hidden inside an
    /// update that looks like it did nothing.
    #[must_use]
    pub fn update(&self, opponents: &[Opponent]) -> Self {
        // Guard the boundary before doing arithmetic. Non-finite or nonsensical opponents
        // are DROPPED rather than aborting the period: a comparison against a damaged row
        // is missing evidence, not a reason to discard the evidence that is present.
        let ops: Vec<Opponent> = opponents
            .iter()
            .copied()
            .filter(|op| finite(op.mu) && finite(op.phi) && op.phi > 0.0)
            .collect();
        if ops.is_empty() || !self.is_finite() {
            return *self;
        }

        let v = variance(self.mu, &ops);
        if !finite(v) {
            // A period of pure expected results carries no evidence.
            return *self;
        }

        let (mu, phi) = reduced(self.mu, self.phi);
        let delta = delta(self.mu, &ops);
        let new_sigma = volatility(phi, self.sigma, v, delta);

        // Step 6: φ* is the pre-convergence cap.
        let phi_star = (phi * phi + new_sigma * new_sigma).sqrt();
        // Step 7 uses the NEW φ.
        let phi_new = 1.0 / (1.0 / (phi_star * phi_star) + 1.0 / v).sqrt();
        let mu_new = mu + phi_new * phi_new * residual_sum(self.mu, &ops);

        let (mu_new, phi_new) = unreduced(mu_new, phi_new);
        Self {
            mu: mu_new,
            phi: phi_new,
            sigma: new_sigma,
        }
    }

    /// The paper's step 6 for a work that did not compete: rating and volatility unchanged,
    /// rating deviation widened to `φ* = sqrt(φ² + σ²)`.
    ///
    /// This is the mechanism that makes a Glicko-2 arena different from Elo. A work nobody
    /// has compared in months carries more uncertainty, so a later comparison moves it
    /// further — and a dormant work cannot sit frozen at the top of the ladder.
    #[must_use]
    pub fn inactivity(&self) -> Self {
        if !self.is_finite() || self.phi <= 0.0 || self.sigma <= 0.0 {
            return *self;
        }
        let (_, phi) = reduced(self.mu, self.phi);
        let phi_star = (phi * phi + self.sigma * self.sigma).sqrt();
        let (_, phi_new) = unreduced(0.0, phi_star);
        Self {
            mu: self.mu,
            phi: phi_new,
            sigma: self.sigma,
        }
    }

    /// The rating deviation, which is how uncertain the ladder is about this work.
    #[must_use]
    pub fn uncertainty(&self) -> f64 {
        self.phi
    }
}

/// `E(μ, μⱼ, φⱼ)`: the probability that `μ` beats `μⱼ`.
///
/// Both arguments are on the Glicko scale and both reductions happen here — the gap and the
/// φ. Written with `exp()`, as the paper does.
#[must_use]
pub fn expected(mu: f64, mu_j: f64, phi_j: f64) -> f64 {
    1.0 / (1.0 + (-g(reduce_phi(phi_j)) * (mu - mu_j) / SCALE).exp())
}

/// `E()` with the pre-2001 `400` divisor instead of `ln(10)`.
///
/// Present only so a test can prove the right constant is the one in use: the difference is
/// 0.5% per rating, which looks like noise next to a leaderboard and is systematically wrong
/// across every row in the table.
#[must_use]
pub fn expected_with_legacy_divisor(mu: f64, mu_j: f64, phi_j: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf(-g(reduce_phi(phi_j)) * (mu - mu_j) / SCALE) / 400.0)
}

/// Step 3, the published form: `v = [Σ wⱼ g(φⱼ)² E(1−E)]⁻¹`.
///
/// The weight sits in the **denominator's** sum, which is the direction that is easy to get
/// backwards: more influence means more evidence, more evidence means a smaller `v`, and a
/// smaller `v` means the rating moves *less* for the same δ.
///
/// Returns `+∞` when the period carried no information at all (every comparison exactly
/// 50/50 against a certainty-free opponent), which callers treat as "leave the rating
/// alone" rather than dividing by zero.
#[must_use]
pub fn variance(mu: f64, opponents: &[Opponent]) -> f64 {
    let mut sum = 0.0;
    for op in opponents {
        let e = expected(mu, op.mu, op.phi);
        let g_phi = g(reduce_phi(op.phi));
        sum += op.influence() * g_phi * g_phi * e * (1.0 - e);
    }
    if sum == 0.0 {
        return f64::INFINITY;
    }
    1.0 / sum
}

/// Step 4: `δ = v Σ wⱼ g(φⱼ)(sⱼ − E)`.
///
/// The weighting is not repeated here: `v` and [`residual_sum`] both already carry it, and a
/// third copy of the rule is a third place for it to disagree.
#[must_use]
pub fn delta(mu: f64, opponents: &[Opponent]) -> f64 {
    variance(mu, opponents) * residual_sum(mu, opponents)
}

/// The evidence in the period: `Σ wⱼ g(φⱼ)(sⱼ − E)`.
///
/// The weight is folded in per result rather than applied to the finished sum: the sum is
/// over results, each result carries its own voter, and scaling the total afterwards would
/// be saying every voter's evidence is worth the same.
fn residual_sum(mu: f64, opponents: &[Opponent]) -> f64 {
    let mut sum = 0.0;
    for op in opponents {
        sum += op.influence() * g(reduce_phi(op.phi)) * (op.outcome.residual() - expected(mu, op.mu, op.phi));
    }
    sum
}

/// `g(φ) = 1 / sqrt(1 + 3φ²/π²)`, with φ on the Glicko-2 scale.
fn g(phi: f64) -> f64 {
    1.0 / (1.0 + 3.0 * phi * phi / (std::f64::consts::PI * std::f64::consts::PI)).sqrt()
}

/// Glicko → Glicko-2 for a rating deviation. The **only** place this division happens.
fn reduce_phi(phi: f64) -> f64 {
    phi / SCALE
}

/// Glicko → Glicko-2 for (μ, φ).
fn reduced(mu: f64, phi: f64) -> (f64, f64) {
    ((mu - MU_INIT) / SCALE, reduce_phi(phi))
}

/// Glicko-2 → Glicko for (μ, φ).
fn unreduced(mu: f64, phi: f64) -> (f64, f64) {
    (SCALE * mu + MU_INIT, SCALE * phi)
}

/// The volatility objective, exactly as the paper writes it.
///
/// Discontinuous where it crosses zero, which is the whole reason step 4 is an Illinois
/// iteration over a bracket instead of a root-finder that assumes smoothness.
fn f(x: f64, a: f64, phi_sq: f64, v: f64, delta: f64) -> f64 {
    let ex = x.exp();
    let den = phi_sq + v + ex;
    ex * (delta * delta - phi_sq - v - ex) / (2.0 * den * den) - (x - a) / (TAU * TAU)
}

/// Step 4: the Illinois iteration on the volatility bracket.
///
/// The bracket is neither symmetric nor guessed. `A = a = ln(σ²)` always. If
/// `δ² > φ² + v` the root is to the RIGHT and `B = ln(δ² − φ² − v)`; otherwise it is to the
/// LEFT and `B = a − kτ` for the smallest `k` that brackets it.
///
/// **The bracket may be oriented either way, and that is not a detail.** In the log branch
/// `B` can land far above `A` (around +9 for a big upset by an established work). A guard
/// that insists `A > B` rejects that perfectly good bracket and returns σ unchanged —
/// silently freezing volatility for exactly the works whose volatility most needs to move,
/// with a symptom of "the ratings look stable because they are stuck".
fn volatility(phi: f64, sigma: f64, v: f64, delta: f64) -> f64 {
    let phi_sq = phi * phi;
    let a = (sigma * sigma).ln();

    let mut a_point = a;
    let mut b_point = if delta * delta > phi_sq + v {
        (delta * delta - phi_sq - v).ln()
    } else {
        let mut k = 1;
        while k < 100 && f(a - f64::from(k) * TAU, a, phi_sq, v, delta) < 0.0 {
            k += 1;
        }
        if k >= 100 {
            // No bracket exists: σ is non-finite upstream. Leave it alone rather than
            // propagate a NaN into every rating this work will ever appear in.
            return sigma;
        }
        a - f64::from(k) * TAU
    };

    // What must hold is that f changes sign across the bracket — nothing about its
    // orientation. Illinois is orientation-agnostic and the false-position step is
    // symmetric, so the endpoints are ordered here and nothing else.
    let mut f_a = f(a_point, a, phi_sq, v, delta);
    let mut f_b = f(b_point, a, phi_sq, v, delta);
    if !finite(f_a) || !finite(f_b) || f_a * f_b > 0.0 {
        return sigma;
    }
    if a_point > b_point {
        std::mem::swap(&mut a_point, &mut b_point);
        std::mem::swap(&mut f_a, &mut f_b);
    }

    for iteration in 0.. {
        if (b_point - a_point).abs() <= EPS || iteration > 200 {
            break;
        }
        let denominator = f_b - f_a;
        if denominator == 0.0 {
            break;
        }
        let c = a_point + (a_point - b_point) * f_a / denominator;
        let f_c = f(c, a, phi_sq, v, delta);
        if f_c * f_b <= 0.0 {
            // The new point is on the far side: move A to the old B.
            a_point = b_point;
            f_a = f_b;
        } else {
            // Same side: damp the retained value. This is the Illinois modification, and
            // dropping it is what made the older closed form unreliable.
            f_a /= 2.0;
        }
        b_point = c;
        f_b = f_c;
    }
    (a_point / 2.0).exp()
}

/// Not `NaN`, not `±∞`.
fn finite(value: f64) -> bool {
    !value.is_nan() && !value.is_infinite()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Glickman's worked example: player 1500/200/0.06 against three opponents,
    /// `(1400, 30)` won, `(1550, 100)` lost, `(1700, 300)` lost.
    ///
    /// The paper prints `r′ 1464.06`, `RD′ 151.52`, `σ′ 0.05999`, `v 1.7785`,
    /// `δ −0.4834`, `φ* 1.152862` — and every one of those is pinned separately below.
    /// Pinning only the final answer would let three different implementations through: a
    /// wrong `v` with a compensating error in `μ′`, a right `μ′` from the draft step 7, and
    /// a bisection that happens to land near 0.06.
    fn reference() -> (Rating, Vec<Opponent>) {
        let opponents = vec![
            Opponent::new(1400.0, 30.0, Outcome::Win),
            Opponent::new(1550.0, 100.0, Outcome::Loss),
            Opponent::new(1700.0, 300.0, Outcome::Loss),
        ];
        (
            Rating {
                mu: 1500.0,
                phi: 200.0,
                sigma: 0.06,
            },
            opponents,
        )
    }

    #[test]
    fn step_three_v_matches_the_paper_and_rejects_the_draft() {
        let (rating, opponents) = reference();
        let v = variance(rating.mu, &opponents);
        assert!(
            (v - 1.7785).abs() < 0.001,
            "v = {v:.4}, paper says 1.7785 (the pre-2012 draft gives 0.2921)"
        );
        // The draft formula, computed here in full, so the assertion above is proven to
        // discriminate rather than merely to pass.
        let draft: f64 = 1.0
            / (1.0
                + opponents
                    .iter()
                    .map(|o| g(reduce_phi(o.phi)).powi(2))
                    .sum::<f64>());
        assert!((draft - 0.2921).abs() < 0.001, "draft = {draft:.4}");
        assert!((v - draft).abs() > 1.0, "the two forms must not be near each other");
    }

    #[test]
    fn step_four_delta_matches_the_paper() {
        let (rating, opponents) = reference();
        let d = delta(rating.mu, &opponents);
        assert!((d - -0.4834).abs() < 0.001, "delta = {d:.4}, paper says -0.4834");
    }

    #[test]
    fn steps_six_to_eight_match_the_paper() {
        let (rating, opponents) = reference();
        let updated = rating.update(&opponents);

        let (_, phi) = reduced(1500.0, 200.0);
        let phi_star = (phi * phi + updated.sigma * updated.sigma).sqrt();
        assert!(
            (phi_star - 1.152862).abs() < 1e-5,
            "step 6: phi* = {phi_star:.6}, paper says 1.152862"
        );

        let v = variance(rating.mu, &opponents);
        let phi_reduced = 1.0 / (1.0 / (phi_star * phi_star) + 1.0 / v).sqrt();
        assert!(
            (phi_reduced - 0.8722).abs() < 2e-4,
            "step 7: phi' = {phi_reduced:.6}, paper says 0.8722"
        );

        assert!(
            (updated.mu - 1464.06).abs() < 0.01,
            "step 8: r' = {:.4}, paper says 1464.06",
            updated.mu
        );
        assert!(
            (updated.phi - 151.52).abs() < 0.01,
            "step 8: RD' = {:.4}, paper says 151.52",
            updated.phi
        );
        assert!(
            (updated.sigma - 0.05999).abs() < 1e-5,
            "step 4: sigma' = {:.6}, paper says 0.05999",
            updated.sigma
        );
        // The paper's example moves volatility by only 1e-5, which is inside the tolerance
        // above — so an implementation that never touches volatility at all would pass it.
        // This is the assertion that catches the frozen-sigma failure mode, and it is exact
        // rather than tolerant because any real iteration lands somewhere other than where
        // it started.
        assert_ne!(
            updated.sigma, rating.sigma,
            "volatility did not move at all: the iteration was skipped or refused its bracket"
        );
    }

    #[test]
    fn the_volatility_iteration_matches_the_papers_table() {
        // v 1.7785, delta -0.4834, phi 1.1513, sigma 0.06, tau 0.5 — iteration 0.
        let phi_sq = 1.1513 * 1.1513;
        let a = (0.06f64 * 0.06).ln();
        let f_a = f(a, a, phi_sq, 1.7785, -0.4834);
        assert!(
            (f_a - -0.00053567).abs() < 1e-7,
            "f(A) = {f_a:.8}, paper's table says -0.00053567"
        );
        let f_b = f(a - TAU, a, phi_sq, 1.7785, -0.4834);
        assert!(
            (f_b - 1.999675).abs() < 1e-6,
            "f(B) = {f_b:.8}, paper's table says 1.999675"
        );

        let sigma = volatility(1.1513, 0.06, 1.7785, -0.4834);
        assert!(
            (sigma - 0.05999).abs() < 1e-5,
            "sigma' = {sigma:.6}, paper says 0.05999"
        );
    }

    #[test]
    fn the_exponent_uses_ln_ten_and_not_400() {
        // The right answer is the paper's 0.639 for the first opponent; the wrong constant
        // is both wrong AND far away, so this cannot pass by luck.
        let right = expected(1500.0, 1400.0, 30.0);
        assert!((right - 0.639).abs() < 0.001, "E = {right:.4}, paper says 0.639");

        let wrong = expected_with_legacy_divisor(1500.0, 1400.0, 30.0);
        assert!(
            (wrong - right).abs() > 0.01,
            "the legacy divisor must produce a different number, else nothing is under test"
        );
    }

    #[test]
    fn a_period_with_no_opponents_is_a_no_op() {
        let rating = Rating::initial();
        assert_eq!(rating.update(&[]), rating);
    }

    #[test]
    fn a_win_raises_and_a_loss_lowers() {
        let rating = Rating::initial();
        let won = rating.update(&[Opponent::new(1500.0, 350.0, Outcome::Win)]);
        let lost = rating.update(&[Opponent::new(1500.0, 350.0, Outcome::Loss)]);
        assert!(won.mu > rating.mu, "a win must raise the rating");
        assert!(lost.mu < rating.mu, "a loss must lower it");
        // Symmetry: the same comparison against an equal opponent moves both sides equally.
        assert!(
            ((won.mu - rating.mu) + (lost.mu - rating.mu)).abs() < 1e-9,
            "the pair is not symmetric"
        );
    }

    #[test]
    fn a_draw_is_halfway_and_not_a_skip() {
        let rating = Rating::initial();
        let drawn = rating.update(&[Opponent::new(1500.0, 350.0, Outcome::Draw)]);
        // Between an equal opponent, a draw must barely move the rating — but it must still
        // shrink the RD, because the comparison happened and it told us *something*.
        assert!((drawn.mu - rating.mu).abs() < 1e-6, "a draw against an equal moved the rating");
        assert!(
            drawn.phi < rating.phi,
            "a draw must reduce uncertainty: {:?}",
            drawn
        );
    }

    #[test]
    fn the_rating_deviation_shrinks_and_stays_positive() {
        let mut rating = Rating::initial();
        for _ in 0..40 {
            rating = rating.update(&[Opponent::new(1500.0, 100.0, Outcome::Win)]);
        }
        assert!(rating.phi > 0.0, "RD went non-positive");
        assert!(rating.phi < PHI_INIT, "RD never shrank over 40 comparisons");
        assert!(rating.is_finite());
    }

    #[test]
    fn inactivity_only_widens_the_rating_deviation() {
        let fresh = Rating::initial();
        let dormant = fresh.inactivity();
        assert_eq!(dormant.mu, fresh.mu, "inactivity must not move the rating");
        assert_eq!(dormant.sigma, fresh.sigma, "inactivity must not move volatility");
        assert!(dormant.phi > fresh.phi, "inactivity must widen uncertainty");
        // And the widened RD makes a comparison move the rating further.
        let opponent = [Opponent::new(1600.0, 50.0, Outcome::Win)];
        let moved_fresh = fresh.update(&opponent).mu - fresh.mu;
        let moved_dormant = dormant.update(&opponent).mu - dormant.mu;
        assert!(
            moved_dormant > moved_fresh,
            "a dormant work must move further: {moved_dormant} vs {moved_fresh}"
        );
    }

    #[test]
    fn the_volatility_bracket_may_be_inverted() {
        // An established work (RD 30) beats a much stronger, also well-known one:
        // delta^2 > phi^2 + v, so this is the log branch and B lands above A.
        let rating = Rating {
            mu: 1500.0,
            phi: 30.0,
            sigma: 0.06,
        };
        let opponents = [Opponent::new(2300.0, 20.0, Outcome::Win)];
        let v = variance(rating.mu, &opponents);
        let d = delta(rating.mu, &opponents);
        let (_, phi) = reduced(rating.mu, rating.phi);
        assert!(
            d * d > phi * phi + v,
            "this test no longer exercises the inverted bracket"
        );
        let sigma = volatility(phi, rating.sigma, v, d);
        assert!(
            sigma > rating.sigma,
            "an upset must raise volatility, not leave it frozen: {sigma} vs {}",
            rating.sigma
        );
    }

    #[test]
    fn a_damaged_opponent_is_dropped_and_not_fatal() {
        let rating = Rating::initial();
        let clean = rating.update(&[Opponent::new(1500.0, 200.0, Outcome::Win)]);
        let mixed = rating.update(&[
            Opponent::new(1500.0, 200.0, Outcome::Win),
            Opponent::new(f64::NAN, 200.0, Outcome::Win),
            Opponent::new(1600.0, 0.0, Outcome::Win),
            Opponent::new(1700.0, -50.0, Outcome::Loss),
        ]);
        assert_eq!(mixed, clean, "a damaged row changed the result");
        assert!(mixed.is_finite());
    }

    #[test]
    fn a_damaged_rating_never_poisons_the_ladder() {
        let broken = [
            Rating { mu: f64::NAN, phi: 200.0, sigma: 0.06 },
            Rating { mu: 1500.0, phi: f64::INFINITY, sigma: 0.06 },
            Rating { mu: 1500.0, phi: 200.0, sigma: 0.0 },
        ];
        for rating in broken {
            let updated = rating.update(&[Opponent::new(1500.0, 200.0, Outcome::Win)]);
            // Unchanged, and — the point — still whatever it was rather than NaN-written.
            assert_eq!(updated.mu.is_nan(), rating.mu.is_nan());
            let dormant = rating.inactivity();
            assert_eq!(dormant.mu.is_nan(), rating.mu.is_nan());
        }
    }

    #[test]
    fn a_weight_of_zero_means_unset_and_not_silence() {
        let rating = Rating::initial();
        let unweighted = rating.update(&[Opponent::new(1600.0, 100.0, Outcome::Win)]);
        let zero_weight = rating.update(&[Opponent::weighted(1600.0, 100.0, Outcome::Win, 0.0)]);
        assert_eq!(unweighted, zero_weight, "0 must mean unset");

        // And a low weight is *less* evidence: it must move the rating less far.
        let light = rating.update(&[Opponent::weighted(1600.0, 100.0, Outcome::Win, 0.1)]);
        assert!(
            (light.mu - rating.mu).abs() < (unweighted.mu - rating.mu).abs(),
            "a 0.1-weight comparison moved the rating as far as a full one"
        );
    }

    #[test]
    fn weighted_comparisons_move_both_sides_equally() {
        // The pair's symmetry is the reason the weight rides on the opponent: if the two
        // sides were weighted differently the same comparison would move two works by
        // different amounts, and the ladder would drift under its own bookkeeping.
        let a = Rating::initial();
        let b = Rating::initial();
        let a_moved = a.update(&[Opponent::weighted(b.mu, b.phi, Outcome::Win, 0.25)]);
        let b_moved = b.update(&[Opponent::weighted(a.mu, a.phi, Outcome::Loss, 0.25)]);
        assert!(
            ((a_moved.mu - MU_INIT) + (b_moved.mu - MU_INIT)).abs() < 1e-9,
            "asymmetric move: {} and {}",
            a_moved.mu,
            b_moved.mu
        );
    }
}
