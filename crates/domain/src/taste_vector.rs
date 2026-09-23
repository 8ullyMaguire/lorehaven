//! Multi-dimensional taste vectors and resonance computation.
//! Pure functions, no I/O.

use serde::{Deserialize, Serialize};

/// Default taste dimensions. Instance-configurable via `[taste].dimensions`.
pub const DEFAULT_DIMENSIONS: &[&str] = &[
    "angst",
    "pacing",
    "prose_density",
    "canon_compliance",
    "trope_intensity",
];

/// A single dimension in the instance taste profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TasteDimension {
    /// Dimension key (e.g., "angst").
    pub key: String,
    /// Human-readable label.
    pub label: String,
    /// Admin's ideal position on this axis (0.0–1.0).
    pub admin_target: f64,
    /// How strongly this dimension influences recommendations.
    pub weight: f64,
}

/// The instance taste profile: admin-defined dimensions + exemplars/anti-examples.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TasteProfile {
    /// Instance-defined dimensions.
    pub dimensions: Vec<TasteDimension>,
    /// Works the admin loves (positive anchors).
    pub exemplars: Vec<String>,
    /// Works the admin dislikes (negative anchors).
    pub anti_examples: Vec<String>,
}

/// A user's position on each taste dimension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TasteVector {
    /// Position on each dimension (0.0–1.0).
    pub dimensions: Vec<f64>,
}

/// Components of the resonance score.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResonanceComponents {
    /// Jaccard similarity of user vs admin exemplars.
    pub bookmark_overlap: f64,
    /// Correlation of shared ratings.
    pub rating_correlation: f64,
    /// Fraction of admin-liked works this user finished.
    pub completion_alignment: f64,
    /// Time on admin-aligned works / total reading time.
    pub reading_time_ratio: f64,
}

/// Weights for resonance computation (instance-configurable).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResonanceWeights {
    pub bookmark_overlap: f64,
    pub rating_correlation: f64,
    pub completion_alignment: f64,
    pub reading_time_ratio: f64,
}

impl Default for ResonanceWeights {
    fn default() -> Self {
        Self {
            bookmark_overlap: 0.30,
            rating_correlation: 0.25,
            completion_alignment: 0.25,
            reading_time_ratio: 0.20,
        }
    }
}

/// Qualitative label for resonance score.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ResonanceLabel {
    Strong,
    Moderate,
    Weak,
    Neutral,
    Dissonant,
}

impl ResonanceLabel {
    /// User-facing qualitative text (never a raw number, §16.17).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Strong => "Aligned: Strong",
            Self::Moderate => "Aligned: Moderate",
            Self::Weak => "Aligned: Weak",
            Self::Neutral => "Neutral",
            Self::Dissonant => "Divergent",
        }
    }
}

/// Euclidean distance between two taste vectors.
/// Returns 0.0 (identical) to ~infinity. Normalize to 0..1 with `normalized_distance`.
pub fn euclidean_distance(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f64>()
        .sqrt()
}

/// Manhattan distance (alternative, configurable via `[taste].distance_metric`).
pub fn manhattan_distance(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(x, y)| (x - y).abs()).sum()
}

/// Normalize distance to 0..1 using tanh scaling.
/// `scale` controls how quickly distance saturates. Default 2.0.
pub fn normalized_distance(a: &[f64], b: &[f64], scale: f64) -> f64 {
    if scale <= 0.0 {
        return 0.0;
    }
    let raw = euclidean_distance(a, b);
    (raw / scale).tanh()
}

/// Resonance is inverse normalized distance: 1.0 = identical to admin, 0.0 = infinitely far.
pub fn resonance_from_distance(distance: f64) -> f64 {
    (1.0 - distance).clamp(0.0, 1.0)
}

/// Compute a user's taste vector from their interactions with rated works.
/// Returns a vector where each dimension is the weighted average of the user's
/// rated works on that dimension.
///
/// Returns an empty vec if `work_vectors` is empty or dimensions mismatch.
pub fn compute_user_vector(work_vectors: &[Vec<f64>], work_weights: &[f64]) -> Vec<f64> {
    let n = work_vectors.first().map(|v| v.len()).unwrap_or(0);
    if n == 0 {
        return vec![];
    }
    let mut result = vec![0.0; n];
    let mut total_weight = 0.0;
    for (vec, weight) in work_vectors.iter().zip(work_weights.iter()) {
        if vec.len() != n {
            continue;
        }
        for i in 0..n {
            result[i] += vec[i] * weight;
        }
        total_weight += weight;
    }
    if total_weight > 0.0 {
        for val in result.iter_mut() {
            *val /= total_weight;
        }
    }
    result
}

/// Incremental update: add a single work's contribution to an existing vector.
/// `old_weight_sum` is the previous total weight (for efficient delta).
/// Returns the new total weight sum.
pub fn update_vector_incremental(
    current: &mut [f64],
    work_vector: &[f64],
    work_weight: f64,
    old_weight_sum: f64,
) -> f64 {
    let new_weight_sum = old_weight_sum + work_weight;
    if new_weight_sum > 0.0 && work_vector.len() == current.len() {
        for (c, w) in current.iter_mut().zip(work_vector.iter()) {
            *c = (*c * old_weight_sum + w * work_weight) / new_weight_sum;
        }
    }
    new_weight_sum
}

/// Blend two vectors with a mixing factor. Used for taste probes and
/// onboarding quiz results.
pub fn blend_vectors(a: &[f64], b: &[f64], t: f64) -> Vec<f64> {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| x * (1.0 - t) + y * t)
        .collect()
}

/// Compute resonance score from components and weights.
pub fn compute_resonance(components: &ResonanceComponents, weights: &ResonanceWeights) -> f64 {
    let score = components.bookmark_overlap * weights.bookmark_overlap
        + components.rating_correlation * weights.rating_correlation
        + components.completion_alignment * weights.completion_alignment
        + components.reading_time_ratio * weights.reading_time_ratio;
    score.clamp(0.0, 1.0)
}

/// Convert resonance score to qualitative label.
pub fn qualitative_label(score: f64) -> ResonanceLabel {
    match score {
        s if s >= 0.7 => ResonanceLabel::Strong,
        s if s >= 0.5 => ResonanceLabel::Moderate,
        s if s >= 0.3 => ResonanceLabel::Weak,
        s if s > 0.0 => ResonanceLabel::Neutral,
        _ => ResonanceLabel::Dissonant,
    }
}

/// Jaccard similarity between two sets of strings.
pub fn jaccard_similarity(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let set_a: std::collections::HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let set_b: std::collections::HashSet<&str> = b.iter().map(|s| s.as_str()).collect();
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

/// Compute distance between a user vector and the admin profile.
pub fn distance_to_profile(user: &TasteVector, profile: &TasteProfile) -> f64 {
    let admin_vec: Vec<f64> = profile.dimensions.iter().map(|d| d.admin_target).collect();
    if user.dimensions.len() != admin_vec.len() {
        return 1.0;
    }
    normalized_distance(&user.dimensions, &admin_vec, 2.0)
}

/// Compute resonance between a user and the admin profile.
pub fn profile_resonance(user: &TasteVector, profile: &TasteProfile) -> f64 {
    resonance_from_distance(distance_to_profile(user, profile))
}

/// Derive a work's taste vector from its tags and the instance dimensions.
///
/// Each dimension starts at neutral 0.5. A tag whose canonical name equals a
/// dimension key (case-insensitive) shifts that dimension by `weight / 100`
/// from neutral, clamped to [0,1]. Tag weights are signed (a negative weight
/// suppresses). Tags matching no dimension are ignored. The returned vector is
/// ordered by dimension key sort order — callers must sort dimensions the same
/// way before interpreting positions.
pub fn work_vector_from_tags(
    dimensions: &[(String, String, f64, f64)],
    tags: &[(String, i64)],
) -> Vec<f64> {
    let mut sorted: Vec<&(String, String, f64, f64)> = dimensions.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    sorted
        .iter()
        .map(|(key, _label, _target, _weight)| {
            let mut score = 0.5f64;
            for (tag, weight) in tags {
                if tag.to_lowercase() == key.to_lowercase() {
                    score += *weight as f64 / 100.0;
                }
            }
            score.clamp(0.0, 1.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euclidean_distance_identical_vectors() {
        let a = vec![0.5, 0.5, 0.5];
        assert!((euclidean_distance(&a, &a) - 0.0).abs() < 1e-10);
    }

    #[test]
    fn euclidean_distance_known_value() {
        let a = vec![0.0, 0.0];
        let b = vec![3.0, 4.0];
        assert!((euclidean_distance(&a, &b) - 5.0).abs() < 1e-10);
    }

    #[test]
    fn manhattan_distance_identical() {
        let a = vec![0.5, 0.5];
        assert!((manhattan_distance(&a, &a) - 0.0).abs() < 1e-10);
    }

    #[test]
    fn manhattan_distance_known_value() {
        let a = vec![0.0, 0.0];
        let b = vec![3.0, 4.0];
        assert!((manhattan_distance(&a, &b) - 7.0).abs() < 1e-10);
    }

    #[test]
    fn normalized_distance_saturates_to_one() {
        let a = vec![0.0];
        let b = vec![100.0];
        let d = normalized_distance(&a, &b, 2.0);
        assert!(d > 0.99 && d <= 1.0);
    }

    #[test]
    fn normalized_distance_zero_for_identical() {
        let a = vec![0.5, 0.5];
        assert!((normalized_distance(&a, &a, 2.0) - 0.0).abs() < 1e-10);
    }

    #[test]
    fn resonance_from_distance_inverse() {
        assert!((resonance_from_distance(0.0) - 1.0).abs() < 1e-10);
        assert!((resonance_from_distance(1.0) - 0.0).abs() < 1e-10);
        assert!((resonance_from_distance(0.5) - 0.5).abs() < 1e-10);
    }

    #[test]
    fn resonance_clamps() {
        assert!(resonance_from_distance(-0.5) >= 0.0);
        assert!(resonance_from_distance(1.5) <= 1.0);
    }

    #[test]
    fn compute_user_vector_empty() {
        let result = compute_user_vector(&[], &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn compute_user_vector_single_work() {
        let work = vec![0.8, 0.6, 0.4];
        let result = compute_user_vector(&[work.clone()], &[1.0]);
        assert_eq!(result, work);
    }

    #[test]
    fn compute_user_vector_weighted() {
        let w1 = vec![1.0, 0.0];
        let w2 = vec![0.0, 1.0];
        // Equal weights → midpoint
        let result = compute_user_vector(&[w1.clone(), w2.clone()], &[1.0, 1.0]);
        assert!((result[0] - 0.5).abs() < 1e-10);
        assert!((result[1] - 0.5).abs() < 1e-10);

        // Double weight on w1 → 2/3, 1/3
        let result = compute_user_vector(&[w1, w2], &[2.0, 1.0]);
        assert!((result[0] - 2.0 / 3.0).abs() < 1e-10);
        assert!((result[1] - 1.0 / 3.0).abs() < 1e-10);
    }

    #[test]
    fn compute_user_vector_skips_mismatched_dimensions() {
        let w1 = vec![0.5, 0.5];
        let w2 = vec![0.5, 0.5, 0.5]; // mismatched
        let result = compute_user_vector(&[w1, w2], &[1.0, 1.0]);
        assert_eq!(result, vec![0.5, 0.5]);
    }

    #[test]
    fn update_vector_incremental_one_step() {
        let work = vec![0.8, 0.6];
        let mut current = vec![0.0, 0.0];
        let new_sum = update_vector_incremental(&mut current, &work, 1.0, 0.0);
        assert!((new_sum - 1.0).abs() < 1e-10);
        assert!((current[0] - 0.8).abs() < 1e-10);
        assert!((current[1] - 0.6).abs() < 1e-10);
    }

    #[test]
    fn update_vector_incremental_converges() {
        let work = vec![1.0, 0.0];
        let mut current = vec![0.0, 0.0];
        let mut sum = 0.0;
        for _ in 0..100 {
            sum = update_vector_incremental(&mut current, &work, 1.0, sum);
        }
        // After many identical updates, should converge to work vector
        assert!((current[0] - 1.0).abs() < 1e-10);
        assert!((current[1] - 0.0).abs() < 1e-10);
    }

    #[test]
    fn update_vector_incremental_skips_mismatched() {
        let work = vec![0.8, 0.6, 0.4]; // 3 dims
        let mut current = vec![0.0, 0.0]; // 2 dims
        let new_sum = update_vector_incremental(&mut current, &work, 1.0, 0.0);
        assert!((new_sum - 1.0).abs() < 1e-10);
        // current unchanged because dimensions mismatch
        assert!((current[0] - 0.0).abs() < 1e-10);
    }

    #[test]
    fn blend_vectors_boundaries() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let t0 = blend_vectors(&a, &b, 0.0);
        let t1 = blend_vectors(&a, &b, 1.0);
        assert_eq!(t0, a);
        assert_eq!(t1, b);
    }

    #[test]
    fn blend_vectors_midpoint() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let mid = blend_vectors(&a, &b, 0.5);
        assert!((mid[0] - 0.5).abs() < 1e-10);
        assert!((mid[1] - 0.5).abs() < 1e-10);
    }

    // ================================================================
    // New tests for taste-gravitational system (§16.17)
    // ================================================================

    #[test]
    fn compute_resonance_default_weights() {
        let components = ResonanceComponents {
            bookmark_overlap: 0.8,
            rating_correlation: 0.6,
            completion_alignment: 0.9,
            reading_time_ratio: 0.7,
        };
        let weights = ResonanceWeights::default();
        let score = compute_resonance(&components, &weights);
        // 0.8*0.3 + 0.6*0.25 + 0.9*0.25 + 0.7*0.2 = 0.24 + 0.15 + 0.225 + 0.14 = 0.755
        assert!((score - 0.755).abs() < 1e-10);
    }

    #[test]
    fn compute_resonance_clamps_high() {
        let components = ResonanceComponents {
            bookmark_overlap: 2.0,
            rating_correlation: 2.0,
            completion_alignment: 2.0,
            reading_time_ratio: 2.0,
        };
        let weights = ResonanceWeights::default();
        assert_eq!(compute_resonance(&components, &weights), 1.0);
    }

    #[test]
    fn qualitative_label_mapping() {
        assert_eq!(qualitative_label(0.8), ResonanceLabel::Strong);
        assert_eq!(qualitative_label(0.6), ResonanceLabel::Moderate);
        assert_eq!(qualitative_label(0.4), ResonanceLabel::Weak);
        assert_eq!(qualitative_label(0.1), ResonanceLabel::Neutral);
        assert_eq!(qualitative_label(0.0), ResonanceLabel::Dissonant);
    }

    #[test]
    fn resonance_label_as_str() {
        assert_eq!(ResonanceLabel::Strong.as_str(), "Aligned: Strong");
        assert_eq!(ResonanceLabel::Moderate.as_str(), "Aligned: Moderate");
        assert_eq!(ResonanceLabel::Weak.as_str(), "Aligned: Weak");
        assert_eq!(ResonanceLabel::Neutral.as_str(), "Neutral");
        assert_eq!(ResonanceLabel::Dissonant.as_str(), "Divergent");
    }

    #[test]
    fn jaccard_similarity_identical() {
        let a = vec!["a".to_string(), "b".to_string()];
        assert!((jaccard_similarity(&a, &a) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn jaccard_similarity_disjoint() {
        let a = vec!["a".to_string()];
        let b = vec!["b".to_string()];
        assert!((jaccard_similarity(&a, &b) - 0.0).abs() < 1e-10);
    }

    #[test]
    fn jaccard_similarity_partial() {
        let a = vec!["a".to_string(), "b".to_string()];
        let b = vec!["b".to_string(), "c".to_string()];
        // intersection = {b}, union = {a, b, c} → 1/3
        assert!((jaccard_similarity(&a, &b) - 1.0 / 3.0).abs() < 1e-10);
    }

    #[test]
    fn jaccard_similarity_empty() {
        let a: Vec<String> = vec![];
        let b: Vec<String> = vec![];
        assert!((jaccard_similarity(&a, &b) - 0.0).abs() < 1e-10);
    }

    #[test]
    fn profile_resonance_identical() {
        let profile = TasteProfile {
            dimensions: vec![TasteDimension {
                key: "angst".to_string(),
                label: "Angst".to_string(),
                admin_target: 0.5,
                weight: 1.0,
            }],
            exemplars: vec![],
            anti_examples: vec![],
        };
        let user = TasteVector {
            dimensions: vec![0.5],
        };
        assert!((profile_resonance(&user, &profile) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn profile_resonance_distant() {
        let profile = TasteProfile {
            dimensions: vec![TasteDimension {
                key: "angst".to_string(),
                label: "Angst".to_string(),
                admin_target: 0.0,
                weight: 1.0,
            }],
            exemplars: vec![],
            anti_examples: vec![],
        };
        let user = TasteVector {
            dimensions: vec![1.0],
        };
        let r = profile_resonance(&user, &profile);
        // distance = tanh(1.0 / 2.0) ≈ 0.462, resonance = 1 - 0.462 ≈ 0.538
        assert!(r < 0.6);
        assert!(r > 0.4);
    }

    #[test]
    fn distance_to_profile_mismatched_dims() {
        let profile = TasteProfile {
            dimensions: vec![TasteDimension {
                key: "a".to_string(),
                label: "A".to_string(),
                admin_target: 0.5,
                weight: 1.0,
            }],
            exemplars: vec![],
            anti_examples: vec![],
        };
        let user = TasteVector {
            dimensions: vec![0.5, 0.5],
        };
        assert_eq!(distance_to_profile(&user, &profile), 1.0);
    }
}

// ---------------------------------------------------------------------------
// Taste Calibration Arena (spec §0.4.2a)
// ---------------------------------------------------------------------------

/// A card shown in the arena: a work with metadata + excerpt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArenaCard {
    pub work_id: String,
    pub title: String,
    pub fandom: String,
    pub tags: Vec<String>,
    pub word_count: u32,
    pub excerpt: String,
    /// Dimension this card is designed to vary on (for active learning).
    pub target_dimension: String,
}

/// An arena round: 4 cards to compare.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArenaRound {
    pub cards: Vec<ArenaCard>,
}

/// A ballot: which card was best and which was worst.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArenaBallot {
    pub best_work_id: String,
    pub worst_work_id: String,
    pub reason_tags: Vec<String>,
}

/// Per-dimension Elo rating for arena calibration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DimensionElo {
    pub dimension_key: String,
    pub elo_rating: f64,
    pub matches_played: u32,
}

/// Generate an arena round from a pool of works.
///
/// Picks 4 works that share at least one major attribute (fandom, genre, or
/// length bracket) and maximizes variance on 1-2 target dimensions.
///
/// `pool` should be pre-filtered to works the account hasn't already voted on.
/// `dimensions` are the instance's taste dimensions (from TasteProfile).
/// `target_dimensions` are the dimensions this round should vary on (active
/// learning picks the dimensions where the model is most uncertain).
pub fn generate_arena_round(
    pool: &[ArenaCard],
    _dimensions: &[TasteDimension], // reserved for dimension weighting
    target_dimensions: &[String],
) -> Option<ArenaRound> {
    if pool.len() < 4 {
        return None;
    }

    // Group by shared attribute (fandom first, then length bracket).
    let mut by_fandom: std::collections::HashMap<String, Vec<&ArenaCard>> =
        std::collections::HashMap::new();
    for card in pool {
        by_fandom.entry(card.fandom.clone()).or_default().push(card);
    }

    // Find a fandom group with at least 4 cards.
    let group = by_fandom.values().find(|g| g.len() >= 4)?;

    // Pick 4 cards that maximize variance on the target dimensions.
    // Simple greedy: pick the card with highest variance on target dimensions,
    // then pick 3 more that are most different from the first.
    let mut selected: Vec<&ArenaCard> = Vec::new();
    let mut remaining: Vec<&ArenaCard> = group.clone();

    // Start with the card that has the most extreme position on the first
    // target dimension (highest or lowest — either is fine, we want spread).
    if let Some(first) = remaining.iter().max_by(|a, b| {
        dimension_score(a, &target_dimensions[0])
            .partial_cmp(&dimension_score(b, &target_dimensions[0]))
            .unwrap_or(std::cmp::Ordering::Equal)
    }) {
        selected.push(*first);
        remaining.retain(|c| c.work_id != first.work_id);
    }

    // Pick 3 more that are most different from the first.
    while selected.len() < 4 && !remaining.is_empty() {
        let first = selected[0];
        let next = remaining
            .iter()
            .max_by(|a, b| {
                let da = card_distance(a, first, &target_dimensions);
                let db = card_distance(b, first, &target_dimensions);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned()?;
        selected.push(next);
        remaining.retain(|c| c.work_id != next.work_id);
    }

    if selected.len() < 4 {
        return None;
    }

    Some(ArenaRound {
        cards: selected.into_iter().cloned().collect(),
    })
}

/// Apply an arena ballot to update per-dimension Elo ratings.
///
/// The ballot gives a partial ranking: best > {middle two} > worst.
/// We translate this to virtual 1v1 Elo matches:
///   - best beats middle1, middle2, worst
///   - middle1 beats worst
///   - middle2 beats worst
/// Each match updates the Elo of the dimension that most differentiates the
/// two cards.
///
/// Returns updated Elo ratings for all dimensions.
pub fn apply_arena_ballot(
    elos: &[DimensionElo],
    round: &ArenaRound,
    ballot: &ArenaBallot,
) -> Vec<DimensionElo> {
    let mut updated: Vec<DimensionElo> = elos.to_vec();

    // Find best and worst cards.
    let best = round
        .cards
        .iter()
        .find(|c| c.work_id == ballot.best_work_id);
    let worst = round
        .cards
        .iter()
        .find(|c| c.work_id == ballot.worst_work_id);
    let (Some(best), Some(worst)) = (best, worst) else {
        return updated;
    };

    // Determine which dimension most differentiates best from worst.
    let target_dim = if !ballot.reason_tags.is_empty() {
        // Use the first reason tag as the differentiating dimension.
        ballot.reason_tags[0].clone()
    } else {
        // Fall back to the card's target dimension.
        best.target_dimension.clone()
    };

    // Update Elo for the target dimension.
    // best beats worst → best's dimension Elo increases, worst's decreases.
    // But since we track per-dimension (not per-card) Elo, we update the
    // dimension's Elo based on whether the "preferred" direction won.
    //
    // Simplified model: the dimension's Elo represents how strongly the
    // reader weights this dimension. A win for the card that scores higher
    // on this dimension → the dimension matters more → Elo up.
    let best_score = dimension_score(best, &target_dim);
    let worst_score = dimension_score(worst, &target_dim);

    if let Some(dim_elo) = updated.iter_mut().find(|d| d.dimension_key == target_dim) {
        let expected = 1.0 / (1.0 + 10f64.powf(-(dim_elo.elo_rating - 1500.0) / 400.0));
        let actual = if best_score > worst_score { 1.0 } else { 0.0 };
        let k = 32.0;
        dim_elo.elo_rating += k * (actual - expected);
        dim_elo.matches_played += 1;
    }

    updated
}

/// Compute a card's score on a given dimension.
///
/// This is a placeholder — in production, this would use the work's actual
/// dimensional scores (from the taste profile's dimension analysis).
/// For now, we use a hash of the work_id + dimension as a deterministic
/// pseudo-score, so the arena can be tested end-to-end.
fn dimension_score(card: &ArenaCard, dimension: &str) -> f64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    card.work_id.hash(&mut hasher);
    dimension.hash(&mut hasher);
    let hash = hasher.finish();
    (hash % 1000) as f64 / 1000.0
}

/// Compute distance between two cards on the target dimensions.
fn card_distance(a: &ArenaCard, b: &ArenaCard, dimensions: &[String]) -> f64 {
    dimensions
        .iter()
        .map(|d| (dimension_score(a, d) - dimension_score(b, d)).abs())
        .sum()
}

/// Compute the weight vector from Elo ratings.
///
/// Higher Elo = more matches played with consistent outcomes = more confident
/// weight. We normalize to sum to 1.0.
pub fn weights_from_elos(elos: &[DimensionElo]) -> Vec<(String, f64)> {
    if elos.is_empty() {
        return vec![];
    }

    let raw: Vec<(String, f64)> = elos
        .iter()
        .map(|e| {
            // Weight = Elo above baseline (1500), scaled.
            // A dimension with Elo 1600 after 10 matches is more reliable
            // than one with Elo 1600 after 2 matches.
            let confidence = (e.matches_played as f64).min(40.0) / 40.0;
            let weight = ((e.elo_rating - 1500.0) / 100.0 + 1.0) * confidence;
            (e.dimension_key.clone(), weight.max(0.01))
        })
        .collect();

    let total: f64 = raw.iter().map(|(_, w)| w).sum();
    raw.into_iter().map(|(k, w)| (k, w / total)).collect()
}

#[cfg(test)]
mod arena_tests {
    use super::*;

    fn test_card(id: &str, fandom: &str, dim: &str) -> ArenaCard {
        ArenaCard {
            work_id: id.to_string(),
            title: format!("Work {}", id),
            fandom: fandom.to_string(),
            tags: vec!["test".to_string()],
            word_count: 5000,
            excerpt: "A test excerpt.".to_string(),
            target_dimension: dim.to_string(),
        }
    }

    #[test]
    fn generate_arena_round_requires_four_cards_same_fandom() {
        let pool = vec![
            test_card("1", "fandom_a", "prose"),
            test_card("2", "fandom_a", "prose"),
            test_card("3", "fandom_a", "prose"),
            test_card("4", "fandom_a", "prose"),
        ];
        let dims = vec![TasteDimension {
            key: "prose".to_string(),
            label: "Prose".to_string(),
            admin_target: 0.5,
            weight: 1.0,
        }];
        let round = generate_arena_round(&pool, &dims, &["prose".to_string()]);
        assert!(round.is_some());
        assert_eq!(round.unwrap().cards.len(), 4);
    }

    #[test]
    fn generate_arena_round_rejects_small_pool() {
        let pool = vec![test_card("1", "fandom_a", "prose")];
        let dims = vec![];
        let round = generate_arena_round(&pool, &dims, &["prose".to_string()]);
        assert!(round.is_none());
    }

    #[test]
    fn apply_arena_ballot_updates_elo() {
        let round = ArenaRound {
            cards: vec![
                test_card("1", "fandom_a", "prose"),
                test_card("2", "fandom_a", "prose"),
                test_card("3", "fandom_a", "prose"),
                test_card("4", "fandom_a", "prose"),
            ],
        };
        let elos = vec![DimensionElo {
            dimension_key: "prose".to_string(),
            elo_rating: 1500.0,
            matches_played: 0,
        }];
        let ballot = ArenaBallot {
            best_work_id: "1".to_string(),
            worst_work_id: "4".to_string(),
            reason_tags: vec!["prose".to_string()],
        };
        let updated = apply_arena_ballot(&elos, &round, &ballot);
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].matches_played, 1);
        // Elo should have changed (either up or down).
        assert_ne!(updated[0].elo_rating, 1500.0);
    }

    #[test]
    fn weights_from_elos_normalizes() {
        let elos = vec![
            DimensionElo {
                dimension_key: "prose".to_string(),
                elo_rating: 1600.0,
                matches_played: 10,
            },
            DimensionElo {
                dimension_key: "pacing".to_string(),
                elo_rating: 1400.0,
                matches_played: 5,
            },
        ];
        let weights = weights_from_elos(&elos);
        assert_eq!(weights.len(), 2);
        let sum: f64 = weights.iter().map(|(_, w)| w).sum();
        assert!((sum - 1.0).abs() < 0.001);
    }

    #[test]
    fn weights_from_elos_empty() {
        let weights = weights_from_elos(&[]);
        assert!(weights.is_empty());
    }
}
