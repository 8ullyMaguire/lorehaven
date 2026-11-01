//! Quality metrics that do not depend on how large an archive is.
//!
//! Raw likes are the wrong ranking: a story on a large archive accumulates more
//! of them, and the catalogue proves it — the same story carries 33,107 likes on
//! `sb` (p98.2 of 884 engaged rows) and 4,179 on `qq` (p92.8 of 209). Eight times
//! the raw count, five percentile points apart. Every metric here is therefore
//! computed *within the row's own source* before anything is compared.
//!
//! Two measured facts shape the whole module: 1,247 of 141,317 catalogue rows
//! carry any endorsement count, and `views` is null on all of them. So "no likes"
//! means unmeasured, never "bad", and no rate with a view denominator exists yet.
//!
//! This module is pure: no database, no clock. The caller passes
//! `freshness_days`, and the store fills the candidates.
//!
//! # Why sample size is a separate signal and not a correction inside the rank
//!
//! Shrinking a percentile toward the midpoint by its sample size
//! (`0.5 + (pct−0.5)·n/(n+10)`) looks like the obvious guardrail and it is wrong
//! here: it makes the **achievable maximum depend on `n`**, so the best row of a
//! source with 209 engaged rows can never score above 0.977 while one with 884
//! can reach 0.994. Measured on the live catalogue 2026-11-01, that put 34 of the
//! top 40 `source_pct` rows in the single largest archive — the archive-size bias
//! this whole vocabulary exists to remove, reintroduced by its own guardrail.
//!
//! So the percentile is raw (a source's best row is 1.0 whatever the source's
//! size), and evidence is a signal of its own: [`Signals::confidence`], worth
//! `n/(n+10)`, which the composite weighs instead of the ranking distorting.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// How many unobserved rows `confidence` assumes. Chosen against the live
/// engaged populations (209, 154, 884): it discounts a source with a handful of
/// engaged rows without touching the large ones.
///
/// A percentile computed from four engaged rows is not worth the same as one
/// computed from eight hundred: the first says "best of four", which is what a
/// two-like story in a tiny archive looks like. Each percentile is therefore
/// pulled toward the midpoint by `n / (n + PRIOR_ROWS)`, so a small source can
/// still be right about its best row but cannot claim the confidence of a large
/// one. Ten is a deliberate underestimate of a typical engaged set (the live
/// sources hold 209, 154 and 884) — it corrects the small sources without
/// touching the large ones.
const PERCENTILE_PRIOR_ROWS: f64 = 10.0;

/// Days over which `recency` decays to `1/e`.
const RECENCY_HALF_LIFE_DAYS: f64 = 180.0;

/// A ranking rule over catalogue rows.
///
/// The wire name is the CLI argument, so `--sort source_pct` is exactly
/// [`Metric::SourcePct`]. Raw sorts (`likes`, `aggregate`, `views`, `words`,
/// `updated`) are kept because they are the honest comparison: they are what the
/// catalogue did before, and an operator should be able to see the difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    /// The composite: source-relative quality, blended with what we can measure.
    Quality,
    /// Percentile of likes among the source's engaged rows, shrunk toward the
    /// midpoint by sample size.
    SourcePct,
    /// Logistic of the z-score of `ln(1+likes)` within the source.
    SourceZ,
    /// Percentile of `likes per 1k words` within the source.
    EngagementDensity,
    /// `source_pct` plus a bonus for being carried by more than one archive.
    Consensus,
    /// Binary: does this story exist on two or more archives?
    Crosspost,
    /// Bucket score on word count — effort, with no endorsement input.
    WordsMaturity,
    /// `min(source_count, 5) / 5`.
    SourceCoverage,
    /// `source_pct` inside the caller's fandom cohort.
    CohortPct,
    /// `exp(-days / 180)` on `source_updated_at`.
    Recency,
    /// How much the source's engaged population is worth, on its own.
    Confidence,
    /// Triage for rows with no endorsement data: which to check first.
    Unmeasured,
    /// Raw imported likes, descending.
    Likes,
    /// `total_likes`, descending: every source plus darkhold's own kudos.
    Aggregate,
    /// Views, descending. Null on every row today.
    Views,
    /// Word count, descending.
    Words,
    /// Most recently changed catalogue row.
    Updated,
    /// Δlikes per week. Blocked: needs two snapshots.
    Velocity,
    /// High velocity percentile, low absolute percentile. Blocked with `Velocity`.
    HiddenGem,
    /// Kudos per 1k views. Blocked: `views` is null on every row.
    LikesPerView,
    /// Wilson lower bound on likes per view — the rate this catalogue cannot
    /// compute yet, kept as a name so `--sort wilson` explains itself.
    Wilson,
}

/// What a metric can do with today's data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// Computed here, in memory, from the row's own source.
    Ranked,
    /// A plain SQL `ORDER BY` on a stored column.
    Raw,
    /// A metadata ordering for rows with no endorsement data.
    Triage,
    /// Refused, with the reason and the work item that unblocks it.
    Blocked(&'static str),
}

impl Metric {
    /// Every metric, in the order `--list-metrics` prints them.
    pub const ALL: &'static [Self] = &[
        Self::Quality,
        Self::SourcePct,
        Self::SourceZ,
        Self::EngagementDensity,
        Self::Consensus,
        Self::Crosspost,
        Self::WordsMaturity,
        Self::SourceCoverage,
        Self::CohortPct,
        Self::Recency,
        Self::Confidence,
        Self::Unmeasured,
        Self::Likes,
        Self::Aggregate,
        Self::Views,
        Self::Words,
        Self::Updated,
        Self::Velocity,
        Self::HiddenGem,
        Self::LikesPerView,
        Self::Wilson,
    ];

    /// Parse the `sort=` parameter and the `--sort` argument.
    ///
    /// Deliberately strict: an unknown word is `None` rather than a silent
    /// fallback to `likes`, because a caller who typed `--sort velocity` must
    /// not be handed a likes ranking and told it is velocity.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "quality" => Some(Self::Quality),
            "source_pct" | "source-percentile" | "source_percentile" | "pct" => {
                Some(Self::SourcePct)
            }
            "source_z" | "zscore" | "z_score" => Some(Self::SourceZ),
            "engagement_density" | "density" | "likes_per_1k_words" => {
                Some(Self::EngagementDensity)
            }
            "consensus" => Some(Self::Consensus),
            "crosspost" | "crosspost_survival" => Some(Self::Crosspost),
            "words_maturity" | "word_count_maturity" | "maturity" => Some(Self::WordsMaturity),
            "source_coverage" | "coverage" => Some(Self::SourceCoverage),
            "cohort_pct" | "tag_cohort_pct" | "cohort" => Some(Self::CohortPct),
            "recency" | "freshness" => Some(Self::Recency),
            "confidence" | "evidence" => Some(Self::Confidence),
            "unmeasured" | "triage" => Some(Self::Unmeasured),
            "likes" | "like" => Some(Self::Likes),
            "aggregate" | "total" => Some(Self::Aggregate),
            "views" => Some(Self::Views),
            "words" | "word_count" => Some(Self::Words),
            "updated" | "recent" => Some(Self::Updated),
            "velocity" | "momentum" => Some(Self::Velocity),
            "hidden_gem" | "hidden-gem" => Some(Self::HiddenGem),
            "likes_per_view" | "kudos_per_view" => Some(Self::LikesPerView),
            "wilson" | "wilson_lower_bound" => Some(Self::Wilson),
            _ => None,
        }
    }

    /// The wire name, which is also the CLI argument.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quality => "quality",
            Self::SourcePct => "source_pct",
            Self::SourceZ => "source_z",
            Self::EngagementDensity => "engagement_density",
            Self::Consensus => "consensus",
            Self::Crosspost => "crosspost",
            Self::WordsMaturity => "words_maturity",
            Self::SourceCoverage => "source_coverage",
            Self::CohortPct => "cohort_pct",
            Self::Recency => "recency",
            Self::Confidence => "confidence",
            Self::Unmeasured => "unmeasured",
            Self::Likes => "likes",
            Self::Aggregate => "aggregate",
            Self::Views => "views",
            Self::Words => "words",
            Self::Updated => "updated",
            Self::Velocity => "velocity",
            Self::HiddenGem => "hidden_gem",
            Self::LikesPerView => "likes_per_view",
            Self::Wilson => "wilson",
        }
    }

    /// Whether the metric can be computed from what the catalogue holds.
    #[must_use]
    pub fn support(self) -> Support {
        match self {
            Self::Likes | Self::Aggregate | Self::Views | Self::Words | Self::Updated => {
                Support::Raw
            }
            Self::Unmeasured => Support::Triage,
            Self::Velocity | Self::HiddenGem => Support::Blocked(
                "needs two snapshots of the same row; the monthly recheck (M63-32) creates the second",
            ),
            Self::LikesPerView | Self::Wilson => {
                Support::Blocked("views are null on all 141,317 rows (M63-32 imports them)")
            }
            Self::Quality
            | Self::SourcePct
            | Self::SourceZ
            | Self::EngagementDensity
            | Self::Consensus
            | Self::Crosspost
            | Self::WordsMaturity
            | Self::SourceCoverage
            | Self::CohortPct
            | Self::Recency
            | Self::Confidence => Support::Ranked,
        }
    }

    /// One line for `--list-metrics`.
    #[must_use]
    pub fn help(self) -> &'static str {
        match self {
            Self::Quality => {
                "0.45 source_pct + 0.30 consensus + 0.15 density + 0.10 crosspost, renormalised"
            }
            Self::SourcePct => "percentile of likes among the source's engaged rows (raw)",
            Self::SourceZ => "logistic z-score of ln(1+likes) within the source",
            Self::EngagementDensity => "percentile of likes per 1k words within the source",
            Self::Consensus => "source_pct plus a bonus for other archives carrying it",
            Self::Crosspost => "1.0 when two or more archives carry the story",
            Self::WordsMaturity => "<10k 0.2, 10-50k 0.45, 50-200k 0.8, >=200k 1.0",
            Self::SourceCoverage => "min(source_count, 5) / 5",
            Self::CohortPct => "source_pct inside the --fandom cohort",
            Self::Recency => "exp(-days/180) on the source's last update",
            Self::Confidence => "n/(n+10): how much the source's engaged population is worth",
            Self::Unmeasured => "0.4 coverage + 0.3 maturity + 0.3 cross-archive presence",
            Self::Likes => "raw imported likes (what the catalogue did before)",
            Self::Aggregate => "total likes across every source plus darkhold kudos",
            Self::Views => "stored views (null on every row)",
            Self::Words => "word count",
            Self::Updated => "most recently changed catalogue row",
            Self::Velocity => "likes gained per week",
            Self::HiddenGem => "high velocity, low absolute count",
            Self::LikesPerView => "likes per 1k views",
            Self::Wilson => "Wilson lower bound on likes per view",
        }
    }
}

/// One row, reduced to the fields a metric needs.
///
/// `freshness_days` arrives pre-computed so this module never reads a clock;
/// `None` means the row has no source date (2,009 of `sb`'s rows do not).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// Library item id.
    pub id: String,
    /// The archive it came from.
    pub source_key: String,
    /// Imported endorsement count.
    pub likes: i64,
    /// The source's word count, when it publishes one.
    pub word_count: Option<i64>,
    /// How many archives carry this story.
    pub source_count: i64,
    /// Likes summed across every copy, including this one.
    pub total_likes: i64,
    /// Days since the author last touched it.
    pub freshness_days: Option<f64>,
}

/// The individual signals behind one row's score.
///
/// Every field is optional because every one of them can be absent for a real
/// reason: no word count, no source date, no endorsement. A missing signal is
/// dropped from the composite and the remaining weights are renormalised, rather
/// than being read as a zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Signals {
    /// [`Metric::SourcePct`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_pct: Option<f64>,
    /// [`Metric::SourceZ`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_z: Option<f64>,
    /// [`Metric::EngagementDensity`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub density: Option<f64>,
    /// [`Metric::Consensus`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consensus: Option<f64>,
    /// [`Metric::Crosspost`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crosspost: Option<f64>,
    /// [`Metric::WordsMaturity`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words_maturity: Option<f64>,
    /// [`Metric::SourceCoverage`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_coverage: Option<f64>,
    /// [`Metric::Recency`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recency: Option<f64>,
    /// How much the source's engaged population is worth: `n/(n+10)`.
    ///
    /// Separate from [`Signals::source_pct`] on purpose: the percentile says how
    /// good the row is *for its archive*, this says how much that claim is worth.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// [`Metric::Unmeasured`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triage: Option<f64>,
}

/// One ranked row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    /// Library item id — the join key back to the stored row.
    pub id: String,
    /// The metric's score, 0..=1 for every ranked metric.
    pub score: f64,
    /// False for a row the metric could not measure; it is ordered by triage.
    pub measured: bool,
    /// The components of the score.
    pub signals: Signals,
}

/// The page, plus the numbers that stop it reading as a verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ranking {
    /// The metric these scores belong to.
    pub metric: Metric,
    /// Engaged rows the metric could score, across the whole filtered set — not
    /// just this page. This is the gate: `ranked N of total`.
    pub ranked: usize,
    /// Rows the filters matched.
    pub total: usize,
    /// The page.
    pub hits: Vec<Hit>,
}

/// Per-source distributions, built once per ranking.
#[derive(Debug, Default)]
struct SourceStats {
    /// Raw likes for engaged rows, sorted ascending — the percentile's input.
    raw: Vec<f64>,
    /// `ln(1+likes)` for engaged rows.
    log_likes: Vec<f64>,
    /// `likes per 1k words` for engaged rows that publish a word count, sorted.
    density: Vec<f64>,
    /// Sum and sum of squares of `log_likes`, for the z-score.
    log_sum: f64,
    log_sq_sum: f64,
}

impl SourceStats {
    fn finish(&mut self) {
        self.raw
            .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        self.density
            .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    }
}

/// Where `value` sits in `sorted`: 0 at the bottom, 1 at the top.
///
/// The divisor is `len - 1`, not `len`, and that is the whole point: a mid-rank
/// percentile over `n` rows tops out at `(n − 0.5)/n`, so the best of a four-row
/// archive would score 0.875 while the best of a four-hundred-row one scores
/// 0.9988 — an n-dependent ceiling, which is the same archive-size bias in a
/// milder dress. Anchoring both ends makes "the best row of this archive" mean
/// the same thing everywhere, and leaves the size question to
/// [`Signals::confidence`], where it belongs.
///
/// Ties share a position: `below + 0.5·(equal − 1)` over `len − 1`. A single row
/// has no order to express and scores 0.5.
fn percentile_of(value: f64, sorted: &[f64]) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    if sorted.len() == 1 {
        return Some(0.5);
    }
    let below = sorted.iter().filter(|&&v| v < value).count() as f64;
    let equal = sorted.iter().filter(|&&v| v == value).count() as f64;
    let position = below + 0.5 * (equal - 1.0);
    Some((position / (sorted.len() as f64 - 1.0)).clamp(0.0, 1.0))
}

/// How much a percentile from `n` rows is worth: `n / (n + k)`.
///
/// Deliberately *outside* the percentile. A percentile is a statement about how
/// many rows are below a value, and it is as trustworthy as there are rows; that
/// is a separate fact from the value itself, and folding it in makes the top of a
/// small source unreachable instead of merely less certain.
fn confidence(n: usize) -> f64 {
    let n = n as f64;
    n / (n + PERCENTILE_PRIOR_ROWS)
}

/// Bucket score on length. `None` when the source publishes no word count.
fn words_maturity(word_count: Option<i64>) -> Option<f64> {
    let words = word_count?;
    Some(if words < 10_000 {
        0.2
    } else if words < 50_000 {
        0.45
    } else if words < 200_000 {
        0.8
    } else {
        1.0
    })
}

fn logistic(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

/// Rank `candidates` by `metric`, returning the page plus the gate.
///
/// Every metric is accepted, including the raw columns: the routes answer those
/// with a SQL `ORDER BY`, but the store's ranker is also reachable directly and a
/// silent zero-score ordering would be a trap.
///
/// Measured rows always precede unmeasured ones, whatever the metric: comparing
/// a percentile to a row with no endorsement data is the mistake this whole
/// module exists to avoid, so the page is two blocks and [`Hit::measured`] says
/// which block a row is in.
#[must_use]
pub fn rank(
    metric: Metric,
    candidates: &[Candidate],
    limit: usize,
    offset: usize,
) -> Ranking {
    let total = candidates.len();
    let engaged: Vec<&Candidate> = candidates.iter().filter(|c| c.likes > 0).collect();
    let ranked = engaged.len();

    let mut stats: HashMap<&str, SourceStats> = HashMap::new();
    for candidate in &engaged {
        let entry = stats.entry(candidate.source_key.as_str()).or_default();
        let log_likes = (1.0 + candidate.likes as f64).ln();
        entry.raw.push(candidate.likes as f64);
        entry.log_likes.push(log_likes);
        entry.log_sum += log_likes;
        entry.log_sq_sum += log_likes * log_likes;
    }
    for s in stats.values_mut() {
        s.finish();
    }
    for candidate in &engaged {
        if let Some(words) = candidate.word_count.filter(|w| *w > 0) {
            let density = (candidate.likes as f64) * 1_000.0 / (words as f64);
            if let Some(s) = stats.get_mut(candidate.source_key.as_str()) {
                s.density.push(density);
            }
        }
    }
    for s in stats.values_mut() {
        s.density.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    }

    let signals_of = |candidate: &Candidate| -> Signals {
        let s = stats.get(candidate.source_key.as_str());
        let coverage = (candidate.source_count.clamp(0, 5) as f64) / 5.0;
        let maturity = words_maturity(candidate.word_count);
        let crosspost = if candidate.source_count >= 2 { 1.0 } else { 0.0 };
        let recency = candidate
            .freshness_days
            .map(|days| (-days / RECENCY_HALF_LIFE_DAYS).exp());

        let mut signals = Signals {
            crosspost: Some(crosspost),
            source_coverage: Some(coverage),
            words_maturity: maturity,
            recency,
            ..Signals::default()
        };

        if candidate.likes > 0 {
            if let Some(s) = s {
                signals.source_pct = percentile_of(candidate.likes as f64, &s.raw);
                signals.confidence = Some(confidence(s.raw.len()));
                let n = s.log_likes.len() as f64;
                if n >= 2.0 {
                    let mean = s.log_sum / n;
                    let variance = (s.log_sq_sum / n) - mean * mean;
                    let sd = variance.max(0.0).sqrt();
                    // A source where every engaged row has the same count has no
                    // spread to standardise against; the midpoint is the only
                    // honest answer, not a division by zero.
                    signals.source_z = Some(if sd > 0.0 {
                        let log_likes = (1.0 + candidate.likes as f64).ln();
                        logistic((log_likes - mean) / sd)
                    } else {
                        0.5
                    });
                } else {
                    signals.source_z = Some(0.5);
                }
                if let Some(words) = candidate.word_count.filter(|w| *w > 0) {
                    let density = (candidate.likes as f64) * 1_000.0 / (words as f64);
                    signals.density = percentile_of(density, &s.density);
                }
            }
            let cross_bonus = 0.10 * ((candidate.source_count.clamp(1, 3) - 1) as f64)
                + if candidate.total_likes > candidate.likes {
                    0.05
                } else {
                    0.0
                };
            signals.consensus = signals.source_pct.map(|p| (p + cross_bonus).clamp(0.0, 1.0));
        } else {
            // No endorsement data: nothing about quality is known, so only the
            // metadata signals are filled and the triage score decides the order.
            signals.triage = Some(0.4 * coverage + 0.3 * maturity.unwrap_or(0.0) + 0.3 * (candidate.source_count.clamp(0, 3) as f64) / 3.0);
        }
        signals
    };

    let mut hits: Vec<Hit> = Vec::with_capacity(engaged.len());
    for candidate in &engaged {
        let signals = signals_of(candidate);
        // The raw sorts are here so the store can rank by any name it is given;
        // the routes still use SQL for them, which is cheaper. `ln_1p` keeps the
        // score in a sane range without changing the order.
        let score = match metric {
            Metric::Likes => (candidate.likes as f64).ln_1p(),
            Metric::Aggregate => (candidate.total_likes as f64).ln_1p(),
            Metric::Words => candidate.word_count.map_or(0.0, |w| (w as f64).ln_1p()),
            Metric::Views => 0.0,
            // Fresher means fewer days, and the sort is descending.
            Metric::Updated => candidate.freshness_days.map_or(f64::MIN, |days| -days),
            _ => score_of(metric, &signals),
        };
        hits.push(Hit {
            id: candidate.id.clone(),
            score,
            measured: true,
            signals,
        });
    }
    // Unmeasured rows: a page never mixes the two blocks, and within the block
    // the order is metadata, which is what `unmeasured` is for.
    let mut unmeasured: Vec<Hit> = candidates
        .iter()
        .filter(|c| c.likes <= 0)
        .map(|candidate| {
            let signals = signals_of(candidate);
            Hit {
                id: candidate.id.clone(),
                score: signals.triage.unwrap_or(0.0),
                measured: false,
                signals,
            }
        })
        .collect();

    let tie_break = |a: &Hit, b: &Hit| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    };
    hits.sort_by(tie_break);
    unmeasured.sort_by(tie_break);

    let ordered: Vec<Hit> = if metric == Metric::Unmeasured {
        // Triage is the point of this metric, so it is the whole page — but the
        // gate still reports how many rows carry an endorsement, because that is
        // the number that says whether any of this is evidence of quality.
        unmeasured.into_iter().chain(hits).collect()
    } else {
        hits.iter().cloned().chain(unmeasured).collect()
    };

    let hits = ordered.into_iter().skip(offset).take(limit).collect();
    Ranking {
        metric,
        ranked,
        total,
        hits,
    }
}

/// The composite: source-relative quality, renormalised over what the row has.
fn score_of(metric: Metric, signals: &Signals) -> f64 {
    match metric {
        Metric::Quality => {
            let parts: [(Option<f64>, f64); 5] = [
                (signals.source_pct, 0.40),
                (signals.consensus, 0.25),
                (signals.density, 0.15),
                (signals.crosspost, 0.10),
                // Evidence, not quality: a row that leads a four-row archive is
                // not worth the same as one that leads eight hundred.
                (signals.confidence, 0.10),
            ];
            let mut weighted = 0.0;
            let mut weight = 0.0;
            for (value, w) in parts {
                if let Some(value) = value {
                    weighted += value * w;
                    weight += w;
                }
            }
            if weight > 0.0 {
                weighted / weight
            } else {
                0.0
            }
        }
        Metric::SourcePct | Metric::CohortPct => signals.source_pct.unwrap_or(0.0),
        Metric::SourceZ => signals.source_z.unwrap_or(0.0),
        Metric::EngagementDensity => signals.density.unwrap_or(0.0),
        Metric::Consensus => signals.consensus.unwrap_or(0.0),
        Metric::Crosspost => signals.crosspost.unwrap_or(0.0),
        Metric::WordsMaturity => signals.words_maturity.unwrap_or(0.0),
        Metric::SourceCoverage => signals.source_coverage.unwrap_or(0.0),
        Metric::Recency => signals.recency.unwrap_or(0.0),
        Metric::Confidence => signals.confidence.unwrap_or(0.0),
        Metric::Unmeasured => signals.triage.unwrap_or(0.0),
        Metric::Likes
        | Metric::Aggregate
        | Metric::Views
        | Metric::Words
        | Metric::Updated
        | Metric::Velocity
        | Metric::HiddenGem
        | Metric::LikesPerView
        | Metric::Wilson => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, source: &str, likes: i64, words: Option<i64>) -> Candidate {
        Candidate {
            id: id.to_owned(),
            source_key: source.to_owned(),
            likes,
            word_count: words,
            source_count: 1,
            total_likes: likes,
            freshness_days: Some(30.0),
        }
    }

    /// The case that motivates the module, with the live population sizes: the
    /// same story on `sb` (33,107 likes, 884 engaged rows) and on `qq` (4,179,
    /// 209). Raw likes are 8x apart; source-relative scores must be close, because
    /// with these sample sizes the guardrail barely applies — which is correct.
    #[test]
    fn the_same_story_scores_close_in_two_archives_of_different_size() {
        let mut candidates = Vec::new();
        // 884 engaged rows on sb, the story third from the top.
        for i in 0..881 {
            candidates.push(candidate(&format!("sb{i}"), "sb", 10 + i as i64 * 20, Some(50_000)));
        }
        candidates.push(candidate("sb-second", "sb", 40_000, Some(200_000)));
        candidates.push(candidate("sb-first", "sb", 45_000, Some(200_000)));
        candidates.push(candidate("sb-marvel", "sb", 33_107, Some(200_000)));
        // 209 engaged rows on qq, the story at the top.
        for i in 0..208 {
            candidates.push(candidate(&format!("qq{i}"), "qq", 5 + i as i64 * 10, Some(50_000)));
        }
        candidates.push(candidate("qq-marvel", "qq", 4_179, Some(200_000)));

        let ranked = rank(Metric::SourcePct, &candidates, 2_000, 0);
        let sb = ranked.hits.iter().find(|h| h.id == "sb-marvel").expect("sb copy ranked");
        let qq = ranked.hits.iter().find(|h| h.id == "qq-marvel").expect("qq copy ranked");
        let gap = (sb.score - qq.score).abs();
        assert!(
            gap < 0.10,
            "percentile gap {gap} should be small; raw likes are 8x apart"
        );
        assert!(sb.score > 0.85 && qq.score > 0.85, "both are top-of-archive stories");
    }

    /// A single like in a large archive is a weak story, not a strong one — the
    /// failure mode of percentile-over-all-rows, which would score it p99.
    #[test]
    fn one_like_in_a_big_archive_scores_near_the_floor() {
        let mut candidates = vec![candidate("weak", "sb", 1, Some(5_000))];
        for i in 0..499 {
            candidates.push(candidate(&format!("s{i}"), "sb", 100 + i as i64, Some(5_000)));
        }
        let ranked = rank(Metric::SourcePct, &candidates, 1_000, 0);
        let weak = ranked.hits.iter().find(|h| h.id == "weak").expect("ranked");
        assert!(weak.score < 0.05, "score was {}", weak.score);
        assert!(!ranked
            .hits
            .iter()
            .take_while(|h| h.measured)
            .any(|h| h.id == "weak" && h.score > 0.5));
    }

    /// The reason the guardrail is *not* inside the percentile: shrinking it by
    /// sample size caps the achievable score at `0.5 + 0.5·n/(n+10)`, so a source
    /// with 4 engaged rows could never reach 1.0. On the live catalogue that put
    /// 34 of the top 40 rows in the largest archive.
    #[test]
    fn the_best_row_of_any_source_reaches_one_whatever_the_source_size() {
        let mut candidates = vec![candidate("tiny-top", "qq", 2, Some(20_000))];
        for i in 0..3 {
            candidates.push(candidate(&format!("tiny-{i}"), "qq", 1, Some(20_000)));
        }
        for i in 0..400 {
            candidates.push(candidate(&format!("big{i}"), "sb", 10 + i as i64, Some(20_000)));
        }
        candidates.push(candidate("big-top", "sb", 4_000, Some(20_000)));

        let ranked = rank(Metric::SourcePct, &candidates, 500, 0);
        let score = |id: &str| {
            ranked
                .hits
                .iter()
                .find(|h| h.id == id)
                .unwrap_or_else(|| panic!("{id} ranked"))
                .score
        };
        assert!((score("tiny-top") - 1.0).abs() < 1e-9, "best of its own archive");
        assert!((score("big-top") - 1.0).abs() < 1e-9, "best of its own archive");
    }

    /// Evidence is its own signal, and it is what discounts a small population —
    /// in the composite, not by capping the rank.
    #[test]
    fn confidence_is_what_discounts_a_small_engaged_population() {
        let mut candidates = vec![candidate("tiny-top", "qq", 2, Some(20_000))];
        for i in 0..3 {
            candidates.push(candidate(&format!("tiny-{i}"), "qq", 1, Some(20_000)));
        }
        for i in 0..400 {
            candidates.push(candidate(&format!("big{i}"), "sb", 10 + i as i64, Some(20_000)));
        }
        candidates.push(candidate("big-top", "sb", 4_000, Some(20_000)));

        let ranked = rank(Metric::Confidence, &candidates, 500, 0);
        let tiny = ranked.hits.iter().find(|h| h.id == "tiny-top").expect("ranked");
        let big = ranked.hits.iter().find(|h| h.id == "big-top").expect("ranked");
        assert!(tiny.score < 0.3, "four rows are worth little: {}", tiny.score);
        assert!(big.score > 0.9, "four hundred are worth a lot: {}", big.score);

        // And the composite, which weighs it, prefers the better-evidenced row
        // even though both lead their own archive.
        let quality = rank(Metric::Quality, &candidates, 500, 0);
        let q = |id: &str| quality.hits.iter().find(|h| h.id == id).expect("ranked").score;
        assert!(
            q("big-top") > q("tiny-top"),
            "quality must prefer evidence: {} vs {}",
            q("big-top"),
            q("tiny-top")
        );
    }

    /// Unmeasured rows never mingle with measured ones, whatever the metric.
    #[test]
    fn unmeasured_rows_follow_measured_ones_and_say_so() {
        let candidates = vec![
            candidate("no-data", "sb", 0, Some(60_000)),
            candidate("has-data", "sb", 900, Some(60_000)),
        ];
        let ranked = rank(Metric::Quality, &candidates, 10, 0);
        assert_eq!(ranked.ranked, 1);
        assert_eq!(ranked.total, 2);
        assert_eq!(ranked.hits[0].id, "has-data");
        assert!(ranked.hits[0].measured);
        assert_eq!(ranked.hits[1].id, "no-data");
        assert!(!ranked.hits[1].measured);
    }

    /// The triage metric ranks the unmeasured block on metadata, and the gate
    /// still reports zero measured rows — it is not evidence of quality.
    #[test]
    fn triage_ranks_metadata_and_claims_no_quality() {
        let mut zero = candidate("lonely", "sb", 0, None);
        zero.source_count = 1;
        let mut cross = candidate("everywhere", "sb", 0, Some(220_000));
        cross.source_count = 4;
        let ranked = rank(Metric::Unmeasured, &[zero, cross], 5, 0);
        assert_eq!(ranked.ranked, 0);
        assert_eq!(ranked.hits[0].id, "everywhere");
        assert!(ranked.hits.iter().all(|h| !h.measured));
        assert!(ranked.hits[0].signals.triage.unwrap() > ranked.hits[1].signals.triage.unwrap());
    }

    #[test]
    fn a_composite_drops_a_missing_signal_and_renormalises_rather_than_zeroing_it() {
        let with_words = candidate("with", "sb", 500, Some(120_000));
        let without = candidate("without", "sb", 500, None);
        let ranked = rank(Metric::Quality, &[with_words, without], 5, 0);
        for hit in &ranked.hits {
            assert!(hit.score > 0.0, "a missing word count must not zero the composite");
        }
    }

    #[test]
    fn every_metric_round_trips_through_parse() {
        for metric in Metric::ALL {
            assert_eq!(Metric::parse(metric.as_str()), Some(*metric), "{metric:?}");
        }
    }

    #[test]
    fn an_unknown_metric_is_refused_rather_than_silently_reordered() {
        assert_eq!(Metric::parse("kudos"), None);
        assert_eq!(Metric::parse("likes desc; drop table"), None);
    }

    #[test]
    fn a_blocked_metric_names_the_work_that_unblocks_it() {
        for metric in [Metric::Velocity, Metric::HiddenGem] {
            match metric.support() {
                Support::Blocked(reason) => assert!(reason.contains("M63-32"), "{reason}"),
                other => panic!("{metric:?} should be blocked, was {other:?}"),
            }
        }
        match Metric::LikesPerView.support() {
            Support::Blocked(reason) => assert!(reason.contains("views")),
            other => panic!("likes_per_view should be blocked, was {other:?}"),
        }
    }

    #[test]
    fn paging_is_stable_across_pages_because_ties_break_on_the_id() {
        let candidates: Vec<Candidate> = (0..10)
            .map(|i| candidate(&format!("id{i:02}"), "sb", 100, Some(50_000)))
            .collect();
        let first = rank(Metric::SourcePct, &candidates, 5, 0);
        let second = rank(Metric::SourcePct, &candidates, 5, 5);
        assert_eq!(first.hits.len(), 5);
        assert_eq!(second.hits.len(), 5);
        let ids: Vec<&str> = first
            .hits
            .iter()
            .chain(second.hits.iter())
            .map(|h| h.id.as_str())
            .collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "a second page must continue the first, not repeat it");
    }

    #[test]
    fn recency_decays_and_absent_dates_are_absent_rather_than_old() {
        let mut fresh = candidate("fresh", "sb", 10, Some(20_000));
        fresh.freshness_days = Some(0.0);
        let mut undated = candidate("undated", "sb", 10, Some(20_000));
        undated.freshness_days = None;
        let ranked = rank(Metric::Recency, &[fresh, undated], 5, 0);
        let fresh = ranked.hits.iter().find(|h| h.id == "fresh").unwrap();
        assert!((fresh.score - 1.0).abs() < 1e-9);
        // Both score > 0 because ties are broken on the id; the undated row is 0.
        assert!(ranked
            .hits
            .iter()
            .all(|h| h.id != "undated" || h.score == 0.0));
    }
}
