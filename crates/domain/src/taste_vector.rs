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
            dimensions: vec![
                TasteDimension {
                    key: "angst".to_string(),
                    label: "Angst".to_string(),
                    admin_target: 0.5,
                    weight: 1.0,
                },
            ],
            exemplars: vec![],
            anti_examples: vec![],
        };
        let user = TasteVector { dimensions: vec![0.5] };
        assert!((profile_resonance(&user, &profile) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn profile_resonance_distant() {
        let profile = TasteProfile {
            dimensions: vec![
                TasteDimension {
                    key: "angst".to_string(),
                    label: "Angst".to_string(),
                    admin_target: 0.0,
                    weight: 1.0,
                },
            ],
            exemplars: vec![],
            anti_examples: vec![],
        };
        let user = TasteVector { dimensions: vec![1.0] };
        let r = profile_resonance(&user, &profile);
        // distance = tanh(1.0 / 2.0) ≈ 0.462, resonance = 1 - 0.462 ≈ 0.538
        assert!(r < 0.6);
        assert!(r > 0.4);
    }

    #[test]
    fn distance_to_profile_mismatched_dims() {
        let profile = TasteProfile {
            dimensions: vec![
                TasteDimension {
                    key: "a".to_string(),
                    label: "A".to_string(),
                    admin_target: 0.5,
                    weight: 1.0,
                },
            ],
            exemplars: vec![],
            anti_examples: vec![],
        };
        let user = TasteVector { dimensions: vec![0.5, 0.5] };
        assert_eq!(distance_to_profile(&user, &profile), 1.0);
    }
}
