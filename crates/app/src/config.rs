//! Configuration loading.
//!
//! Documented precedence (spec §5):
//!
//! ```text
//! command-line argument  →  environment variable  →  configuration file  →  default
//! ```
//!
//! Every value is resolved the same way, in one place, so `doctor` and `serve`
//! can never disagree about what the running configuration is.
//!
//! The file is parsed with `deny_unknown_fields`: a typo in a self-hosted
//! operator's `lorehaven.toml` is a startup error with a precise message,
//! not a silently ignored line.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use lorehaven_db::DatabaseConfig;
use lorehaven_domain::media_resilience::{AudioFingerprint, PerceptualHashAlgorithm};
use lorehaven_domain::AccountId;
use serde::{Deserialize, Serialize};

use crate::cli::GlobalArgs;

/// The default configuration file name, looked up in the working directory.
pub const DEFAULT_CONFIG_FILE: &str = "lorehaven.toml";

/// Runtime environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    /// Local development: permissive, verbose.
    Development,
    /// Automated tests: isolated, quiet.
    Test,
    /// A real deployment: strict.
    Production,
}

impl Environment {
    /// Parse the environment name.
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "development" | "dev" => Ok(Self::Development),
            "test" => Ok(Self::Test),
            "production" | "prod" => Ok(Self::Production),
            other => anyhow::bail!(
                "unknown environment {other:?}; expected development, test or production"
            ),
        }
    }

    /// Stable lowercase name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Test => "test",
            Self::Production => "production",
        }
    }

    /// Whether this is a production deployment.
    #[must_use]
    pub const fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }

    /// Whether this is a local development instance.
    ///
    /// Development-only affordances are gated on this rather than on a feature
    /// flag, so a route that exists to make the interface drivable before its
    /// real caller arrives cannot be reached in a deployment.
    #[must_use]
    pub const fn is_development(self) -> bool {
        matches!(self, Self::Development)
    }
}

/// Log output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Human-readable, for a terminal.
    Pretty,
    /// One JSON object per line, for a log collector.
    Json,
}

impl LogFormat {
    /// Parse the format name.
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "pretty" | "text" => Ok(Self::Pretty),
            "json" => Ok(Self::Json),
            other => anyhow::bail!("unknown log format {other:?}; expected pretty or json"),
        }
    }
}

/// The resolved configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Active environment.
    pub environment: Environment,
    /// Public identity of the instance.
    pub site: SiteConfig,
    /// HTTP listener settings.
    pub server: ServerConfig,
    /// Database settings.
    pub database: DatabaseConfig,
    /// File storage settings.
    pub storage: StorageConfig,
    /// Cookie and session settings.
    pub security: SecurityConfig,
    /// Logging settings.
    pub logging: LoggingConfig,
    /// Static asset settings.
    pub assets: AssetsConfig,
    /// Development-only affordances.
    pub dev: DevConfig,
    /// Account and registration settings.
    pub accounts: AccountsConfig,
    /// Age-policy settings.
    pub age: AgeConfig,
    /// Operator-only settings.
    pub administration: AdministrationConfig,
    /// Rate limits.
    pub rate_limits: crate::limiter::Limits,
    /// What the importer may do about a source that refuses a plain request.
    pub imports: ImportsConfig,
    pub retention: RetentionConfig,
    /// Calibrated decision models (spec §11.14, amendment
    /// `calibrated-decision-models.md`).
    pub decisions: DecisionsConfig,
    /// Theme mode and gravity settings (spec §0.4.6).
    pub theme: ThemeConfig,
    /// Discovery feed diversity settings.
    pub discovery: DiscoveryConfig,
    /// TTS narration settings.
    pub tts: TtsConfig,
    /// Bulk export settings.
    pub bulk_export: BulkExportConfig,
    /// Forum settings (spec 35).
    pub forum: ForumConfig,
    /// Resource directory settings (spec §39).
    pub directory: DirectoryConfig,
    /// Export retention settings (spec §38).
    pub exports: ExportsConfig,
    /// Work permission statements and fork guards (spec §40).
    pub works: WorksConfig,
    /// Longevity signals: half-life and interaction warmth (spec §41).
    pub community: CommunityConfig,
    /// Taste gravity settings (spec §0.4, §16.17).
    pub taste: TasteConfig,
    /// Flexible bounty settings (spec §20.3.2).
    pub bounties: BountiesConfig,
    /// Vanguard role settings (spec §16.18).
    pub vanguard: VanguardConfig,
    /// Signal weighting settings (spec §9.7.3).
    pub signals: SignalsConfig,
    /// Instance preset (spec §0.6).
    pub instance: InstanceConfig,
    /// Meta-ranker settings (spec §9.10).
    pub meta_ranker: MetaRankerConfig,
    /// Revision caching settings (spec §38).
    pub revisions: RevisionsConfig,
    /// Job queue settings (spec §38).
    pub jobs: JobsConfig,
    /// Device delivery settings (spec §13.4 / M7-03).
    pub device: Option<DeviceConfig>,
    /// Media resilience settings (spec §32.7).
    pub media_resilience: MediaResilienceConfig,
    /// Library update-check settings (spec §38).
    pub library: LibraryConfig,
    /// Where the configuration file was read from, if any.
    pub config_path: Option<PathBuf>,
    /// Roadmap participation settings (spec §29.4).
    pub roadmap: RoadmapConfig,
    /// Retention governance settings (spec §5, §19.15, amendment §5).
    pub retention_governance: RetentionGovernanceConfig,
}

/// Roadmap participation settings (spec §29.4).
#[derive(Debug, Clone)]
pub struct RoadmapConfig {
    /// The trust level a reader needs to take part in the roadmap — vote on an
    /// arena ballot, vote on a card, or suggest one.
    ///
    /// **This exists because the three call sites hardcoded `1`.** §29.4 says
    /// roadmap participation is gated on a trust level, and the Phase E
    /// amendment cites that sentence while adding a retention proposal route
    /// gated the same way — which left §29.4's sentence decorative: an operator
    /// who wanted a higher bar had no way to ask for one without a code change
    /// and a release. The default is 1, so no existing instance changes
    /// behaviour and the three sites that were already there are unchanged.
    pub min_trust: i64,
}

impl Default for RoadmapConfig {
    fn default() -> Self {
        Self { min_trust: 1 }
    }
}

/// Retention governance settings (spec §5, §19.15, amendment §5).
#[derive(Debug, Clone)]
pub struct RetentionGovernanceConfig {
    /// The trust level needed to open a proposal or cast a ballot.
    ///
    /// Not 0 by default. A retention proposal changes what this instance stores
    /// for every reader, and §5's point is that the decision is a *governance*
    /// one — so the people making it are the ones the instance already knows
    /// something about. 1 rather than 2, because 2 would exclude a
    /// newly-registered reader from a vote on a decision they will live with.
    pub proposal_min_trust: i64,
    /// The instance's bar for a change that *widens* storage.
    ///
    /// Passed straight to `quorum_for`, which clamps it up to `MINIMUM_QUORUM`
    /// (3) — a bar below three is a proposal decided by one person's second
    /// tap. A bar *above* the number of readers is honoured and means "this
    /// instance does not change storage policy by vote", which is a legitimate
    /// thing for an operator to say.
    pub widen_quorum: i64,
    /// How long a proposal's ballot stays open, in days.
    ///
    /// Recorded as a stored `closes_at` per proposal rather than a day count
    /// applied at read time, so an operator changing this does not silently
    /// move the deadline of every open ballot.
    pub proposal_cooling_days: i64,
    /// Whether a passed proposal changes the setting on its own.
    ///
    /// `false` is the default and is the safe direction: in advisory mode the
    /// readers' decision is recorded and an operator applies it, so a quorum of
    /// three is an argument rather than an instruction. In binding mode a
    /// proposal commits after `proposal_cooling_days`, which gives an operator
    /// time to object — the asymmetry is §5.3's, and it is a setting because
    /// both positions are defensible.
    pub binding_mode: bool,
}

impl Default for RetentionGovernanceConfig {
    fn default() -> Self {
        Self {
            proposal_min_trust: 1,
            widen_quorum: 3,
            proposal_cooling_days: 7,
            binding_mode: false,
        }
    }
}

/// Work permission settings (spec §40).
#[derive(Debug, Clone)]
pub struct MediaResilienceConfig {
    /// Minimum healthy links a media reference should have before alerting.
    pub min_healthy_links: i64,
    /// Curator credits awarded for adding a mirror link.
    pub mirror_add_credits: i64,
    /// Curator credits awarded for archiving a link.
    pub archive_add_credits: i64,
    /// Curator credits awarded for verifying a link.
    pub verify_credits: i64,
    /// Max curator credits per account per day.
    pub daily_credits_cap: i64,
    /// Number of consecutive failures before marking a link as dead.
    pub dead_threshold_failures: i64,
    /// How often to check links (in seconds).
    pub check_interval_secs: u64,
    /// Whether media resilience runs on this instance at all (spec §32.7.2).
    pub enabled: bool,
    /// The perceptual hash algorithm used for images (spec §32.7.2).
    pub perceptual_hash_algorithm: PerceptualHashAlgorithm,
    /// Hamming distance at or below which two image hashes are offered as the
    /// same image (spec §32.7.2, default 6).
    pub perceptual_match_threshold: i64,
    /// A perceptual match scoring below this confidence needs a curator to
    /// confirm the linkage. Higher means more human review.
    pub require_curator_confirmation_below: i64,
    /// A perceptual match scoring at or above this confidence auto-attaches.
    /// `0` — the spec's default — means no perceptual match auto-attaches; only
    /// exact content-hash matches do.
    pub require_curator_confirmation_above: i64,
    /// The audio fingerprinting scheme (spec §32.7.2).
    pub audio_fingerprint: AudioFingerprint,
}

impl Default for MediaResilienceConfig {
    fn default() -> Self {
        Self {
            min_healthy_links: 2,
            mirror_add_credits: 15,
            archive_add_credits: 10,
            verify_credits: 5,
            daily_credits_cap: 500,
            dead_threshold_failures: 5,
            check_interval_secs: 3600,
            enabled: true,
            perceptual_hash_algorithm: PerceptualHashAlgorithm::Dhash,
            perceptual_match_threshold: 6,
            require_curator_confirmation_below: 3,
            require_curator_confirmation_above: 0,
            audio_fingerprint: AudioFingerprint::Chromaprint,
        }
    }
}

/// Work permission settings (spec §40).
#[derive(Debug, Clone)]
pub struct WorksConfig {
    /// Maximum fork chain depth. A work whose lineage chain is already this
    /// deep refuses to fork. Default 3.
    pub max_fork_depth: u32,
}

impl Default for WorksConfig {
    fn default() -> Self {
        Self { max_fork_depth: 3 }
    }
}

/// Operator-only settings.
///
/// **There is no staff model yet.** Milestone 5 needs somebody who may look at
/// the queue, and the trust model arrives in Milestone 13, so this names *one
/// account* — never a boolean, never a role, and never a column on `accounts`.
/// M13 replaces it with a trust level (see
/// `docs/plans/junior-implementation-plan.md` §M13), and a `is_admin` flag would
/// have survived until then and been wrong in every query that read it.
#[derive(Debug, Clone, Default)]
pub struct AdministrationConfig {
    /// The account allowed to reach `/admin` routes, if any.
    pub operator_account_id: Option<AccountId>,
    /// Webhook delivery settings.
    pub webhook_timeout_secs: u64,
    pub webhook_max_attempts: u32,
    pub webhook_base_delay_ms: u64,
    pub webhook_allowed_hosts: Vec<String>,
}

/// Account creation settings.
#[derive(Debug, Clone)]
pub struct AccountsConfig {
    /// Whether new accounts may register at all.
    ///
    /// Exposed because a small instance frequently closes registration after an
    /// initial cohort; making that a configuration value rather than a code
    /// change keeps it an operational decision.
    pub registration_open: bool,
}

/// Who defines instance taste, in priority order (spec §0.4.6, §16.15).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct InfluenceSourceConfig {
    /// The kind of influence source.
    pub kind: InfluenceSourceKind,
    /// Minimum number of distinct accounts required for cohort/role/long_term sources.
    #[serde(default = "default_min_members")]
    pub min_members: usize,
    /// For `long_term_users`: minimum account age in days.
    #[serde(default = "default_tenure_days")]
    pub min_tenure_days: u64,
    /// For `long_term_users`: minimum contribution events.
    #[serde(default = "default_min_contributions")]
    pub min_contributions: u64,
    /// For `roles`: which roles qualify.
    #[serde(default)]
    pub roles: Vec<String>,
    /// For `cohort`: explicit member pseud ids.
    #[serde(default)]
    pub members: Vec<String>,
}

fn default_min_members() -> usize {
    5
}
fn default_tenure_days() -> u64 {
    90
}
fn default_min_contributions() -> u64 {
    5
}

/// The kinds of influence sources (spec §16.15).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InfluenceSourceKind {
    /// The operator's declared public topics.
    OperatorTopics,
    /// The §16.2 administrator taste profile.
    AdminTaste,
    /// Aggregate of long-term contributors (§16.15 `long_term_users`).
    LongTermUsers,
}

/// Theme mode and gravity settings (spec §0.4.6).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ThemeConfig {
    /// The instance's discovery posture: `generic`, `thematic`, or `adaptive`.
    #[serde(default = "default_theme_mode")]
    pub mode: String,
    /// Whether the §16.5 dial can reach zero for theme influence.
    /// When false, `theme_dial_floor_bp` is the dial's lower bound.
    #[serde(default = "true_bool")]
    pub allow_user_opt_out: bool,
    /// Lower bound for the §16.5 dial when opt-out is locked (in basis points).
    #[serde(default = "default_theme_dial_floor_bp")]
    pub theme_dial_floor_bp: i64,
    /// Maximum adaptive drift in basis points (0 = no drift).
    #[serde(default)]
    pub adaptive_max_drift_bp: i64,
    /// Ordered influence sources that shape instance taste.
    #[serde(default = "default_influence_sources")]
    pub influence_sources: Vec<InfluenceSourceConfig>,
    /// Tag names that increase topic gravity when a work matches (case-insensitive).
    #[serde(default)]
    pub boost_tags: Vec<String>,
    /// Tag names that decrease topic gravity when a work matches (case-insensitive).
    #[serde(default)]
    pub suppress_tags: Vec<String>,
    /// Per-tag gravity overrides in basis points (tag -> bp).
    #[serde(default)]
    pub tag_gravity_bp: std::collections::HashMap<String, i64>,
}

fn default_theme_mode() -> String {
    "thematic".to_string()
}
fn default_theme_dial_floor_bp() -> i64 {
    1000
}
fn true_bool() -> bool {
    true
}
fn default_influence_sources() -> Vec<InfluenceSourceConfig> {
    vec![InfluenceSourceConfig {
        kind: InfluenceSourceKind::OperatorTopics,
        min_members: 5,
        min_tenure_days: 90,
        min_contributions: 5,
        roles: Vec::new(),
        members: Vec::new(),
    }]
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            mode: default_theme_mode(),
            allow_user_opt_out: true,
            theme_dial_floor_bp: default_theme_dial_floor_bp(),
            adaptive_max_drift_bp: 0,
            influence_sources: default_influence_sources(),
            boost_tags: Vec::new(),
            suppress_tags: Vec::new(),
            tag_gravity_bp: std::collections::HashMap::new(),
        }
    }
}

/// Discovery feed diversity settings.
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// Maximum works from the same fandom in one feed response (0 = unlimited).
    pub per_fandom_cap: usize,
    /// Fraction of the feed reserved for exploration (new fandoms).
    pub exploration_rate: f64,
    /// Whether the half-life job runs and scores influence discovery ranking.
    pub enable_half_life: bool,
    /// Minimum age in days before a work is eligible for half-life scoring.
    pub half_life_min_age_days: i64,
    /// Window size in days for both recent and first-window reader counts.
    pub half_life_window_days: i64,
    /// Which recommender to serve: `legacy`, `pluggable` or `shadow`
    /// (spec §16.1a). See [`RecMode`].
    pub rec_mode: RecMode,
    /// RRF k constant for the strategy blend (spec §16.1a). Default: 60.
    pub rec_rrf_k: f64,
    /// Per-strategy resource ceiling: max results each strategy may contribute.
    pub rec_per_strategy_cap: usize,
    /// Enabled strategy names for the pluggable mode. Empty = all enabled.
    ///
    /// This is the *instance* choice. A reader's own override is the
    /// `discovery.rec_engine` per-account setting, which falls back to this
    /// when unset; see `rec_preference::resolve_strategies`.
    pub rec_enabled_strategies: Vec<String>,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            per_fandom_cap: 0,
            exploration_rate: 0.0,
            enable_half_life: true,
            half_life_min_age_days: 14,
            half_life_window_days: 30,
            rec_mode: RecMode::Legacy,
            rec_rrf_k: 60.0,
            rec_per_strategy_cap: 100,
            rec_enabled_strategies: Vec::new(),
        }
    }
}

/// Community / interaction tier settings (spec §41.2).
#[derive(Debug, Clone)]
pub struct CommunityConfig {
    /// Warmth thresholds in basis points. Default: lurk 0, react 200,
    /// comment 1000, create 3000.
    pub warmth_thresholds: serde_json::Value,
}

impl Default for CommunityConfig {
    fn default() -> Self {
        Self {
            warmth_thresholds: serde_json::json!({
                "lurk": 0,
                "react": 200,
                "comment": 1000,
                "create": 3000
            }),
        }
    }
}

/// Flexible bounty settings (spec §20.3.2, M18 Phase 4.1).
#[derive(Debug, Clone)]
pub struct BountiesConfig {
    /// Allowed bounty types. Default: standard, crowdfunded, reverse.
    pub allowed_types: Vec<String>,
    /// Minimum bounty amount in credits.
    pub min_amount: i64,
    /// Maximum bounty amount in credits.
    pub max_amount: i64,
    /// Crowdfunded bounty auto-activate threshold (fraction 0.0..1.0).
    pub crowdfund_activation_threshold: f64,
}

impl Default for BountiesConfig {
    fn default() -> Self {
        Self {
            allowed_types: vec![
                "standard".to_string(),
                "crowdfunded".to_string(),
                "reverse".to_string(),
            ],
            min_amount: 10,
            max_amount: 10_000,
            crowdfund_activation_threshold: 1.0,
        }
    }
}

/// Vanguard role settings (spec §16.18, M18 Phase 4.2).
#[derive(Debug, Clone)]
pub struct VanguardConfig {
    /// Selection method: resonance_threshold, admin_appointment, contribution_volume.
    pub method: String,
    /// Threshold percentage for resonance_threshold method.
    pub threshold_percent: f64,
    /// Max number of vanguards (for contribution_volume method).
    pub limit: i64,
    /// Whether vanguards can pin works.
    pub can_pin: bool,
    /// Whether vanguards can nominate works for admin review.
    pub can_nominate: bool,
    /// Whether vanguards can create reading clubs.
    pub can_create_clubs: bool,
    /// Whether the Vanguard badge is publicly visible.
    pub public_badge: bool,
    /// Bounty discount fraction (0.5 = 50% off).
    pub bounty_discount: f64,
    /// Default pin duration in days.
    pub pin_duration_days: i64,
}

impl Default for VanguardConfig {
    fn default() -> Self {
        Self {
            method: "contribution_volume".to_string(),
            threshold_percent: 10.0,
            limit: 25,
            can_pin: true,
            can_nominate: true,
            can_create_clubs: true,
            public_badge: true,
            bounty_discount: 0.5,
            pin_duration_days: 30,
        }
    }
}

/// Taste gravity settings (spec §0.4, §16.17).
#[derive(Debug, Clone)]
pub struct TasteConfig {
    /// Taste gravity strength (0.0 = off, 1.0 = full influence).
    pub gravity_strength: f64,
    /// Signal weighting mode.
    pub signal_weight_mode: String,
    /// Admin engagement weight multiplier.
    pub admin_weight: f64,
    /// Diversity injection percentage (0.0..1.0, 0.0 = monoculture).
    pub diversity_injection_percent: f64,
    /// Taste dimensions (names only; vectors are computed at runtime).
    pub dimensions: Vec<String>,
}

impl Default for TasteConfig {
    fn default() -> Self {
        Self {
            gravity_strength: 0.0,
            signal_weight_mode: "taste_weighted".to_string(),
            admin_weight: 1.0,
            diversity_injection_percent: 0.1,
            dimensions: vec![
                "angst".to_string(),
                "pacing".to_string(),
                "prose_density".to_string(),
                "canon_compliance".to_string(),
                "trope_diversity".to_string(),
            ],
        }
    }
}

/// Signal weighting settings (spec §9.7.3).
#[derive(Debug, Clone)]
pub struct SignalsConfig {
    /// Signal weighting mode: egalitarian, taste_weighted, admin_only.
    pub mode: String,
    /// Diversity injection percentage (0.0..1.0).
    pub diversity_injection_percent: f64,
}

impl Default for SignalsConfig {
    fn default() -> Self {
        Self {
            mode: "taste_weighted".to_string(),
            diversity_injection_percent: 0.1,
        }
    }
}

/// Instance preset (spec §0.6) and accessibility posture (spec §0.4.7).
#[derive(Debug, Clone)]
pub struct InstanceConfig {
    /// Preset name: open_library, curated_boutique, admin_garden, genre_haven, experimental_lab, custom.
    pub preset: String,
    /// Who may reach the instance without signing in.
    pub mode: InstanceMode,
}

/// How widely an instance admits readers who have not identified themselves
/// (spec §0.4.7).
///
/// The name is deliberately about the *instance*, not the work. A work's own
/// `visibility` column is the second axis and is unchanged by this setting; the
/// effective access to a work is the two combined by the one eligibility service
/// in `lorehaven_domain::policy`.
///
/// `Public` is the default because spec §7 requires anonymous reading of
/// suitable public fiction to stay available. `WalledGarden` keeps that reading
/// but puts a sign-in wall in front of it. `Private` is the operator's
/// maintenance posture: the instance answers nobody but an administrator, and
/// says so rather than pretending to be an empty public library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InstanceMode {
    /// Anyone may browse. Writes still require a session.
    #[default]
    Public,
    /// Browsing requires a session. The landing page and `/api/v1/meta` stay
    /// open, because a reader who cannot even see that sign-in exists cannot
    /// sign in.
    WalledGarden,
    /// The instance is closed to everyone but the operator.
    Private,
}

/// Which recommender the instance serves (spec §16.1a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecMode {
    /// `discovery::blend` over the multi-engine candidates. Default.
    #[default]
    Legacy,
    /// The strategy registry with an RRF blend.
    Pluggable,
    /// Serves `legacy` *and* runs the pluggable registry alongside, recording
    /// how the two rankings would differ.
    ///
    /// What a reader receives is unchanged — that is the entire safety
    /// property, and the reason this is a mode rather than a flag on
    /// `Pluggable`: a flag would let it decide which ranking was served, which
    /// is the one thing it must not. The spec requires evaluation to precede a
    /// switch, and this is how that happens without switching.
    Shadow,
}

impl RecMode {
    /// Canonical configuration spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Pluggable => "pluggable",
            Self::Shadow => "shadow",
        }
    }

    /// Parse a configuration value, refusing anything unrecognised.
    ///
    /// The route dispatches on this value, so a typo that fell back to
    /// `legacy` would be indistinguishable from a working instance: the
    /// operator would have no way to tell a misspelling from a bug.
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "legacy" => Ok(Self::Legacy),
            "pluggable" => Ok(Self::Pluggable),
            "shadow" => Ok(Self::Shadow),
            other => anyhow::bail!(
                "discovery.rec_mode must be one of legacy, pluggable, shadow, got {other:?}"
            ),
        }
    }
}

impl std::fmt::Display for RecMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl InstanceMode {
    /// Canonical configuration spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::WalledGarden => "walled_garden",
            Self::Private => "private",
        }
    }

    /// Parse a configuration value, refusing anything unrecognised.
    ///
    /// An operator typo is a startup error rather than a silently ignored line:
    /// a misspelled mode that fell back to `public` would open an instance the
    /// operator believed closed.
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "public" | "open" => Ok(Self::Public),
            "walled_garden" | "walled-garden" | "walled" => Ok(Self::WalledGarden),
            "private" | "closed" => Ok(Self::Private),
            other => anyhow::bail!(
                "unknown instance mode {other:?}; expected public, walled_garden or private"
            ),
        }
    }

    /// Whether a request with no session may reach browsing surfaces at all.
    ///
    /// `Private` refuses even a signed-in reader; the operator-only rule is
    /// enforced by the trust level, not here.
    #[must_use]
    pub const fn allows_anonymous_browsing(self) -> bool {
        matches!(self, Self::Public)
    }

    /// The eligibility policy this mode implies.
    #[must_use]
    pub fn access_policy(self) -> lorehaven_domain::policy::AccessPolicy {
        lorehaven_domain::policy::AccessPolicy {
            anonymous_reading_enabled: self.allows_anonymous_browsing(),
            ..lorehaven_domain::policy::AccessPolicy::default()
        }
    }
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            preset: "curated_boutique".to_string(),
            mode: InstanceMode::default(),
        }
    }
}

/// Meta-ranker (Thompson Sampling over recommendation strategies) settings (spec §9.10).
#[derive(Debug, Clone)]
pub struct MetaRankerConfig {
    pub enabled: bool,
    pub exploration_percent: u8,
    pub exploitation_percent: u8,
    pub min_impressions_per_strategy: u64,
    pub rebalance_frequency_hours: u64,
    pub max_active_strategies: usize,
    pub success_metric: String,
    pub auto_disable_threshold: f64,
    pub candidate_exploration_bonus: f64,
    pub candidate_promotion_impressions: u64,
}

impl Default for MetaRankerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            exploration_percent: 15,
            exploitation_percent: 85,
            min_impressions_per_strategy: 50,
            rebalance_frequency_hours: 24,
            max_active_strategies: 20,
            success_metric: "admin_aligned".to_string(),
            auto_disable_threshold: 0.3,
            candidate_exploration_bonus: 2.0,
            candidate_promotion_impressions: 500,
        }
    }
}

/// Revision caching settings (spec §38).
#[derive(Debug, Clone)]
pub struct RevisionsConfig {
    /// How long a cached source revision lives, in seconds.
    pub ttl_secs: i64,
}

impl Default for RevisionsConfig {
    fn default() -> Self {
        Self {
            ttl_secs: 7 * 24 * 60 * 60,
        }
    }
}

/// Job queue settings (spec §38).
#[derive(Debug, Clone)]
pub struct JobsConfig {
    /// How long terminal jobs are kept after completion/failure, in days.
    pub terminal_retention_days: i64,
    /// How long a served recommendation slot is kept so a reader can still ask
    /// why they saw something (spec §33.3a).
    ///
    /// A slot is a record of what a reader was shown. Keeping it indefinitely
    /// would be a profile they never asked for, and "why am I seeing this" is a
    /// question about the recent past. Three days is long enough to cover a
    /// reader who notices a recommendation and goes looking for it the next day.
    pub slot_retention_days: i64,
}

impl Default for JobsConfig {
    fn default() -> Self {
        Self {
            terminal_retention_days: 30,
            slot_retention_days: 3,
        }
    }
}

/// Library update-check settings (spec §38).
#[derive(Debug, Clone)]
pub struct LibraryConfig {
    /// How long an update-check record is kept, in days.
    pub update_check_retention_days: i64,
    /// How many items one library update job checks.
    pub check_batch: i64,
}

impl Default for LibraryConfig {
    fn default() -> Self {
        Self {
            update_check_retention_days: 90,
            check_batch: 50,
        }
    }
}

/// Resource directory settings (spec §39, §45).
#[derive(Debug, Clone)]
pub struct DirectoryConfig {
    /// Extra categories beyond the seed set (§39.2). Operators extend the
    /// directory through the config file; nothing is hardcoded here.
    pub extra_categories: Vec<String>,
    /// How votes are weighted (§39.4).
    pub weighting: String,
    /// Trust multipliers per rung of the §19.1 ladder (TL0..TL6).
    pub trust_vote_weights: [f64; 7],
    /// Floor/ceiling the taste affinity maps onto.
    pub taste_floor: f64,
    pub taste_ceiling: f64,
    /// Category governance settings (§45).
    pub governance: CategoryGovernanceConfig,
    /// Vote decay (spec §39, amendment `docs/spec-amendments/vote-decay.md`).
    ///
    /// A vote counts fully when cast and decays toward nothing, so a directory
    /// ranks current consensus rather than who noticed an entry first. Entries
    /// with few votes never decay — see `Decay::should_decay`.
    pub decay_enabled: bool,
    /// Age in days at which a vote is worth exactly nothing. Default 60.
    pub decay_cutoff_days: f64,
    /// Entries with fewer live votes than this never decay. Default 20.
    pub decay_min_votes: i64,
    /// The curve's shape, an integer. 1 is linear, 2 the default.
    ///
    /// An integer because the score query computes this curve in SQL and
    /// sqlx's bundled SQLite has no math functions at all — only integer powers
    /// are expressible in both dialects.
    pub decay_exponent: u32,
}

/// Category governance settings (spec §45).
#[derive(Debug, Clone)]
pub struct CategoryGovernanceConfig {
    /// Freeze all category governance: no proposals, no votes (§45.3).
    pub frozen: bool,
    /// Operator-raiseable ceiling on active categories (§45.4). Default 32.
    pub max_active_categories: u32,
}

impl Default for CategoryGovernanceConfig {
    fn default() -> Self {
        Self {
            frozen: false,
            max_active_categories: lorehaven_domain::category_governance::MAX_ACTIVE_CATEGORIES,
        }
    }
}

impl DirectoryConfig {
    /// The full category set: seed categories plus configured extras.
    pub fn categories(&self) -> Vec<String> {
        let mut out: Vec<String> = lorehaven_domain::directory::SEED_CATEGORIES
            .iter()
            .map(|s| s.to_string())
            .collect();
        for extra in &self.extra_categories {
            let e = extra.trim().to_lowercase();
            if !e.is_empty() && !out.contains(&e) {
                out.push(e);
            }
        }
        out
    }

    /// The parsed weighting mode.
    pub fn weighting_mode(&self) -> lorehaven_domain::directory::VoteWeighting {
        lorehaven_domain::directory::VoteWeighting::parse(&self.weighting)
            .unwrap_or(lorehaven_domain::directory::VoteWeighting::TrustAndTaste)
    }

    /// The vote-decay policy for this instance.
    ///
    /// Built here rather than stored, so a malformed config value is
    /// normalised in exactly one place: `Decay::from_config` falls back to the
    /// documented default for a zero or negative cutoff rather than producing
    /// a directory where every vote is worth nothing.
    pub fn decay(&self) -> lorehaven_domain::vote_decay::Decay {
        lorehaven_domain::vote_decay::Decay::from_config(
            self.decay_enabled,
            self.decay_cutoff_days,
            self.decay_min_votes,
            self.decay_exponent,
        )
    }
}

impl Default for DirectoryConfig {
    fn default() -> Self {
        Self {
            extra_categories: Vec::new(),
            weighting: "trust_and_taste".to_owned(),
            trust_vote_weights: lorehaven_domain::directory::DEFAULT_TRUST_VOTE_WEIGHTS,
            taste_floor: lorehaven_domain::directory::DEFAULT_TASTE_FLOOR,
            taste_ceiling: lorehaven_domain::directory::DEFAULT_TASTE_CEILING,
            governance: CategoryGovernanceConfig::default(),
            // The documented defaults, not zeros: decay is on by default
            // because a permanent vote measures when someone first noticed an
            // entry rather than what anyone believes now.
            decay_enabled: true,
            decay_cutoff_days: 60.0,
            decay_min_votes: 20,
            decay_exponent: 2,
        }
    }
}

/// TTS narration settings (M26 / spec §32.5).
///
/// Piper is the built-in local engine. Cloud adapters (ElevenLabs, AWS
/// Polly) are added later behind the same `TtsEngine` trait.
#[derive(Debug, Clone)]
pub struct BulkExportConfig {
    /// How many works a bulk export may bundle. Default 50.
    pub max_items: i64,
    /// How many bytes a bulk export may total. Default 1073741824 (1 GiB).
    pub max_bytes: i64,
}

/// Forum settings (spec 35).
#[derive(Debug, Clone)]
pub struct ForumConfig {
    /// The discussion mode applied to **new** works. Existing works keep
    /// their own mode; the default is never applied retroactively.
    pub work_discussion_default: lorehaven_domain::work_discussion::WorkDiscussionMode,
    /// `(minimum trust level, votes per rolling 24h)` rungs of the vote
    /// budget, ascending by level (spec §35.2). A level below every rung gets
    /// the first rung's allowance.
    pub vote_budget: Vec<lorehaven_domain::typed_votes::BudgetRung>,
    /// Meta-mod points (vote flags) a TL4+ steward may spend per rolling 24h.
    pub meta_mod_points: i64,
    /// Verdicts a caster needs before their vote weight may decay. Below this
    /// a single flag changes nothing.
    pub meta_mod_min_verdicts: i64,
    /// The floor a decayed vote weight never goes below, in basis points.
    /// Lower weight, never fewer rights (spec §35.2).
    pub min_vote_weight_bp: i64,
    /// Monthly karma decay for an inactive receiver, in percent (§35.2).
    pub karma_decay_percent: i64,
}

impl Default for ForumConfig {
    fn default() -> Self {
        Self {
            work_discussion_default:
                lorehaven_domain::work_discussion::WorkDiscussionMode::CommentsOnly,
            // Spec §35.2: TL1=10, TL3=30, TL5=60.
            vote_budget: vec![(1, 10), (3, 30), (5, 60)],
            meta_mod_points: 20,
            meta_mod_min_verdicts: 3,
            min_vote_weight_bp: 100,
            karma_decay_percent: lorehaven_domain::typed_votes::KARMA_DECAY_PERCENT,
        }
    }
}

impl Default for BulkExportConfig {
    fn default() -> Self {
        Self {
            max_items: 50,
            max_bytes: 1073741824, // 1 GiB
        }
    }
}

/// TTS narration settings (M26 / spec §32.5).
///
/// Piper is the built-in local engine. Cloud adapters (ElevenLabs, AWS
/// Polly) are added later behind the same `TtsEngine` trait.
#[derive(Debug, Clone)]
pub struct TtsConfig {
    /// Which engine to use. `"piper"` is the only built-in option.
    pub engine: String,
    /// Path to the `piper` binary. Defaults to `piper` on `PATH`.
    pub piper_path: Option<PathBuf>,
    /// Path to the Piper voice model (`.onnx`).
    pub piper_voice_model: Option<PathBuf>,
    /// Default voice name (maps to a Piper model).
    pub default_voice: Option<String>,
    /// Per-instance monthly spend cap in cents (cloud engines only).
    pub monthly_spend_cap_cents: Option<u64>,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            // Local-first is the decision (spec §38.2, lorehaven.toml.example):
            // an instance narrates with the binary on its own machine rather
            // than sending a reader's text to a service. `silent` remains a
            // valid explicit choice for hosts with no synthesizer.
            engine: "piper".into(),
            piper_path: None,
            piper_voice_model: None,
            default_voice: None,
            monthly_spend_cap_cents: None,
        }
    }
}

/// Export retention settings (spec §38).
#[derive(Debug, Clone)]
pub struct ExportsConfig {
    /// How long an export's output is kept after it is produced, in days.
    /// `0` means "keep forever" — the sweep never removes exports.
    pub retention_days: i64,
    /// How long a download grant lives, in seconds. Short by design: the reader
    /// who owns the export can always be given another one, so a leaked URL
    /// has a small window.
    pub grant_ttl_secs: i64,
    /// CTA placement in exported ebooks (spec §42): per-chapter (default),
    /// per-work (last chapter only), or off.
    pub cta_placement: String,
    /// The CTA HTML, sanitized by the instance. The reader does not set this.
    pub cta_html: String,
    /// Quorum of agreeing curator marks before a work is exempt from the
    /// instance CTA (spec §42.2).
    pub cta_quorum: i64,
}

impl Default for ExportsConfig {
    fn default() -> Self {
        Self {
            retention_days: 0,
            grant_ttl_secs: 3600,
            cta_placement: "per_chapter".to_string(),
            cta_html: "<p>If you enjoyed this work, show the author some love — <strong>leave a comment</strong>, <strong>share it</strong>, or <strong>start a discussion</strong>.</p>".to_string(),
            cta_quorum: 3,
        }
    }
}

///
/// # Why this is a configuration section and not a default
///
/// Each of these makes a request the source did not simply serve. A browser
/// fingerprint is a request that does not identify itself as Lorehaven. A solver
/// is a browser somebody runs on this instance's behalf. An archived copy is
/// somebody else's copy of the page rather than the page. An instance that did
/// all three for every challenging source would be doing things its operator
/// never agreed to, so the section is empty by default and every escalation has
/// to be switched on here.
///
/// The *other* half — which sources are behind a wall at all — belongs to the
/// adapter, which is the code that knows its site. This section supplies what the
/// instance is willing to run; the adapter supplies the need. Neither can grant
/// the other's half, which is why the two are merged in
/// [`ImportsConfig::unblock_for`] rather than either being the whole answer.
#[derive(Debug, Clone)]
pub struct ImportsConfig {
    /// A FlareSolverr-compatible service this instance may drive.
    ///
    /// The protocol rather than a program: FlareSolverr, Byparr and
    /// obscura-solverr all speak it, so pointing this at any of them works and
    /// changing tools is a configuration edit. Normally a container on loopback,
    /// which is why a private address is not merely allowed here but the
    /// expected case.
    pub solver_url: Option<String>,
    /// Whether a page the source will not serve may be read from the Internet
    /// Archive instead.
    ///
    /// Off by default. It is the one escalation that reads a *different*
    /// resource, and — for a work the source has deleted — it is also the one
    /// that can put a work back in front of readers that its author withdrew.
    /// That is a preservation decision an operator should make deliberately.
    pub archive_fallback: bool,
    /// Whether a path a source's `robots.txt` forbids is refused.
    ///
    /// **On by default.** `robots.txt` is how a host states which of its pages
    /// it wants crawled, and a crawler that reads the file and then ignores it
    /// is the thing the file exists to be told about. So the compliant
    /// behaviour is what an instance does without being asked, and switching
    /// this off is a deliberate edit with a name on it.
    ///
    /// # Why an operator may switch it off
    ///
    /// Because this is a self-hosted archiving platform, and there are archives
    /// whose `robots.txt` forbids the whole site while they serve a public
    /// reading view. Where such a host has invited the public to read something,
    /// an operator may conclude that a personal import for personal reading is
    /// not what the rule was aimed at — a judgement about *their* instance, made
    /// by the only person who can answer for it. That judgement is theirs to
    /// make, and this is where they make it.
    ///
    /// # What it does not switch
    ///
    /// **Pacing.** `Crawl-delay` from the same file, and the one-second floor
    /// beneath it, are still enforced. A permission question and a load question
    /// arrive in one file; answering the first differently says nothing about
    /// the second, and an instance that overrode the permission and then
    /// hammered the host would have turned a lost permission into a lost
    /// address.
    ///
    /// **Access control.** `robots.txt` is a crawling convention, not
    /// authentication. Nothing here reads a credential, defeats a login, or
    /// reaches a page the host's own code gates — spec §11.5's prohibition on
    /// circumventing access control is untouched, and a page behind an age gate
    /// or a challenge is still out of reach.
    ///
    /// **The record.** Every overridden path is counted on the fetcher, and the
    /// first one per host is logged at `warn` naming the host and the setting.
    /// An operator who switches this on is the one who has to say how much it
    /// cost, and the count is what answers that.
    pub honour_robots: bool,
    /// What this instance does about a `Disallow` (spec §11.5,
    /// `imports.robots_posture`).
    ///
    /// `None` means the operator named no posture, and is what makes the
    /// compatibility key above meaningful: `honour_robots = false` then still
    /// selects `permissive` instead of being silently overridden by a default.
    /// See [`lorehaven_scrapers::robots::resolve_posture`], which is where the
    /// two are combined and which is the only thing that should read this
    /// field.
    pub robots_posture: Option<lorehaven_scrapers::robots::RobotsPosture>,
    /// Per-source posture overrides, narrowed only (spec §11.5 as amended §1.4).
    ///
    /// Keyed by the adapter's `key()` — the same string the source catalogue
    /// and `sources.key` use — rather than by host or by display name, because
    /// those are not unique (`efiction` alone covers several dozen members) and
    /// a name an operator retypes is a name they can mistype.
    ///
    /// Validated at load: an entry wider than the instance posture stops
    /// startup rather than being clamped. See
    /// [`lorehaven_scrapers::robots::resolve_source_override`].
    pub robots_posture_overrides: std::collections::BTreeMap<String, PostureOverride>,
}

/// A validated per-source override, narrowed at load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostureOverride {
    /// The posture this source runs, already checked against the instance's.
    pub posture: lorehaven_scrapers::robots::RobotsPosture,
    /// Whether the operator asked for this to outlive the import run that
    /// justified it (spec §1.4: it expires with the run unless extended).
    ///
    /// Defaults to false, so forgetting the key expires the override with the
    /// run rather than persisting it — the objection §11.5 raised.
    pub persistent: bool,
}

impl Default for ImportsConfig {
    fn default() -> Self {
        Self {
            solver_url: None,
            archive_fallback: false,
            // Compliance, because the alternative is a crawler nobody asked
            // for. See the field documentation for what switching it off means.
            honour_robots: true,
            robots_posture: None,
            robots_posture_overrides: std::collections::BTreeMap::new(),
        }
    }
}

/// The temporary posture overrides in force for one import run (spec §1.4).
///
/// Expiry here is a **drop**, not a timer. A run-scoped override lives in a
/// value the run owns, and when the run returns the value goes with it — so
/// there is no clock to get wrong, no sweeper to forget, and no state left
/// behind for a later run to inherit. An operator who started a run to get past
/// one blocked fetch gets exactly that: the next run of the same source reads
/// the configured posture again.
///
/// The alternative — a timestamp, compared at read time — is strictly worse and
/// was considered: it needs a clock this process does not otherwise trust, it
/// leaves an expired entry that some other code path has to notice and clear,
/// and "expired" and "never set" become two different states that both have to
/// be handled. A run is a real boundary; a timeout is a guess at one.
///
/// `persistent` is deliberately NOT honoured here. An override the operator
/// marked persistent belongs in the config file, where it is reviewed, diffed
/// and reverted like every other setting; a run that quietly promoted itself to
/// permanent would make the change invisible exactly when it matters.
#[derive(Debug, Clone, Default)]
pub struct RunScope {
    /// Per-source postures granted for this run only. An empty map is the
    /// common case and means "the run adds nothing".
    overrides: std::collections::BTreeMap<String, lorehaven_scrapers::robots::RobotsPosture>,
}

impl RunScope {
    /// A run with no temporary overrides — the ordinary case, and cheap.
    pub fn none() -> Self {
        Self::default()
    }

    /// Grant `posture` to `source` for this run only.
    ///
    /// This does NOT validate the grant. A run grant is not refused, clamped or
    /// reported here because it is not a user-supplied setting: it is a value a
    /// caller chose while doing one job, and the caller that holds the config
    /// reads the answer back through
    /// [`ImportsConfig::posture_for_source_in`], which takes the NARROWER of the
    /// file and the run. So a run that grants something wide cannot widen
    /// anything — the grant is simply outranked — and making `grant` refuse it
    /// would only move the rule to a place that cannot see the config.
    ///
    /// Refusing at read time rather than at write time is also what makes the
    /// grant honest to use: the same value is recorded (so a report can name
    /// what the run was asked for) and simultaneously not in force if it is
    /// wider than policy allows.
    pub fn grant(
        &mut self,
        source: &str,
        posture: lorehaven_scrapers::robots::RobotsPosture,
    ) -> &mut Self {
        self.overrides.insert(source.trim().to_lowercase(), posture);
        self
    }

    /// The posture this run holds for `source`, if it holds one.
    pub fn posture_for(&self, source: &str) -> Option<lorehaven_scrapers::robots::RobotsPosture> {
        self.overrides.get(&source.trim().to_lowercase()).copied()
    }

    /// Whether this run overrides anything at all. Used to decide whether a
    /// report should mention the override.
    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty()
    }
}

/// Retention settings (spec §7.7 and §25).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionConfig {
    /// The CEILING for every work's body audience.
    ///
    /// `Anyone` by default, which is §11.15's baseline: this feature narrows
    /// access and nothing here may widen it. An operator who sets a narrower
    /// default has gated every work on the instance, which is a legitimate and
    /// reversible choice. An operator who sets a WIDER one than a work names
    /// does not get that widening, because `narrowest` is what combines the
    /// levels and the work still wins — which is why this is a ceiling and not a
    /// fallback.
    pub default_body_audience: lorehaven_domain::retention::BodyAudience,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            default_body_audience: lorehaven_domain::retention::BodyAudience::Anyone,
        }
    }
}

/// Which provider answers a decision (amendment §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionProvider {
    /// The instance's own classifiers. The default, and the floor under
    /// everything else: an instance with no model running behaves exactly as
    /// it did before the feature existed.
    #[default]
    Deterministic,
    /// A decision model, consulted as a second opinion beside the
    /// deterministic answer. The model can narrow an acceptance to a hold and
    /// can do nothing else.
    Calibrated,
}

impl DecisionProvider {
    /// The wire name, for `/api/v1/meta` (§3.4: an instance that classifies
    /// with a model says so).
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Deterministic => "deterministic",
            Self::Calibrated => "calibrated",
        }
    }
}

/// Calibrated decision models (amendment §3.3).
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionsConfig {
    /// Which provider answers a decision.
    pub provider: DecisionProvider,
    /// The base URL of the serving process. Loopback by default, and a remote
    /// host is the operator's choice rather than this crate's.
    pub base_url: String,
    /// Which model to ask for.
    pub model: String,
    /// The bearer token, if the server wants one.
    ///
    /// Read from `LOREHAVEN_DECISIONS_API_KEY` rather than from the config
    /// file: a file is a thing people paste into issues, and a pasted key is a
    /// leaked key.
    pub api_key: Option<String>,
    /// How long to wait before falling back.
    pub timeout_ms: u64,
    /// The posterior at or above which an acceptance is kept.
    pub accept_threshold: f64,
    /// Below this the model is not consulted, because a near-zero posterior on
    /// "is this a work?" is the deterministic path's own answer and paying for
    /// a model to be told nothing is waste.
    pub consult_floor: f64,
}

impl Default for DecisionsConfig {
    fn default() -> Self {
        Self {
            provider: DecisionProvider::Deterministic,
            base_url: "http://127.0.0.1:8888".to_owned(),
            model: "laya".to_owned(),
            api_key: std::env::var("LOREHAVEN_DECISIONS_API_KEY").ok(),
            timeout_ms: 5_000,
            accept_threshold: 0.90,
            consult_floor: 0.10,
        }
    }
}

impl DecisionsConfig {
    /// Is a posterior worth asking anybody about?
    ///
    /// Below the floor the deterministic path already knows the answer, and
    /// above 1.0 the answer is certain enough that the model's opinion cannot
    /// change the outcome — so neither end is worth a request.
    #[must_use]
    pub fn worth_asking_about(&self, posterior: f64) -> bool {
        posterior.is_finite()
            && posterior > self.consult_floor
            && posterior <= 1.0
            && posterior < self.accept_threshold
    }

    /// Reject an unusable configuration rather than clamping it.
    ///
    /// The comparison is `!(a <= b)` and not `a < b`, which is the whole
    /// reason this function exists: every comparison against `NaN` is false,
    /// so `NaN < NaN` is false and a validation written that way ACCEPTS a
    /// `NaN` threshold. A `NaN` threshold then makes every acceptance hold
    /// forever, because `p < NaN` is false for every `p` — including 1.0 —
    /// so the direction inverts and the model becomes maximally cautious
    /// without anyone deciding that.
    pub fn check_thresholds(&self) -> Result<(), String> {
        if !self.accept_threshold.is_finite() || !(0.0..=1.0).contains(&self.accept_threshold) {
            return Err(format!(
                "decisions.accept_threshold must be a probability in 0.0..=1.0, got {}",
                self.accept_threshold
            ));
        }
        if !self.consult_floor.is_finite() || !(0.0..=1.0).contains(&self.consult_floor) {
            return Err(format!(
                "decisions.consult_floor must be a probability in 0.0..=1.0, got {}",
                self.consult_floor
            ));
        }
        // `partial_cmp` rather than `!(a <= b)`, and the difference is the
        // point. `a <= b` is false for a NaN, so `!(a <= b)` is true and does
        // reject it — but only by accident, and the next reader cannot tell
        // that from a deliberate double negative. `partial_cmp` returns `None`
        // for incomparable values, so rejecting on `None` is the obvious
        // reading, and the equality case is still accepted.
        match self.consult_floor.partial_cmp(&self.accept_threshold) {
            Some(Ordering::Less | Ordering::Equal) => {}
            _ => {
                return Err(format!(
                    "decisions.consult_floor ({}) must not exceed accept_threshold ({}): a \
                     floor above the ceiling asks about nothing, and a NaN in either is not \
                     a threshold at all",
                    self.consult_floor, self.accept_threshold
                ));
            }
        }
        Ok(())
    }
}

impl ImportsConfig {
    /// The posture this instance actually runs (spec §11.5).
    ///
    /// The posture wins when both keys are present, and `honour_robots` is the
    /// fallback for a config written before the posture existed. One function,
    /// called once, so no two call sites can disagree about the answer.
    pub fn resolved_robots_posture(&self) -> lorehaven_scrapers::robots::RobotsPosture {
        lorehaven_scrapers::robots::resolve_posture(self.robots_posture, Some(self.honour_robots))
    }

    /// The posture one source runs under, honouring a per-source override.
    ///
    /// The instance posture is the ceiling; an override can only narrow it. The
    /// narrowing itself is enforced at config load, so this cannot return a
    /// widened posture — but the instance posture is re-resolved here rather
    /// than cached, because a test or a caller may have changed it since, and a
    /// stale cached ceiling would be a silent bypass of the rule.
    pub fn posture_for_source(&self, source: &str) -> lorehaven_scrapers::robots::RobotsPosture {
        match self.robots_posture_overrides.get(source) {
            Some(override_) => override_.posture,
            None => self.resolved_robots_posture(),
        }
    }

    /// The posture one source runs under, given a run that may be holding a
    /// temporary override of its own.
    ///
    /// Two sources to the answer, and the run WINS. That ordering is the whole
    /// point of a run-scoped override: an operator starts a run to get past one
    /// blocked fetch, and a narrowing written in the config file for the same
    /// source must not be able to override the thing the operator just did by
    /// hand. A file narrowing is still a real ceiling — see
    /// [`RunScope::narrowing_only`] — but within a run the narrower of the two
    /// is taken, so neither can silently overrule the other.
    pub fn posture_for_source_in(
        &self,
        source: &str,
        run: Option<&RunScope>,
    ) -> lorehaven_scrapers::robots::RobotsPosture {
        let from_file = self.posture_for_source(source);
        match run.and_then(|run| run.posture_for(source)) {
            // `narrowest`, from the module that owns the enum. The first version
            // of this ranked the postures inline with a hand-built score:
            // `u8::from(p == Strict) + u8::from(p == MetadataOnly)`, which
            // gives Permissive a rank of 0 — the same inversion that made
            // `narrows_or_equals` wrong twice, in a second place. A rule that
            // needs "narrower" must not re-rank.
            Some(from_run) => lorehaven_scrapers::robots::narrowest(from_run, from_file),
            None => from_file,
        }
    }

    /// Every override, checked against the instance posture (spec §1.4).
    ///
    /// Called once at load so a widening entry stops startup. Called again
    /// anywhere else it would be redundant, which is deliberate: the check has
    /// one home, and a second one is a second answer.
    pub fn validate_robots_posture_overrides(&self) -> Result<(), String> {
        let instance = self.resolved_robots_posture();
        for (source, override_) in &self.robots_posture_overrides {
            lorehaven_scrapers::robots::resolve_source_override(
                source,
                override_.posture,
                instance,
            )
            .map_err(|refused| refused.to_string())?;
        }
        Ok(())
    }
}

impl ImportsConfig {
    /// The escalation chain for one source.
    ///
    /// The adapter's declaration plus what this instance is willing to run. An
    /// adapter asking for a fingerprint gets one only if the build carries the
    /// feature; an instance with a solver configured offers it only to a source
    /// that declared a wall.
    #[must_use]
    pub fn unblock_for(
        &self,
        adapter: &dyn lorehaven_scrapers::SourceAdapter,
    ) -> lorehaven_scrapers::Unblock {
        let mut unblock = adapter.unblock();
        if let Some(url) = &self.solver_url {
            unblock = unblock.with_solver(lorehaven_scrapers::SolverConfig::new(url.clone()));
        }
        if self.archive_fallback {
            unblock = unblock.with_archive();
        }
        unblock
    }
}

impl ImportsConfig {
    /// Why this source cannot be imported on this instance, when it cannot.
    ///
    /// # Why this is a refusal and not a fallback
    ///
    /// A source behind a wall needs something this instance either has or has
    /// not: a build carrying the fingerprint transport, or a solver service it can
    /// reach. Where it has not, there is no request worth making — every page
    /// would come back a challenge, and the import would fail one page at a time
    /// with a message about the wrong thing. So the answer is produced before
    /// anything is queued, and it names the fix.
    ///
    /// The same shape as the source-health refusal beside it in the routes: work
    /// that cannot succeed is refused while a reader is still looking at the page,
    /// rather than promised to a queue that will fail it later.
    ///
    /// Returned rather than logged because on a self-hosted instance the reader
    /// and the operator are the same person, and this is the one refusal they can
    /// act on themselves.
    #[must_use]
    pub fn unreachable_reason(
        &self,
        adapter: &dyn lorehaven_scrapers::SourceAdapter,
    ) -> Option<String> {
        let name = adapter.display_name();
        match adapter.wall() {
            lorehaven_scrapers::Wall::None => None,
            lorehaven_scrapers::Wall::Fingerprint => (!lorehaven_scrapers::FINGERPRINT_SUPPORTED)
                .then(|| {
                    format!(
                        "the {name} source refuses a plain request and needs a browser's TLS fingerprint, which this build \
                         was compiled without; rebuild with the `cloudflare-impersonation` feature to read it"
                    )
                }),
            lorehaven_scrapers::Wall::Solver => self.solver_url.is_none().then(|| {
                format!(
                    "the {name} source answers a bot challenge that only a driven browser can clear, and this instance has \
                     no solver service configured; run FlareSolverr, Byparr or obscura-solverr and point \
                     `imports.solver_url` at it"
                )
            }),
        }
    }
}

/// Age-policy settings.
///
/// Spec §7 requires the age machinery to be *configured*, not hard-coded to a
/// jurisdiction, and warns explicitly against allowing unrestricted child
/// registration merely because a checkbox exists.
#[derive(Debug, Clone)]
pub struct AgeConfig {
    /// The age at which a person may consent to their own data processing.
    /// Fourteen under Spanish law, which is the operator's stated basis.
    pub threshold: u8,
    /// Whether an under-threshold authorization workflow is actually in place.
    ///
    /// **False by default.** While it is false, an account that declares itself
    /// under the threshold is created in the `restricted` state: it may read,
    /// and it may not write, message or be discovered. Turning this on is a
    /// legal and operational decision, not a feature toggle.
    pub guardian_workflow_enabled: bool,
}

/// Public identity of the instance.
#[derive(Debug, Clone)]
pub struct SiteConfig {
    /// Human-readable site name.
    pub name: String,
    /// Public base URL, without a trailing slash.
    pub base_url: String,
    /// Operator contact address, for legal pages and feeds.
    pub contact_email: Option<String>,
    /// Instance topics (M30) — configurable public/private with bonus credits.
    pub topics: Vec<InstanceTopic>,
}

/// A topic an instance is about (M30 / spec §0.4).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct InstanceTopic {
    /// The operator's own label ("hurt/comfort", "omegaverse", ...).
    pub name: String,
    /// Whether the topic is shown publicly (landing page, meta API, leaderboards).
    #[serde(default)]
    pub public: bool,
    /// Extra credits a reader earns for finishing a work in this topic.
    #[serde(default = "default_bonus_credits")]
    pub bonus_credits: u32,
}

fn default_bonus_credits() -> u32 {
    0
}

/// HTTP listener settings.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Interface to bind.
    pub bind: String,
    /// Port to bind.
    pub port: u16,
    /// Maximum accepted request body size.
    pub max_body_bytes: usize,
    /// Per-request timeout.
    pub request_timeout: Duration,
    /// Allowed CORS origins. Empty means same-origin only.
    pub cors_origins: Vec<String>,
}

/// File storage settings.
#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// Root directory for all managed files.
    pub root: PathBuf,
}

/// Cookie and session settings.
#[derive(Debug, Clone)]
pub struct SecurityConfig {
    /// Whether cookies carry the `Secure` attribute.
    pub cookie_secure: bool,
    /// Where the secret key lives, when it is not in `LOREHAVEN_SECRET_KEY`.
    pub secret_key_file: Option<PathBuf>,
    /// Session lifetime.
    pub session_ttl: Duration,
    /// Whether state-changing cookie-authenticated requests need a CSRF token.
    pub csrf_required: bool,
    /// Whether `X-Forwarded-For` may be believed for rate-limit keying.
    ///
    /// Off by default, and it must stay off unless a reverse proxy is genuinely
    /// in front: the header is trivially forgeable, and trusting it lets a
    /// client mint a fresh rate-limit bucket per request.
    pub trust_proxy: bool,
}

/// Logging settings.
#[derive(Debug, Clone)]
pub struct LoggingConfig {
    /// Tracing filter directive.
    pub filter: String,
    /// Output format.
    pub format: LogFormat,
}

/// Static asset settings.
#[derive(Debug, Clone)]
pub struct AssetsConfig {
    /// Serve assets from this directory instead of the embedded bundle.
    /// Development only: a production deployment serves what was compiled in.
    pub dir: Option<PathBuf>,
}

/// Development-only affordances.
#[derive(Debug, Clone)]
pub struct DevConfig {
    /// Whether the seed command may run against this instance.
    pub seed_enabled: bool,
}

impl Config {
    /// Resolve the configuration from arguments, environment, file and defaults.
    pub fn load(global: &GlobalArgs) -> Result<Self> {
        let config_path = match &global.config {
            Some(path) => {
                if !path.exists() {
                    anyhow::bail!(
                        "configuration file {} does not exist (from --config)",
                        path.display()
                    );
                }
                Some(path.clone())
            }
            None => {
                let default = PathBuf::from(DEFAULT_CONFIG_FILE);
                default.exists().then_some(default)
            }
        };

        let file: FileConfig = match &config_path {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .with_context(|| format!("reading {}", path.display()))?;
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
            }
            None => FileConfig::default(),
        };

        let environment = match &global.environment {
            Some(value) => Environment::parse(value)?,
            None => match &file.environment {
                Some(value) => Environment::parse(value)?,
                None => Environment::Development,
            },
        };

        let production = environment.is_production();

        // --- site -----------------------------------------------------------
        let site_file = file.site.unwrap_or_default();
        let base_url = global
            .base_url
            .clone()
            .or(site_file.base_url)
            .unwrap_or_else(|| {
                let port = global.port.or(file.server.as_ref().and_then(|s| s.port));
                match port {
                    Some(port) => format!("http://localhost:{port}"),
                    None => "http://localhost:8080".to_owned(),
                }
            });

        let site = SiteConfig {
            name: site_file.name.unwrap_or_else(|| "Lorehaven".to_owned()),
            base_url: base_url.trim_end_matches('/').to_owned(),
            contact_email: site_file.contact_email,
            topics: site_file.topics.unwrap_or_default(),
        };

        // --- server ---------------------------------------------------------
        let server_file = file.server.unwrap_or_default();
        let server = ServerConfig {
            bind: global
                .bind
                .clone()
                .or(server_file.bind)
                .unwrap_or_else(|| "127.0.0.1".to_owned()),
            port: global.port.or(server_file.port).unwrap_or(8080),
            max_body_bytes: server_file.max_body_bytes.unwrap_or(2 * 1024 * 1024),
            request_timeout: Duration::from_secs(server_file.request_timeout_secs.unwrap_or(30)),
            cors_origins: server_file.cors_origins.unwrap_or_default(),
        };

        // --- storage --------------------------------------------------------
        let storage_file = file.storage.unwrap_or_default();
        let storage_root = global
            .storage_root
            .clone()
            .or(storage_file.root)
            .unwrap_or_else(|| {
                if production {
                    PathBuf::from("/var/lib/lorehaven")
                } else {
                    PathBuf::from("./data")
                }
            });

        let storage = StorageConfig {
            root: storage_root.clone(),
        };

        // --- database -------------------------------------------------------
        let database_file = file.database.unwrap_or_default();
        let database_url = global
            .database_url
            .clone()
            .or(database_file.url)
            .unwrap_or_else(|| {
                format!(
                    "sqlite://{}/lorehaven.sqlite?mode=rwc",
                    storage_root.display()
                )
            });

        let mut database = DatabaseConfig::new(database_url);
        database.max_connections =
            database_file
                .max_connections
                .unwrap_or(if production { 10 } else { 5 });
        if let Some(secs) = database_file.acquire_timeout_secs {
            database.acquire_timeout = Duration::from_secs(secs);
        }

        // --- security -------------------------------------------------------
        let security_file = file.security.unwrap_or_default();
        let security = SecurityConfig {
            // Secure cookies are the production default; development on
            // http://localhost would silently break with them on.
            cookie_secure: security_file.cookie_secure.unwrap_or(production),
            session_ttl: Duration::from_secs(
                60 * 60 * 24 * u64::from(security_file.session_ttl_days.unwrap_or(30)),
            ),
            csrf_required: security_file.csrf_required.unwrap_or(true),
            trust_proxy: security_file.trust_proxy.unwrap_or(false),
            secret_key_file: security_file.secret_key_file,
        };

        // --- logging --------------------------------------------------------
        let logging_file = file.logging.unwrap_or_default();
        let logging = LoggingConfig {
            filter: global
                .log
                .clone()
                .or(logging_file.filter)
                .unwrap_or_else(|| {
                    if production {
                        "info".to_owned()
                    } else {
                        "info,lorehaven_app=debug,lorehaven_db=debug".to_owned()
                    }
                }),
            format: match &global.log_format {
                Some(value) => LogFormat::parse(value)?,
                None => match &logging_file.format {
                    Some(value) => LogFormat::parse(value)?,
                    None if production => LogFormat::Json,
                    None => LogFormat::Pretty,
                },
            },
        };

        // --- assets ---------------------------------------------------------
        let assets_file = file.assets.unwrap_or_default();
        let assets = AssetsConfig {
            dir: assets_file.dir,
        };

        // --- development ----------------------------------------------------
        // The operator: an argument, then the environment (clap resolves those
        // two), then the file. A malformed id is a startup error rather than a
        // silently missing operator, because the failure mode of the latter is
        // an admin page nobody can open.
        let administration_file = file.administration.clone().unwrap_or_default();
        let operator_account_id = global
            .operator_account_id
            .clone()
            .or(administration_file.operator_account_id)
            .map(|raw| {
                raw.parse::<AccountId>()
                    .with_context(|| format!("the operator account id {raw:?} is not a UUID"))
            })
            .transpose()?;

        let dev_file = file.dev.unwrap_or_default();
        let dev = DevConfig {
            seed_enabled: dev_file.seed_enabled.unwrap_or(!production),
        };

        // --- accounts -------------------------------------------------------
        let accounts_file = file.accounts.unwrap_or_default();
        let accounts = AccountsConfig {
            registration_open: accounts_file.registration_open.unwrap_or(true),
        };

        // --- age policy -----------------------------------------------------
        let age_file = file.age.unwrap_or_default();
        let age = AgeConfig {
            threshold: age_file.threshold.unwrap_or(14),
            guardian_workflow_enabled: age_file.guardian_workflow_enabled.unwrap_or(false),
        };

        // --- rate limits ----------------------------------------------------
        let rate_limits = {
            let defaults = crate::limiter::Limits::default();
            let section = file.rate_limits.unwrap_or_default();
            let build =
                |burst: Option<u32>, per_minute: Option<u32>, fallback: crate::limiter::Quota| {
                    crate::limiter::Quota {
                        burst: burst.unwrap_or(fallback.burst),
                        per_minute: per_minute.unwrap_or(fallback.per_minute),
                    }
                };
            crate::limiter::Limits {
                auth: build(section.auth_burst, section.auth_per_minute, defaults.auth),
                write: build(
                    section.write_burst,
                    section.write_per_minute,
                    defaults.write,
                ),
                search: build(
                    section.search_burst,
                    section.search_per_minute,
                    defaults.search,
                ),
                export: build(
                    section.export_burst,
                    section.export_per_minute,
                    defaults.export,
                ),
                default: build(
                    section.default_burst,
                    section.default_per_minute,
                    defaults.default,
                ),
                address_multiplier: section
                    .address_multiplier
                    .unwrap_or(defaults.address_multiplier),
            }
        };

        let imports_file = file.imports.unwrap_or_default();
        let imports = ImportsConfig {
            // An empty string in a config file means "unset", not "the solver is
            // the empty URL": the latter would fail at the first challenged
            // chapter instead of at startup.
            solver_url: imports_file
                .solver_url
                .map(|url| url.trim().to_owned())
                .filter(|url| !url.is_empty()),
            archive_fallback: imports_file.archive_fallback.unwrap_or(false),
            // Compliance unless the file says otherwise. `unwrap_or(true)`
            // rather than `ImportsConfig::default()`'s value read back, because
            // this is the line that decides it and it should read that way.
            honour_robots: imports_file.honour_robots.unwrap_or(true),
            // `None` when absent, so `resolved_robots_posture` can tell a
            // deliberate posture from a default and fall back to the
            // compatibility key when there is nothing deliberate to find.
            robots_posture: imports_file.robots_posture,
            // A section with no `posture` key is dropped rather than defaulted.
            // Defaulting would invent a posture the operator did not write, and
            // §1.4's rule is that an override is something someone set
            // deliberately — so an empty table entry is a mistake worth naming
            // rather than an instruction to be strict about one source.
            robots_posture_overrides: imports_file
                .robots_posture_overrides
                .into_iter()
                .filter_map(|(source, section)| {
                    section.posture.map(|posture| {
                        (
                            // Normalised to lowercase, because `SourceKey`
                            // normalises and a config naming `AO3` would
                            // otherwise silently never match.
                            source.trim().to_lowercase(),
                            PostureOverride {
                                posture,
                                persistent: section.persistent.unwrap_or(false),
                            },
                        )
                    })
                })
                .collect(),
        };

        // --- tts ------------------------------------------------------------
        let tts_file = file.tts.unwrap_or_default();
        let tts = TtsConfig {
            // An empty string means "unset", not "an engine named """, for the
            // same reason an empty solver URL does: the failure belongs at the
            // first narration, where the message can name what to install.
            engine: tts_file
                .engine
                .map(|engine| engine.trim().to_owned())
                .filter(|engine| !engine.is_empty())
                .unwrap_or_else(|| TtsConfig::default().engine),
            piper_path: tts_file.piper_path,
            piper_voice_model: tts_file.piper_voice_model,
            default_voice: tts_file
                .default_voice
                .map(|voice| voice.trim().to_owned())
                .filter(|voice| !voice.is_empty()),
            monthly_spend_cap_cents: tts_file.monthly_spend_cap_cents,
        };

        // --- forum ----------------------------------------------------------
        let forum_file = file.forum.unwrap_or_default();
        let mut vote_budget: Vec<lorehaven_domain::typed_votes::BudgetRung> = forum_file
            .vote_budget
            .map(|rungs| {
                rungs
                    .iter()
                    .map(|(level, votes)| {
                        let level: i64 = level.trim().parse().map_err(|_| {
                            anyhow::anyhow!("forum.vote_budget key {level:?} is not a trust level")
                        })?;
                        Ok((level, *votes))
                    })
                    .collect::<anyhow::Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_else(|| ForumConfig::default().vote_budget);
        // Ascending by level, so resolution is "the highest rung reached" and
        // not "whichever key the parser happened to see last".
        vote_budget.sort_unstable_by_key(|(level, _)| *level);
        let forum = ForumConfig {
            work_discussion_default: forum_file
                .work_discussion_default
                .as_deref()
                .and_then(lorehaven_domain::work_discussion::WorkDiscussionMode::parse)
                .unwrap_or_else(|| ForumConfig::default().work_discussion_default),
            vote_budget,
            meta_mod_points: forum_file
                .meta_mod_points
                .unwrap_or_else(|| ForumConfig::default().meta_mod_points),
            meta_mod_min_verdicts: forum_file
                .meta_mod_min_verdicts
                .unwrap_or_else(|| ForumConfig::default().meta_mod_min_verdicts),
            min_vote_weight_bp: forum_file
                .min_vote_weight_bp
                .unwrap_or_else(|| ForumConfig::default().min_vote_weight_bp),
            karma_decay_percent: forum_file
                .karma_decay_percent
                .unwrap_or_else(|| ForumConfig::default().karma_decay_percent),
        };

        // --- exports ------------------------------------------------------
        let exports_file = file.exports.unwrap_or_default();
        let exports = ExportsConfig {
            retention_days: exports_file
                .retention_days
                .unwrap_or_else(|| ExportsConfig::default().retention_days),
            grant_ttl_secs: exports_file
                .grant_ttl_secs
                .unwrap_or_else(|| ExportsConfig::default().grant_ttl_secs),
            cta_placement: exports_file
                .cta_placement
                .unwrap_or_else(|| ExportsConfig::default().cta_placement),
            cta_html: exports_file
                .cta_html
                .unwrap_or_else(|| ExportsConfig::default().cta_html),
            cta_quorum: exports_file
                .cta_quorum
                .unwrap_or_else(|| ExportsConfig::default().cta_quorum),
        };

        // --- directory -----------------------------------------------------
        let directory = match file.directory {
            Some(d) => {
                let defaults = DirectoryConfig::default();
                let mut weights = defaults.trust_vote_weights;
                if let Some(w) = d.trust_vote_weights {
                    if w.len() == 7 {
                        weights = w.try_into().expect("seven weights");
                    }
                }
                let governance = d.governance.map(|g| CategoryGovernanceConfig {
                    frozen: g.frozen.unwrap_or(false),
                    max_active_categories: g
                        .max_active_categories
                        .unwrap_or(defaults.governance.max_active_categories),
                });
                DirectoryConfig {
                    extra_categories: d.extra_categories.unwrap_or_default(),
                    weighting: d.weighting.unwrap_or(defaults.weighting),
                    trust_vote_weights: weights,
                    taste_floor: d.taste_floor.unwrap_or(defaults.taste_floor),
                    taste_ceiling: d.taste_ceiling.unwrap_or(defaults.taste_ceiling),
                    governance: governance.unwrap_or(defaults.governance),
                    decay_enabled: d.decay_enabled.unwrap_or(defaults.decay_enabled),
                    decay_cutoff_days: d.decay_cutoff_days.unwrap_or(defaults.decay_cutoff_days),
                    decay_min_votes: d.decay_min_votes.unwrap_or(defaults.decay_min_votes),
                    decay_exponent: d.decay_exponent.unwrap_or(defaults.decay_exponent),
                }
            }
            None => DirectoryConfig::default(),
        };

        // --- works (spec §40) ----------------------------------------------
        let works = WorksConfig {
            max_fork_depth: file
                .works
                .as_ref()
                .and_then(|w| w.max_fork_depth)
                .unwrap_or(3),
        };

        let config = Self {
            environment,
            site,
            server,
            database,
            storage,
            security,
            logging,
            assets,
            administration: AdministrationConfig {
                operator_account_id,
                webhook_timeout_secs: file
                    .administration
                    .as_ref()
                    .and_then(|a| a.webhook_timeout_secs)
                    .unwrap_or(10),
                webhook_max_attempts: file
                    .administration
                    .as_ref()
                    .and_then(|a| a.webhook_max_attempts)
                    .unwrap_or(5),
                webhook_base_delay_ms: file
                    .administration
                    .as_ref()
                    .and_then(|a| a.webhook_base_delay_ms)
                    .unwrap_or(500),
                webhook_allowed_hosts: file
                    .administration
                    .as_ref()
                    .and_then(|a| a.webhook_allowed_hosts.clone())
                    .unwrap_or_default(),
            },
            dev,
            accounts,
            age,
            rate_limits,
            imports,
            retention: RetentionConfig {
                // `and_then(parse_stored)`: an unrecognised value yields `None`
                // and therefore the default, which is the fail-closed direction
                // for a field whose purpose is to withhold. It is NOT an error
                // at load, deliberately — `deny_unknown_fields` already refuses
                // an unknown *key*, and refusing an unknown *value* here would
                // make a typo look like a broken instance rather than a setting
                // that did not apply. The trade is stated here because it is a
                // trade: an operator who writes `default_body_audience =
                // "trusted_readers"` gets `Anyone` and no warning, and the only
                // evidence is the parsed config.
                default_body_audience: file
                    .retention
                    .as_ref()
                    .and_then(|r| r.default_body_audience.as_deref())
                    .and_then(|v| lorehaven_domain::retention::BodyAudience::parse_stored(Some(v)))
                    .unwrap_or(lorehaven_domain::retention::BodyAudience::Anyone),
            },
            decisions: {
                let base = DecisionsConfig::default();
                let d = file.decisions.as_ref();
                DecisionsConfig {
                    provider: d
                        .and_then(|d| d.provider.as_deref())
                        .map_or(base.provider, |raw| match raw {
                            "deterministic" => DecisionProvider::Deterministic,
                            "calibrated" => DecisionProvider::Calibrated,
                            // An unrecognised provider name is the DEFAULT,
                            // and the default is deterministic. The same
                            // trade as `retention` above, and it is the right
                            // direction: a typo in this field must not turn
                            // into a model being consulted, and it must not
                            // stop the instance from starting either.
                            _ => DecisionProvider::Deterministic,
                        }),
                    base_url: d.and_then(|d| d.base_url.clone()).unwrap_or(base.base_url),
                    model: d.and_then(|d| d.model.clone()).unwrap_or(base.model),
                    // The environment wins over the file. A key in a config
                    // file is a key in a git repository, a paste into an
                    // issue, and a backup.
                    api_key: std::env::var("LOREHAVEN_DECISIONS_API_KEY")
                        .ok()
                        .or_else(|| d.and_then(|d| d.api_key.clone())),
                    timeout_ms: d.and_then(|d| d.timeout_ms).unwrap_or(base.timeout_ms),
                    accept_threshold: d
                        .and_then(|d| d.accept_threshold)
                        .unwrap_or(base.accept_threshold),
                    consult_floor: d
                        .and_then(|d| d.consult_floor)
                        .unwrap_or(base.consult_floor),
                }
            },
            theme: {
                let t = file.theme.unwrap_or_default();
                ThemeConfig {
                    mode: t.mode.unwrap_or_else(default_theme_mode),
                    allow_user_opt_out: t.allow_user_opt_out.unwrap_or(true),
                    theme_dial_floor_bp: t
                        .theme_dial_floor_bp
                        .unwrap_or_else(default_theme_dial_floor_bp),
                    adaptive_max_drift_bp: t.adaptive_max_drift_bp.unwrap_or(0),
                    influence_sources: t
                        .influence_sources
                        .unwrap_or_else(default_influence_sources),
                    boost_tags: t.boost_tags.unwrap_or_default(),
                    suppress_tags: t.suppress_tags.unwrap_or_default(),
                    tag_gravity_bp: t.tag_gravity_bp.unwrap_or_default(),
                }
            },
            // §16.1a names `rec.mode` an operator control, and until this was
            // wired the value was only reachable from Rust: the section existed
            // on `Config` and nowhere in `FileConfig`, so a deployment could
            // not set it and no test failed, because a value nobody can set
            // cannot be wrong. A default is taken first so a partial
            // `[discovery]` table keeps every other knob, and `mode` is parsed
            // rather than defaulted — an unrecognised value must be refused
            // here, where the operator can see it, rather than reaching the
            // route and silently meaning `legacy`.
            discovery: {
                let d = file.discovery.unwrap_or_default();
                let mut cfg = DiscoveryConfig::default();
                if let Some(mode) = d.mode {
                    cfg.rec_mode = RecMode::parse(&mode)?;
                }
                if let Some(k) = d.rrf_k {
                    cfg.rec_rrf_k = k;
                }
                if let Some(strategies) = d.enabled_strategies {
                    cfg.rec_enabled_strategies = strategies;
                }
                cfg
            },
            tts,
            // --- bulk_export (spec §38) -----------------------------------------
            bulk_export: BulkExportConfig {
                max_items: file
                    .bulk_export
                    .as_ref()
                    .and_then(|b| b.max_items)
                    .unwrap_or_else(|| BulkExportConfig::default().max_items),
                max_bytes: file
                    .bulk_export
                    .as_ref()
                    .and_then(|b| b.max_bytes)
                    .unwrap_or_else(|| BulkExportConfig::default().max_bytes),
            },
            forum,
            exports,
            directory,
            works,
            community: CommunityConfig::default(),
            bounties: {
                let b = file.bounties.clone().unwrap_or_default();
                BountiesConfig {
                    allowed_types: match b.allowed_types {
                        Some(ref types) if !types.is_empty() => types.clone(),
                        _ => BountiesConfig::default().allowed_types,
                    },
                    min_amount: b.min_amount.unwrap_or(BountiesConfig::default().min_amount),
                    max_amount: b.max_amount.unwrap_or(BountiesConfig::default().max_amount),
                    crowdfund_activation_threshold: b
                        .crowdfund_activation_threshold
                        .unwrap_or(BountiesConfig::default().crowdfund_activation_threshold),
                }
            },
            vanguard: {
                let v = file.vanguard.clone().unwrap_or_default();
                VanguardConfig {
                    method: v.method.unwrap_or_else(|| VanguardConfig::default().method),
                    threshold_percent: v
                        .threshold_percent
                        .unwrap_or(VanguardConfig::default().threshold_percent),
                    limit: v.limit.unwrap_or(VanguardConfig::default().limit),
                    can_pin: v.can_pin.unwrap_or(VanguardConfig::default().can_pin),
                    can_nominate: v
                        .can_nominate
                        .unwrap_or(VanguardConfig::default().can_nominate),
                    can_create_clubs: v
                        .can_create_clubs
                        .unwrap_or(VanguardConfig::default().can_create_clubs),
                    public_badge: v
                        .public_badge
                        .unwrap_or(VanguardConfig::default().public_badge),
                    bounty_discount: v
                        .bounty_discount
                        .unwrap_or(VanguardConfig::default().bounty_discount),
                    pin_duration_days: v
                        .pin_duration_days
                        .unwrap_or(VanguardConfig::default().pin_duration_days),
                }
            },
            taste: {
                let t = file.taste.unwrap_or_default();
                TasteConfig {
                    gravity_strength: t.gravity_strength.unwrap_or(0.0),
                    signal_weight_mode: t
                        .signal_weight_mode
                        .unwrap_or_else(|| "taste_weighted".to_string()),
                    admin_weight: t.admin_weight.unwrap_or(1.0),
                    diversity_injection_percent: t.diversity_injection_percent.unwrap_or(0.1),
                    dimensions: t.dimensions.unwrap_or_else(|| {
                        vec![
                            "angst".to_string(),
                            "pacing".to_string(),
                            "prose_density".to_string(),
                            "canon_compliance".to_string(),
                            "trope_diversity".to_string(),
                        ]
                    }),
                }
            },
            signals: {
                let s = file.signals.unwrap_or_default();
                SignalsConfig {
                    mode: s.mode.unwrap_or_else(|| "taste_weighted".to_string()),
                    diversity_injection_percent: s.diversity_injection_percent.unwrap_or(0.1),
                }
            },
            instance: {
                let i = file.instance.unwrap_or_default();
                InstanceConfig {
                    preset: i.preset.unwrap_or_else(|| "curated_boutique".to_string()),
                    // A misspelled mode stops startup rather than falling back to
                    // the permissive default, so an operator who believed they
                    // had closed the instance finds out.
                    mode: i
                        .mode
                        .as_deref()
                        .map(InstanceMode::parse)
                        .transpose()?
                        .unwrap_or_default(),
                }
            },
            // --- meta_ranker (spec §9.10) --------------------------------------
            meta_ranker: {
                let m = file.meta_ranker.unwrap_or_default();
                MetaRankerConfig {
                    enabled: m.enabled.unwrap_or(true),
                    exploration_percent: m.exploration_percent.unwrap_or(15) as u8,
                    exploitation_percent: m.exploitation_percent.unwrap_or(85) as u8,
                    min_impressions_per_strategy: m.min_impressions_per_strategy.unwrap_or(50),
                    rebalance_frequency_hours: m.rebalance_frequency_hours.unwrap_or(24),
                    max_active_strategies: m.max_active_strategies.unwrap_or(20),
                    success_metric: m
                        .success_metric
                        .unwrap_or_else(|| "admin_aligned".to_string()),
                    auto_disable_threshold: m.auto_disable_threshold.unwrap_or(0.3),
                    candidate_exploration_bonus: m.candidate_exploration_bonus.unwrap_or(2.0),
                    candidate_promotion_impressions: m
                        .candidate_promotion_impressions
                        .unwrap_or(500),
                }
            },
            // --- revisions (spec §38) -----------------------------------------
            revisions: RevisionsConfig {
                ttl_secs: file
                    .revisions
                    .as_ref()
                    .and_then(|r| r.ttl_secs)
                    .unwrap_or_else(|| RevisionsConfig::default().ttl_secs),
            },
            // --- jobs (spec §38) ----------------------------------------------
            jobs: JobsConfig {
                slot_retention_days: file
                    .jobs
                    .as_ref()
                    .and_then(|j| j.slot_retention_days)
                    .unwrap_or_else(|| JobsConfig::default().slot_retention_days),
                terminal_retention_days: file
                    .jobs
                    .as_ref()
                    .and_then(|j| j.terminal_retention_days)
                    .unwrap_or_else(|| JobsConfig::default().terminal_retention_days),
            },
            // --- device delivery (spec §13.4 / M7-03) -------------------------
            device: file.device.as_ref().map(|d| DeviceConfig {
                kindle_email: d.kindle_email.clone(),
                device_email: d.device_email.clone(),
            }),
            // --- media_resilience (spec §32.7) -----------------------------------
            media_resilience: {
                let d = MediaResilienceConfig::default();
                let m = file.media_resilience.as_ref();
                MediaResilienceConfig {
                    enabled: m.and_then(|m| m.enabled).unwrap_or(d.enabled),
                    perceptual_hash_algorithm: match m
                        .and_then(|m| m.perceptual_hash_algorithm.as_deref())
                    {
                        Some(raw) => raw.parse().map_err(|e: String| {
                            anyhow::anyhow!("media_resilience.perceptual_hash_algorithm: {e}")
                        })?,
                        None => d.perceptual_hash_algorithm,
                    },
                    perceptual_match_threshold: m
                        .and_then(|m| m.perceptual_match_threshold)
                        .unwrap_or(d.perceptual_match_threshold),
                    require_curator_confirmation_below: m
                        .and_then(|m| m.require_curator_confirmation_below)
                        .unwrap_or(d.require_curator_confirmation_below),
                    require_curator_confirmation_above: m
                        .and_then(|m| m.require_curator_confirmation_above)
                        .unwrap_or(d.require_curator_confirmation_above),
                    audio_fingerprint: match m.and_then(|m| m.audio_fingerprint.as_deref()) {
                        Some(raw) => raw.parse().map_err(|e: String| {
                            anyhow::anyhow!("media_resilience.audio_fingerprint: {e}")
                        })?,
                        None => d.audio_fingerprint,
                    },
                    // The pre-existing keys have no TOML surface of their own;
                    // they keep the defaults they have always had.
                    min_healthy_links: d.min_healthy_links,
                    mirror_add_credits: d.mirror_add_credits,
                    archive_add_credits: d.archive_add_credits,
                    verify_credits: d.verify_credits,
                    daily_credits_cap: d.daily_credits_cap,
                    dead_threshold_failures: d.dead_threshold_failures,
                    check_interval_secs: d.check_interval_secs,
                }
            },
            // --- library (spec §38) --------------------------------------------
            roadmap: RoadmapConfig::default(),
            retention_governance: RetentionGovernanceConfig::default(),
            library: LibraryConfig {
                update_check_retention_days: file
                    .library
                    .as_ref()
                    .and_then(|l| l.update_check_retention_days)
                    .unwrap_or_else(|| LibraryConfig::default().update_check_retention_days),
                check_batch: file
                    .library
                    .as_ref()
                    .and_then(|l| l.check_batch)
                    .unwrap_or_else(|| LibraryConfig::default().check_batch),
            },
            config_path,
        };

        config.validate()?;
        // The decision thresholds are checked HERE rather than in
        // `DecisionsConfig::check_thresholds`'s callers, because a NaN
        // threshold in a config file is the one misconfiguration whose failure
        // is silent: every instance still starts, every test still passes, and
        // every acceptance is held forever because `p < NaN` is false. Refusing
        // to load is the only place it can be caught.
        config
            .decisions
            .check_thresholds()
            .map_err(anyhow::Error::msg)?;
        // A widening override stops startup. It is a separate call rather than
        // part of `validate()` because it needs the *resolved* instance
        // posture, which is itself a function of two config keys — and reading
        // `imports.robots_posture` directly here would compare against a
        // posture the instance is not actually running.
        config
            .imports
            .validate_robots_posture_overrides()
            .map_err(anyhow::Error::msg)?;
        Ok(config)
    }

    /// Build a configuration for tests and embedded use, bypassing the file.
    #[must_use]
    pub fn development_defaults() -> Self {
        Self {
            environment: Environment::Development,
            site: SiteConfig {
                name: "Lorehaven".to_owned(),
                base_url: "http://localhost:8080".to_owned(),
                contact_email: None,
                topics: Vec::new(),
            },
            server: ServerConfig {
                bind: "127.0.0.1".to_owned(),
                port: 8080,
                max_body_bytes: 2 * 1024 * 1024,
                request_timeout: Duration::from_secs(30),
                cors_origins: Vec::new(),
            },
            database: DatabaseConfig::new("sqlite://:memory:"),
            storage: StorageConfig {
                root: PathBuf::from("./data"),
            },
            security: SecurityConfig {
                cookie_secure: false,
                secret_key_file: None,
                session_ttl: Duration::from_secs(60 * 60 * 24 * 30),
                csrf_required: true,
                trust_proxy: false,
            },
            logging: LoggingConfig {
                filter: "info".to_owned(),
                format: LogFormat::Pretty,
            },
            assets: AssetsConfig { dir: None },
            administration: AdministrationConfig {
                operator_account_id: None,
                webhook_timeout_secs: 10,
                webhook_max_attempts: 5,
                webhook_base_delay_ms: 500,
                webhook_allowed_hosts: Vec::new(),
            },
            dev: DevConfig { seed_enabled: true },
            accounts: AccountsConfig {
                registration_open: true,
            },
            age: AgeConfig {
                threshold: 14,
                guardian_workflow_enabled: false,
            },
            rate_limits: crate::limiter::Limits::default(),
            // Nothing is escalated to in tests. An instance that impersonated or
            // drove a solver by default would make every test that fetches a page
            // depend on which escalations happened to be configured.
            imports: ImportsConfig::default(),
            retention: RetentionConfig::default(),
            decisions: DecisionsConfig::default(),
            theme: ThemeConfig::default(),
            discovery: DiscoveryConfig::default(),
            tts: TtsConfig::default(),
            bulk_export: BulkExportConfig::default(),
            forum: ForumConfig::default(),
            exports: ExportsConfig::default(),
            directory: DirectoryConfig::default(),
            works: WorksConfig::default(),
            community: CommunityConfig::default(),
            bounties: BountiesConfig::default(),
            vanguard: VanguardConfig::default(),
            taste: TasteConfig::default(),
            signals: SignalsConfig::default(),
            instance: InstanceConfig::default(),
            meta_ranker: MetaRankerConfig::default(),
            revisions: RevisionsConfig::default(),
            jobs: JobsConfig::default(),
            device: None,
            media_resilience: MediaResilienceConfig::default(),
            roadmap: RoadmapConfig::default(),
            retention_governance: RetentionGovernanceConfig::default(),
            library: LibraryConfig::default(),
            config_path: None,
        }
    }

    /// Reject values that cannot be sane in any environment.
    ///
    /// Environment-specific rules (a production instance must not run with
    /// development settings) live in [`crate::safety`].
    pub fn validate(&self) -> Result<()> {
        if self.server.port == 0 {
            anyhow::bail!("server.port must not be 0");
        }
        if !(1024..=64 * 1024 * 1024).contains(&self.server.max_body_bytes) {
            anyhow::bail!(
                "server.max_body_bytes must be between 1024 and 67108864, got {}",
                self.server.max_body_bytes
            );
        }
        let timeout = self.server.request_timeout.as_secs();
        if !(1..=600).contains(&timeout) {
            anyhow::bail!("server.request_timeout_secs must be between 1 and 600, got {timeout}");
        }
        if self.security.session_ttl.as_secs() == 0 {
            anyhow::bail!("security.session_ttl_days must be greater than zero");
        }
        if !self.site.base_url.starts_with("http://") && !self.site.base_url.starts_with("https://")
        {
            anyhow::bail!(
                "site.base_url must start with http:// or https://, got {:?}",
                self.site.base_url
            );
        }
        if self.site.name.trim().is_empty() {
            anyhow::bail!("site.name must not be empty");
        }
        // `rec.mode` needs no check here: it is a `RecMode`, so an unrecognised
        // value is refused by `RecMode::parse` at load time rather than
        // surviving as a string that silently means `legacy`.
        // Perceptual dedup settings (spec §32.7.2). A threshold outside the
        // 1..=32 domain would either match nothing or match everything, and both
        // are silent: the instance would claim to deduplicate and quietly not.
        if !lorehaven_domain::media_resilience::perceptual_match_threshold_is_valid(
            self.media_resilience.perceptual_match_threshold,
        ) {
            anyhow::bail!(
                "media_resilience.perceptual_match_threshold must be between 1 and 32, got {}",
                self.media_resilience.perceptual_match_threshold
            );
        }
        for (key, value) in [
            (
                "require_curator_confirmation_below",
                self.media_resilience.require_curator_confirmation_below,
            ),
            (
                "require_curator_confirmation_above",
                self.media_resilience.require_curator_confirmation_above,
            ),
        ] {
            if !(0..=100).contains(&value) {
                anyhow::bail!(
                    "media_resilience.{key} is a confidence percentage and must be between 0 and 100, got {value}"
                );
            }
        }
        if !(13..=18).contains(&self.age.threshold) {
            anyhow::bail!(
                "age.threshold must be between 13 and 18, got {}",
                self.age.threshold
            );
        }
        if self.rate_limits.write.burst == 0 || self.rate_limits.auth.burst == 0 {
            anyhow::bail!("rate limits must allow at least one request in a burst");
        }
        // Forum vote settings (spec §35.2). These are numbers a route uses to
        // refuse a vote, so a nonsense value has to stop the instance rather
        // than make every vote impossible or free.
        if self.forum.vote_budget.is_empty() {
            anyhow::bail!("forum.vote_budget must have at least one trust rung");
        }
        for (level, votes) in &self.forum.vote_budget {
            if *level < 0 {
                anyhow::bail!("forum.vote_budget trust levels must not be negative, got {level}");
            }
            if *votes < 0 {
                anyhow::bail!(
                    "forum.vote_budget must not be negative, got {votes} at level {level}"
                );
            }
        }
        if self.forum.meta_mod_points < 0 {
            anyhow::bail!("forum.meta_mod_points must not be negative");
        }
        if self.forum.meta_mod_min_verdicts < 1 {
            anyhow::bail!("forum.meta_mod_min_verdicts must be at least 1");
        }
        if !(0..=lorehaven_domain::typed_votes::WEIGHT_SCALE_BP)
            .contains(&self.forum.min_vote_weight_bp)
        {
            anyhow::bail!(
                "forum.min_vote_weight_bp must be between 0 and {}",
                lorehaven_domain::typed_votes::WEIGHT_SCALE_BP
            );
        }
        if !(0..=100).contains(&self.forum.karma_decay_percent) {
            anyhow::bail!("forum.karma_decay_percent must be between 0 and 100");
        }
        // Export retention settings (spec §38).
        if self.exports.retention_days < 0 {
            anyhow::bail!(
                "exports.retention_days must be >= 0, got {}",
                self.exports.retention_days
            );
        }
        if self.exports.grant_ttl_secs <= 0 {
            anyhow::bail!(
                "exports.grant_ttl_secs must be > 0, got {}",
                self.exports.grant_ttl_secs
            );
        }
        let cta_placement = &self.exports.cta_placement;
        if cta_placement != "per_chapter" && cta_placement != "per_work" && cta_placement != "off" {
            anyhow::bail!(
                "exports.cta_placement must be per_chapter, per_work, or off, got {}",
                cta_placement
            );
        }
        if self.exports.cta_quorum < 0 {
            anyhow::bail!(
                "exports.cta_quorum must be >= 0, got {}",
                self.exports.cta_quorum
            );
        }
        // Checked here rather than at the first challenged chapter: a solver URL
        // that does not parse is an operator's typo, and a typo should stop the
        // instance rather than surface hours later as an import that cannot read
        // one source.
        if let Some(url) = &self.imports.solver_url {
            let parsed = url::Url::parse(url)
                .map_err(|error| anyhow::anyhow!("imports.solver_url is not a URL: {error}"))?;
            if !matches!(parsed.scheme(), "http" | "https") {
                anyhow::bail!(
                    "imports.solver_url must be http or https, got {:?}",
                    parsed.scheme()
                );
            }
        }
        Ok(())
    }

    /// The filesystem path of the SQLite database, when one is in use.
    #[must_use]
    pub fn sqlite_file(&self) -> Option<PathBuf> {
        if !self.database.url.starts_with("sqlite") {
            return None;
        }
        let rest = self
            .database
            .url
            .trim_start_matches("sqlite://")
            .trim_start_matches("sqlite:");
        let path = rest.split('?').next().unwrap_or_default();
        (!path.is_empty() && path != ":memory:").then(|| PathBuf::from(path))
    }

    /// Whether the configuration came from a file on disk.
    #[must_use]
    pub fn has_config_file(&self) -> bool {
        self.config_path.is_some()
    }
}

// ---------------------------------------------------------------------------
// File representation
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    environment: Option<String>,
    site: Option<SiteSection>,
    server: Option<ServerSection>,
    database: Option<DatabaseSection>,
    storage: Option<StorageSection>,
    security: Option<SecuritySection>,
    logging: Option<LoggingSection>,
    assets: Option<AssetsSection>,
    administration: Option<AdministrationSection>,
    dev: Option<DevSection>,
    accounts: Option<AccountsSection>,
    age: Option<AgeSection>,
    rate_limits: Option<RateLimitSection>,
    imports: Option<ImportsSection>,
    tts: Option<TtsSection>,
    bulk_export: Option<BulkExportSection>,
    forum: Option<ForumSection>,
    exports: Option<ExportsSection>,
    directory: Option<DirectorySection>,
    works: Option<WorksSection>,
    revisions: Option<RevisionsSection>,
    jobs: Option<JobsSection>,
    /// Device delivery settings (spec §13.4 / M7-03).
    device: Option<DeviceSection>,
    library: Option<LibrarySection>,
    /// Media resilience settings (spec §32.7).
    media_resilience: Option<MediaResilienceSection>,
    /// Recommendation mode (spec §16.1a).
    discovery: Option<DiscoverySection>,
    theme: Option<ThemeSection>,
    /// Taste gravity settings (spec §0.4, §16.17).
    taste: Option<TasteSection>,
    /// Flexible bounty settings (spec §20.3.2).
    bounties: Option<BountiesSection>,
    /// Vanguard role settings (spec §16.18).
    vanguard: Option<VanguardSection>,
    /// Signal weighting settings (spec §9.7.3).
    signals: Option<SignalsSection>,
    /// Instance preset (spec §0.6).
    instance: Option<InstanceSection>,
    /// Meta-ranker settings (spec §9.10).
    meta_ranker: Option<MetaRankerSection>,
    /// Body-audience baseline (spec §7.7).
    retention: Option<RetentionSection>,
    /// Calibrated decision models (amendment `calibrated-decision-models.md`).
    decisions: Option<DecisionsSection>,
}

/// The `[retention]` table (spec §7.7): who may read a body by default.
///
/// This table did not exist, and the absence was invisible. `retention:
/// RetentionConfig::default()` was hardcoded at the file-load site, so
/// `default_body_audience` was loadable in a Rust test and **unreachable in a
/// deployment** — an operator control that reads as configurable and is not.
/// The same shape as `rec.mode` below, and the same lesson: a value that is
/// never read cannot be wrong, so nothing failed until something tried to set
/// it.
///
/// `deny_unknown_fields` for the same reason every other section has it: a
/// misspelled key in an operator file should be an error at load, not a setting
/// that silently does not apply.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetentionSection {
    /// The instance-wide body audience. A *ceiling* for every work, not a
    /// fallback: `narrowest` combines it with the work's own value, so a work
    /// can narrow but never widen past it.
    ///
    /// A **string**, parsed by `BodyAudience::parse_stored`, rather than the
    /// derived `Deserialize` — for two reasons. The derived form is
    /// `{"trust_at_least": 4}`, which is not the spelling stored in the column
    /// and not the spelling an operator would guess from the database. And
    /// `parse_stored` is the parser that fails closed on a misspelling, so a
    /// typo in this file inherits the instance default rather than becoming an
    /// audience nobody chose.
    default_body_audience: Option<String>,
}
/// The `[decisions]` file section.
///
/// `deny_unknown_fields` is on the file as a whole, so an unknown key inside
/// this section is refused like any other — which is the point of mirroring
/// the surrounding style rather than adding a lenient variant.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionsSection {
    /// `deterministic` or `calibrated`.
    #[serde(default)]
    provider: Option<String>,
    /// Where the model is served from.
    #[serde(default)]
    base_url: Option<String>,
    /// Which model to ask for.
    #[serde(default)]
    model: Option<String>,
    /// Read from the environment rather than here, so a key is never a
    /// file that gets pasted into a bug report.
    #[serde(default)]
    api_key: Option<String>,
    /// How long to wait before falling back to the deterministic path.
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// The posterior at or above which an acceptance is kept.
    #[serde(default)]
    accept_threshold: Option<f64>,
    /// Below this the model is not consulted at all.
    #[serde(default)]
    consult_floor: Option<f64>,
}

/// The `[discovery]` table (spec §16.1a): which recommender the instance
/// serves, and which strategies it may use.
///
/// This section existed as a Rust field only until now, which meant `rec.mode`
/// — a documented operator control — could not be set from a config file at
/// all. The type was loadable in a test and unreachable in a deployment, and
/// nothing failed, because a value that is never read cannot be wrong.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiscoverySection {
    /// `legacy`, `pluggable` or `shadow`. An unrecognised value is refused at
    /// load rather than falling back: the route dispatches on this, so a typo
    /// that fell back to `legacy` would be indistinguishable from a working
    /// instance.
    mode: Option<String>,
    /// The RRF k constant for the strategy blend.
    rrf_k: Option<f64>,
    /// Restrict the enabled strategy set. Empty means all enabled.
    enabled_strategies: Option<Vec<String>>,
}

/// The `[theme]` table (spec §0.4.6): theme mode and gravity settings.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeSection {
    mode: Option<String>,
    allow_user_opt_out: Option<bool>,
    theme_dial_floor_bp: Option<i64>,
    adaptive_max_drift_bp: Option<i64>,
    influence_sources: Option<Vec<InfluenceSourceConfig>>,
    boost_tags: Option<Vec<String>>,
    suppress_tags: Option<Vec<String>>,
    tag_gravity_bp: Option<std::collections::HashMap<String, i64>>,
}

/// The `[taste]` table (spec §0.4, §16.17): taste gravity settings.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TasteSection {
    gravity_strength: Option<f64>,
    signal_weight_mode: Option<String>,
    admin_weight: Option<f64>,
    diversity_injection_percent: Option<f64>,
    dimensions: Option<Vec<String>>,
}

/// The `[bounties]` table (spec §20.3.2): flexible bounty settings.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct BountiesSection {
    allowed_types: Option<Vec<String>>,
    min_amount: Option<i64>,
    max_amount: Option<i64>,
    crowdfund_activation_threshold: Option<f64>,
}

/// The `[vanguard]` table (spec §16.18): vanguard role settings.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct VanguardSection {
    method: Option<String>,
    threshold_percent: Option<f64>,
    limit: Option<i64>,
    can_pin: Option<bool>,
    can_nominate: Option<bool>,
    can_create_clubs: Option<bool>,
    public_badge: Option<bool>,
    bounty_discount: Option<f64>,
    pin_duration_days: Option<i64>,
}

/// The `[signals]` table (spec §9.7.3): signal weighting settings.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignalsSection {
    mode: Option<String>,
    diversity_injection_percent: Option<f64>,
}

/// The `[instance]` table (spec §0.6): instance preset.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstanceSection {
    preset: Option<String>,
    /// Who may reach the instance without signing in (spec §0.4.7).
    mode: Option<String>,
}

/// The `[meta_ranker]` table (spec §9.10): meta-ranker settings.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetaRankerSection {
    /// Enable the multi-armed bandit meta-ranker.
    enabled: Option<bool>,
    /// % of discovery slots filled by randomly selected strategy.
    exploration_percent: Option<u32>,
    /// % filled by current best-ranked strategy.
    exploitation_percent: Option<u32>,
    /// Don't rank strategy until it has enough impressions.
    min_impressions_per_strategy: Option<u64>,
    /// How often strategy weights are recomputed, in hours.
    rebalance_frequency_hours: Option<u64>,
    /// Maximum number of active strategies.
    max_active_strategies: Option<usize>,
    /// Success metric: admin_aligned, engagement, completion, hybrid.
    success_metric: Option<String>,
    /// Auto-disable threshold (success_rate < threshold for N periods).
    auto_disable_threshold: Option<f64>,
    /// Bonus multiplier for candidate strategies during exploration.
    candidate_exploration_bonus: Option<f64>,
    /// Impressions before a candidate can be promoted.
    candidate_promotion_impressions: Option<u64>,
}

/// The `[revisions]` table (spec §38): source revision cache settings.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RevisionsSection {
    /// How long a cached source revision lives, in seconds. Default 604800 (7d).
    ttl_secs: Option<i64>,
}

/// The `[jobs]` table (spec §38): job queue settings.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct JobsSection {
    /// How long terminal jobs are kept, in days. Default 30.
    terminal_retention_days: Option<i64>,
    /// How long a served recommendation slot is kept, in days. Default 3.
    slot_retention_days: Option<i64>,
}

/// The `[library]` table (spec §38): library update-check settings.
/// The `[media_resilience]` table (spec §32.7): media resilience settings.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct MediaResilienceSection {
    /// Whether media resilience runs on this instance at all.
    enabled: Option<bool>,
    /// Image perceptual hash algorithm. `dhash` is the default and the only
    /// implemented value; `phash`, `whash` and `ahash` are accepted so an
    /// operator can record intent, but a build asked to compute one refuses
    /// rather than storing a different algorithm's output.
    perceptual_hash_algorithm: Option<String>,
    /// Hamming distance at or below which two image hashes are the same image.
    perceptual_match_threshold: Option<i64>,
    /// Perceptual matches below this confidence need curator confirmation.
    require_curator_confirmation_below: Option<i64>,
    /// Perceptual matches at or above this confidence auto-attach.
    require_curator_confirmation_above: Option<i64>,
    /// Audio fingerprinting scheme: chromaprint | acoustid.
    audio_fingerprint: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LibrarySection {
    /// How long an update-check record is kept, in days. Default 90.
    update_check_retention_days: Option<i64>,
    /// How many items one library update job checks. Default 50.
    check_batch: Option<i64>,
}

/// The `[bulk_export]` table (spec §38): bulk export limits.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct BulkExportSection {
    /// How many works a bulk export may bundle. Default 50.
    max_items: Option<i64>,
    /// How many bytes a bulk export may total. Default 1073741824 (1 GiB).
    max_bytes: Option<i64>,
}

/// The `[works]` table (spec §40): fork and permission statement settings.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorksSection {
    /// Maximum fork chain depth (default 3).
    max_fork_depth: Option<u32>,
}

/// The `[tts]` table: which engine narrates, and how an operator configured it
/// (M26 / spec §32.5).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TtsSection {
    /// `piper` (local, the default) or `silent` (a pipeline check that needs no
    /// synthesizer). Cloud engines will be added behind the same `TtsEngine`.
    engine: Option<String>,
    /// Path to the `piper` binary. Unset means "whatever `piper` is on `PATH`".
    piper_path: Option<PathBuf>,
    /// Path to the Piper voice model (`.onnx`).
    piper_voice_model: Option<PathBuf>,
    /// The voice a narration uses when the request does not name one.
    default_voice: Option<String>,
    /// Per-instance monthly spend cap in cents. Only cloud engines can spend;
    /// this exists so the ceiling is configuration before an adapter that
    /// charges arrives, not a number invented in a route.
    monthly_spend_cap_cents: Option<u64>,
}

/// The `[forum]` table (spec 35.0).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ForumSection {
    /// `thread_only` (recommended), `comments_only` (legacy default), or `both`.
    work_discussion_default: Option<String>,
    /// Vote budget rungs, keyed by trust level: `vote_budget = { "1" = 10,
    /// "3" = 30, "5" = 60 }` (spec §35.2).
    vote_budget: Option<std::collections::BTreeMap<String, i64>>,
    /// Meta-mod points a TL4+ steward may spend per rolling 24h.
    meta_mod_points: Option<i64>,
    /// Verdicts needed before a caster's vote weight may decay.
    meta_mod_min_verdicts: Option<i64>,
    /// The floor a decayed vote weight never goes below, in basis points.
    min_vote_weight_bp: Option<i64>,
    /// Monthly karma decay for an inactive receiver, in percent.
    karma_decay_percent: Option<i64>,
}

/// The `[exports]` table: how long an export's output is kept, and how long a
/// download grant lives (spec §38).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportsSection {
    /// How long an export's output is kept after it is produced, in days.
    /// `0` means "keep forever" — the sweep never removes exports.
    retention_days: Option<i64>,
    /// How long a download grant lives, in seconds.
    grant_ttl_secs: Option<i64>,
    /// CTA placement: per_chapter (default), per_work, or off (spec §42).
    cta_placement: Option<String>,
    /// The CTA HTML, sanitized by the instance. The reader does not set this.
    cta_html: Option<String>,
    /// Quorum of agreeing curator marks before a work is exempt from the CTA.
    cta_quorum: Option<i64>,
}

/// `[directory]` — the resource directory (spec §39.2, §39.4).
#[derive(Debug, Default, Deserialize)]
struct DirectorySection {
    /// Extra categories beyond the seed set.
    extra_categories: Option<Vec<String>>,
    /// `flat`, `trust` or `trust_and_taste` (the default).
    weighting: Option<String>,
    /// Seven trust multipliers, TL0..TL6.
    trust_vote_weights: Option<Vec<f64>>,
    /// Floor/ceiling the taste affinity maps onto.
    taste_floor: Option<f64>,
    taste_ceiling: Option<f64>,
    /// Category governance settings (spec §45).
    governance: Option<CategoryGovernanceSection>,
    /// Vote decay (spec §39, amendment `vote-decay.md`). Default on.
    decay_enabled: Option<bool>,
    /// Age in days at which a vote is worth exactly nothing. Default 60.
    decay_cutoff_days: Option<f64>,
    /// Entries with fewer live votes than this never decay. Default 20.
    decay_min_votes: Option<i64>,
    /// The curve's shape, an integer: 1 is linear, 2 the default.
    decay_exponent: Option<u32>,
}

/// `[directory.governance]` — category governance switches (spec §45.3).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CategoryGovernanceSection {
    /// Freeze all category governance: no proposals, no votes (§45.3).
    frozen: Option<bool>,
    /// Operator-raiseable ceiling on active categories (§45.4). Default 32.
    max_active_categories: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportsSection {
    /// A FlareSolverr-compatible service, e.g. `http://127.0.0.1:8191`.
    solver_url: Option<String>,
    /// Whether an archived copy may be read as a last resort.
    archive_fallback: Option<bool>,
    /// Whether a path a source's `robots.txt` forbids is refused. Default true.
    ///
    /// DEPRECATED in favour of `robots_posture`, and kept because a config
    /// written before the posture existed still has to load. `false` reads as
    /// `permissive`.
    honour_robots: Option<bool>,
    /// `strict | metadata_only | permissive`. Wins over `honour_robots` when
    /// both are present, so a stale boolean cannot outvote a deliberate choice.
    ///
    /// An unrecognised value stops startup rather than falling back to
    /// `strict`: a typo that defaulted would look like compliance while the
    /// operator's file said something else.
    robots_posture: Option<lorehaven_scrapers::robots::RobotsPosture>,
    /// Per-source overrides, narrowed only.
    ///
    /// A table of tables in TOML: `[imports.robots_posture_overrides.ao3]` with
    /// `posture` and optional `persistent`.
    ///
    /// `#[serde(default)]` because this is a map and absence is the normal
    /// state: without it EVERY config that does not mention overrides fails to
    /// parse with "missing field robots_posture_overrides", which is how a
    /// purely additive option breaks every existing installation. Every other
    /// optional section here is an `Option<T>` and so absent-tolerant; a
    /// defaulted collection has to say so explicitly to get the same effect.
    #[serde(default)]
    robots_posture_overrides: std::collections::BTreeMap<String, PostureOverrideSection>,
}

/// One `[imports.robots_posture_overrides.<source>]` entry.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PostureOverrideSection {
    posture: Option<lorehaven_scrapers::robots::RobotsPosture>,
    /// Whether the override outlives the import run that needed it. Absent means
    /// it does not, which is §1.4's default and the reason a forgotten override
    /// cannot persist.
    persistent: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SiteSection {
    name: Option<String>,
    base_url: Option<String>,
    contact_email: Option<String>,
    topics: Option<Vec<InstanceTopic>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerSection {
    bind: Option<String>,
    port: Option<u16>,
    max_body_bytes: Option<usize>,
    request_timeout_secs: Option<u64>,
    cors_origins: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DatabaseSection {
    url: Option<String>,
    max_connections: Option<u32>,
    acquire_timeout_secs: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct StorageSection {
    root: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SecuritySection {
    cookie_secure: Option<bool>,
    session_ttl_days: Option<u32>,
    csrf_required: Option<bool>,
    trust_proxy: Option<bool>,
    /// Where the secret key lives, when it is not in the environment.
    secret_key_file: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountsSection {
    registration_open: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgeSection {
    threshold: Option<u8>,
    guardian_workflow_enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RateLimitSection {
    auth_burst: Option<u32>,
    auth_per_minute: Option<u32>,
    write_burst: Option<u32>,
    write_per_minute: Option<u32>,
    search_burst: Option<u32>,
    search_per_minute: Option<u32>,
    export_burst: Option<u32>,
    export_per_minute: Option<u32>,
    default_burst: Option<u32>,
    default_per_minute: Option<u32>,
    address_multiplier: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoggingSection {
    filter: Option<String>,
    format: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetsSection {
    dir: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
#[derive(Clone)]
struct AdministrationSection {
    operator_account_id: Option<String>,
    /// Webhook delivery settings (spec §38).
    webhook_timeout_secs: Option<u64>,
    webhook_max_attempts: Option<u32>,
    webhook_base_delay_ms: Option<u64>,
    webhook_allowed_hosts: Option<Vec<String>>,
}

/// Device delivery settings (spec §13.4 / M7-03).
///
/// Optional because the adapter is optional. When `None`, device delivery
/// returns `501 Not Implemented`.
#[derive(Debug, Default, Clone)]
pub struct DeviceConfig {
    /// Email address for Kindle delivery.
    pub kindle_email: Option<String>,
    /// Email address for generic device delivery.
    pub device_email: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceSection {
    /// Email address for Kindle delivery (M7-03 / spec §13.4).
    kindle_email: Option<String>,
    /// Email address for generic device delivery.
    device_email: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DevSection {
    seed_enabled: Option<bool>,
}

/// Ensure a directory exists, creating it when necessary.
pub fn ensure_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)
        .with_context(|| format!("creating directory {}", path.display()))?;
    Ok(())
}

impl Config {
    /// Parse a TOML body directly into a `Config` — for tests that need to
    /// assert config values without building an entire scratch instance.
    pub fn parse_from_str(body: &str) -> Result<Self> {
        let dir = std::env::temp_dir().join(format!(
            "lorehaven-parse-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("lorehaven.toml");
        std::fs::write(&path, body).expect("write config");
        Self::load(&GlobalArgs {
            config: Some(path),
            ..GlobalArgs::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_development_friendly() {
        let config = Config::load(&GlobalArgs::default()).expect("loads");
        assert_eq!(config.environment, Environment::Development);
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.bind, "127.0.0.1");
        assert!(!config.security.cookie_secure);
        assert!(config.dev.seed_enabled);
        assert!(config.assets.dir.is_none());
        assert!(config.database.url.starts_with("sqlite://"));
    }

    #[test]
    fn environment_variables_beat_defaults() {
        let args = GlobalArgs {
            port: Some(4321),
            ..GlobalArgs::default()
        };
        let config = Config::load(&args).expect("loads");
        assert_eq!(config.server.port, 4321);
        // The default base_url follows the resolved port, so links stay correct.
        assert_eq!(config.site.base_url, "http://localhost:4321");
    }

    #[test]
    fn production_flips_the_safe_defaults() {
        let args = GlobalArgs {
            environment: Some("production".to_owned()),
            storage_root: Some(PathBuf::from("/tmp/lorehaven-test")),
            ..GlobalArgs::default()
        };
        let config = Config::load(&args).expect("loads");
        assert!(config.environment.is_production());
        assert!(
            config.security.cookie_secure,
            "cookies must be Secure in production"
        );
        assert!(
            !config.dev.seed_enabled,
            "seeding must be off in production"
        );
        assert_eq!(config.logging.format, LogFormat::Json);
    }

    #[test]
    fn unknown_environments_are_refused() {
        let args = GlobalArgs {
            environment: Some("staging".to_owned()),
            ..GlobalArgs::default()
        };
        assert!(Config::load(&args).is_err());
    }

    #[test]
    fn invalid_values_are_refused_with_a_clear_rule() {
        let mut config = Config::development_defaults();
        config.server.port = 0;
        assert!(config.validate().is_err());

        let mut config = Config::development_defaults();
        config.server.max_body_bytes = 10;
        assert!(config.validate().is_err());

        let mut config = Config::development_defaults();
        config.site.base_url = "ftp://example.com".to_owned();
        assert!(config.validate().is_err());

        let mut config = Config::development_defaults();
        config.site.name = "  ".to_owned();
        assert!(config.validate().is_err());
    }

    #[test]
    fn a_config_file_is_read_and_overridden_by_the_environment() {
        let dir = std::env::temp_dir().join(format!("lorehaven-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("lorehaven.toml");
        std::fs::write(
            &path,
            r#"
environment = "development"

[site]
name = "Test Haven"

[server]
port = 7000
"#,
        )
        .expect("write config");

        let args = GlobalArgs {
            config: Some(path.clone()),
            ..GlobalArgs::default()
        };
        let config = Config::load(&args).expect("loads");
        assert_eq!(config.site.name, "Test Haven");
        assert_eq!(config.server.port, 7000);

        // An explicit argument still wins over the file.
        let args = GlobalArgs {
            config: Some(path.clone()),
            port: Some(7001),
            ..GlobalArgs::default()
        };
        let config = Config::load(&args).expect("loads");
        assert_eq!(config.server.port, 7001);
    }

    #[cfg(test)]
    fn load_from(name: &str, body: &str) -> Result<Config> {
        // The directory is removed before the write. Reusing a directory keyed
        // only by (pid, name) means a leftover file from an earlier run in the
        // same process is read instead of the body passed in here: two tests
        // sharing a name silently test the first one's config, and a refusal
        // test then reads a valid file and loads successfully. That is how the
        // perceptual-threshold refusal tests first "passed" a config with
        // whash and a threshold of 9 while supposedly asserting that nonsense
        // is rejected.
        let dir =
            std::env::temp_dir().join(format!("lorehaven-imports-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("lorehaven.toml");
        std::fs::write(&path, body).expect("write config");
        Config::load(&GlobalArgs {
            config: Some(path),
            ..GlobalArgs::default()
        })
    }

    /// `[retention] default_body_audience` is reachable from a file.
    ///
    /// This section did not exist until now, and the absence was invisible in
    /// the way that hurts most: `default_body_audience` was a real field on
    /// `Config`, so every test that needed a narrow instance baseline could set
    /// it *in Rust*, and nothing failed. A deployment could not set it at all.
    /// The same shape as `rec.mode` — a documented operator control that was
    /// loadable in a test and unreachable in production, with no error anywhere
    /// because a value nobody can set cannot be wrong.
    ///
    /// The test reads from a *file*, not by assigning a field, for exactly that
    /// reason: an assignment test passes against the broken version, which is
    /// what this one was verified to do.
    #[test]
    fn the_instance_body_audience_is_reachable_from_a_config_file() {
        use lorehaven_domain::retention::BodyAudience;

        let parsed = load_from(
            "retention-trust",
            "environment = \"development\"\n\
             [retention]\n\
             default_body_audience = \"trust_at_least:3\"\n",
        )
        .expect("an operator may set the instance body audience");
        assert_eq!(
            parsed.retention.default_body_audience,
            BodyAudience::TrustAtLeast(3),
            "the file must reach the field, parameterised level included"
        );

        // And the plain spellings round-trip, so the string form in TOML and
        // the stored form in the column are the same vocabulary.
        for (spelling, expected) in [
            ("anyone", BodyAudience::Anyone),
            ("accounts_only", BodyAudience::AccountsOnly),
            ("role_operator", BodyAudience::RoleOperator),
            ("role_vanguard", BodyAudience::RoleVanguard),
            ("role_curator", BodyAudience::RoleCurator),
            ("trust_at_least", BodyAudience::TrustAtLeast(0)),
        ] {
            let parsed = load_from(
                &format!("retention-{spelling}"),
                &format!(
                    "environment = \"development\"\n\
                     [retention]\n\
                     default_body_audience = \"{spelling}\"\n"
                ),
            )
            .expect("a known audience spelling loads");
            assert_eq!(
                parsed.retention.default_body_audience, expected,
                "{spelling}"
            );
        }
    }

    /// An unrecognised audience inherits rather than becoming real, and an
    /// unknown key is refused.
    ///
    /// Two different failures, deliberately handled two different ways.
    ///
    /// An unknown **value** falls back to `Anyone`, which is the *widest* — so
    /// this is a widening path, and the honest thing is to say so rather than
    /// dress it up. It is still the right choice: refusing the load would take
    /// the whole service down for a typo in a field whose default is the safe
    /// one.
    ///
    /// An unknown **key** is refused by `deny_unknown_fields`, because a
    /// misspelled `default_body_audience` that silently does nothing is exactly
    /// the failure this section exists to prevent.
    ///
    /// Note which of the three tests this one is: it passes against the
    /// unwired version, because it only asserts *fallback* behaviour and the
    /// unwired version falls back to exactly the same place. That is correct
    /// and it is why the other two exist.
    #[test]
    fn an_unrecognised_audience_inherits_and_an_unknown_key_is_refused() {
        use lorehaven_domain::retention::BodyAudience;

        for nonsense in ["trusted_readers", "trust_at_leastx", " anyone"] {
            let parsed = load_from(
                &format!("retention-bad-{nonsense}"),
                &format!(
                    "environment = \"development\"\n\
                     [retention]\n\
                     default_body_audience = \"{nonsense}\"\n"
                ),
            )
            .expect("a nonsense value loads rather than taking the instance down");
            assert_eq!(
                parsed.retention.default_body_audience,
                BodyAudience::Anyone,
                "{nonsense:?} must not become a real audience"
            );
        }

        // A section that is silent keeps the default.
        let parsed = load_from(
            "retention-partial",
            "environment = \"development\"\n\
             [retention]\n",
        )
        .expect("a partial retention section is valid");
        assert_eq!(parsed.retention.default_body_audience, BodyAudience::Anyone);

        assert!(
            load_from(
                "retention-misspelled",
                "environment = \"development\"\n\
                 [retention]\n\
                 default_body_audiencee = \"accounts_only\"\n",
            )
            .is_err(),
            "a misspelled key must be refused at load, not silently ignored"
        );
    }

    /// The instance default is a CEILING: a work may narrow past it, never widen.
    ///
    /// The config surface makes this reachable from a deployment for the first
    /// time, so the direction of the combination is now something an operator can
    /// observe — and the direction is the whole of §7.7's rule until the source
    /// level exists.
    #[test]
    fn the_instance_audience_is_a_ceiling_a_work_may_narrow_past_it() {
        use lorehaven_domain::retention::{narrowest, BodyAudience};

        let parsed = load_from(
            "retention-ceiling",
            "environment = \"development\"\n\
             [retention]\n\
             default_body_audience = \"accounts_only\"\n",
        )
        .expect("loads");
        let instance = parsed.retention.default_body_audience;
        assert_eq!(instance, BodyAudience::AccountsOnly);

        // A work may be stricter than the instance.
        assert_eq!(
            narrowest(instance, BodyAudience::RoleCurator),
            BodyAudience::RoleCurator
        );
        // A work may NOT be looser, which is the widening this prevents: an
        // operator editing one work must not be able to hand it to everyone.
        assert_eq!(narrowest(instance, BodyAudience::Anyone), instance);
    }

    #[test]
    fn nothing_is_escalated_to_unless_it_is_configured() {
        // The default has to be that an instance does nothing on a source's
        // behalf that its operator did not ask for: no solver, no archived copy,
        // and no fingerprint unless an adapter declares one.
        let config = Config::development_defaults();
        assert!(config.imports.solver_url.is_none());
        assert!(!config.imports.archive_fallback);
        // And it reads the rules it is asked to follow. The list above is of
        // things an instance must *not* do unasked; this is the one thing it
        // must do unasked.
        assert!(
            config.imports.honour_robots,
            "an instance complies with robots.txt unless its operator says otherwise"
        );
    }

    #[test]
    fn a_source_forbidding_what_an_adapter_needs_is_still_imported_when_the_operator_says_so() {
        // The override an operator reaches for when an archive's `robots.txt`
        // forbids the site while it serves a public reading view. Read from the
        // file rather than built by assignment, because the point is that an
        // operator can set it without a code change.
        let parsed = load_from(
            "robots",
            "environment = \"development\"\n\
             [imports]\n\
             honour_robots = false\n",
        )
        .expect("an operator may override a source's Disallow rules");
        assert!(!parsed.imports.honour_robots);

        // And the default holds when the section is present but silent about it.
        let parsed = load_from(
            "robots-default",
            "environment = \"development\"\n\
             [imports]\n\
             archive_fallback = true\n",
        )
        .expect("a partial imports section is valid");
        assert!(parsed.imports.honour_robots);
    }

    #[test]
    fn a_robots_posture_is_read_from_the_file() {
        use lorehaven_scrapers::robots::RobotsPosture;

        for (spelling, expected) in [
            ("strict", RobotsPosture::Strict),
            ("metadata_only", RobotsPosture::MetadataOnly),
            ("permissive", RobotsPosture::Permissive),
        ] {
            let parsed = load_from(
                &format!("posture-{spelling}"),
                &format!(
                    "environment = \"development\"\n\
                     [imports]\n\
                     robots_posture = \"{spelling}\"\n"
                ),
            )
            .unwrap_or_else(|e| panic!("{spelling} parses: {e}"));
            assert_eq!(
                parsed.imports.robots_posture,
                Some(expected),
                "{spelling} loads as itself"
            );
            assert_eq!(
                parsed.imports.resolved_robots_posture(),
                expected,
                "and is the posture the instance runs"
            );
        }
    }

    #[test]
    fn an_unrecognised_posture_stops_startup_rather_than_defaulting() {
        use lorehaven_scrapers::robots::RobotsPosture;

        // A typo that fell back to `strict` would look like compliance while the
        // operator's file said something else entirely — the instance would be
        // refusing fetches the operator believes it is allowed to make, and the
        // file gives no sign of it. This matches how `access_mode` and
        // `rec.mode` already behave.
        //
        // The refusal is asserted at the deserialiser rather than through the
        // file loader, because the loader wraps every parse failure in a
        // message naming only the file — so a test written there proves the
        // loader ran and nothing about which value was wrong. What has to be
        // true is that this enum refuses, and the loader's refusal is a
        // consequence of it.
        let err = serde_json::from_str::<RobotsPosture>("\"strict-but-maybe\"")
            .expect_err("an unrecognised posture is refused, not defaulted");
        let text = err.to_string();
        assert!(
            text.contains("strict-but-maybe") || text.contains("unknown variant"),
            "the message names the offending value or says it is unknown, so an operator can \
             see which line is wrong: {text}"
        );

        // And the whole config really does refuse to load, not just this value
        // in isolation.
        load_from(
            "posture-typo",
            "environment = \"development\"\n\
             [imports]\n\
             robots_posture = \"strict-but-maybe\"\n",
        )
        .expect_err("and the operator's file is refused outright, so startup stops");
    }

    #[test]
    fn the_posture_wins_over_a_stale_honour_robots() {
        use lorehaven_scrapers::robots::RobotsPosture;

        // Both keys present, and they disagree. The posture wins, because the
        // boolean is the key nobody edits any more: letting it outvote the
        // posture would mean an operator's deliberate choice is silently
        // replaced by a value they left behind months ago.
        let parsed = load_from(
            "posture-wins",
            "environment = \"development\"\n\
             [imports]\n\
             honour_robots = false\n\
             robots_posture = \"strict\"\n",
        )
        .expect("both keys parse");
        assert!(
            !parsed.imports.honour_robots,
            "the boolean is preserved exactly as written"
        );
        assert_eq!(
            parsed.imports.resolved_robots_posture(),
            RobotsPosture::Strict,
            "and the posture is what the fetcher reads, so `strict` wins over the stale `false`"
        );
    }

    #[test]
    fn honour_robots_still_selects_permissive_when_no_posture_is_named() {
        use lorehaven_scrapers::robots::RobotsPosture;

        // The compatibility path, and the one an existing config takes. A
        // version of `resolve_posture` that took `RobotsPosture` rather than
        // `Option<RobotsPosture>` returned the default `Strict` here and made
        // this file's own instruction a no-op — the key parsed, and nothing
        // obeyed it.
        let parsed = load_from(
            "posture-compat",
            "environment = \"development\"\n\
             [imports]\n\
             honour_robots = false\n",
        )
        .expect("a pre-posture config still loads");
        assert_eq!(parsed.imports.robots_posture, None, "no posture was named");
        assert_eq!(
            parsed.imports.resolved_robots_posture(),
            RobotsPosture::Permissive,
            "`honour_robots = false` still means permissive"
        );

        // And the absence of both keys is strict, which is what an
        // unconfigured instance does.
        let parsed = load_from(
            "posture-absent",
            "environment = \"development\"\n\
             [imports]\n\
             archive_fallback = true\n",
        )
        .expect("a partial imports section is valid");
        assert_eq!(parsed.imports.robots_posture, None);
        assert_eq!(
            parsed.imports.resolved_robots_posture(),
            RobotsPosture::Strict
        );
    }

    #[test]
    fn a_per_source_override_may_only_narrow() {
        // The whole table, in one place, because "narrowing" is a relation and a
        // relation tested one pair at a time leaves the untested pairs to
        // whatever the code happens to do.
        //
        //     instance \ override   strict   metadata_only   permissive
        //     strict                   ok       REFUSED         REFUSED
        //     metadata_only            ok       ok              REFUSED
        //     permissive               ok       ok              ok
        //
        // `postures` and each row of `expected` are in the SAME order
        // (strict, metadata_only, permissive), and `expected[i][j]` answers
        // "instance `postures[i]`, override `postures[j]`". The first version
        // of this test wrote the table above in one order and indexed it in
        // another, and the resulting failure read as though the *rule* were
        // inverted rather than the table — which is a trap worth avoiding by
        // keeping the comment and the array in the same sequence.
        use lorehaven_scrapers::robots::RobotsPosture;

        let postures = [
            RobotsPosture::Strict,
            RobotsPosture::MetadataOnly,
            RobotsPosture::Permissive,
        ];
        // `true` where the override is at least as cautious as the instance.
        let expected = [
            // instance is strict: nothing is narrower than it.
            [true, false, false],
            // instance is metadata_only: only strict narrows.
            [true, true, false],
            // instance is permissive: everything narrows, including itself.
            [true, true, true],
        ];

        for (instance, row) in postures.iter().zip(expected.iter()) {
            for (override_posture, allowed) in postures.iter().zip(row.iter()) {
                let result = lorehaven_scrapers::robots::resolve_source_override(
                    "ao3",
                    *override_posture,
                    *instance,
                );
                assert_eq!(
                    result.is_ok(),
                    *allowed,
                    "instance {instance:?} with an override of {override_posture:?} is {}",
                    if *allowed { "allowed" } else { "REFUSED" }
                );
                if *allowed {
                    assert_eq!(result.expect("allowed"), *override_posture);
                }
            }
        }
    }

    #[test]
    fn a_widening_override_is_refused_by_name() {
        use lorehaven_scrapers::robots::RobotsPosture;

        // The refusal has to be legible, because the operator's next action is
        // to read it. A message that says only "invalid config" leaves them
        // guessing which of the three settings is wrong and which way.
        let error = lorehaven_scrapers::robots::resolve_source_override(
            "tgstorytime",
            RobotsPosture::Permissive,
            RobotsPosture::Strict,
        )
        .expect_err("widening is refused");
        let text = error.to_string();
        for needed in [
            "tgstorytime",            // which source
            "permissive",             // what was asked for
            "strict",                 // what the instance runs
            "may only narrow",        // the rule
            "imports.robots_posture", // where to change the instance instead
        ] {
            assert!(
                text.contains(needed),
                "the message must name {needed:?}: {text}"
            );
        }
    }

    #[test]
    fn a_narrowing_override_is_read_from_the_file_and_applies_to_that_source_only() {
        use lorehaven_scrapers::robots::RobotsPosture;

        let parsed = load_from(
            "override-narrowing",
            "environment = \"development\"\n\
             [imports]\n\
             robots_posture = \"permissive\"\n\
             \n\
             [imports.robots_posture_overrides.tgstorytime]\n\
             posture = \"strict\"\n\
             \n\
             [imports.robots_posture_overrides.ao3]\n\
             posture = \"metadata_only\"\n",
        )
        .expect("a narrowing override is valid: the instance is permissive, so both narrow");

        assert_eq!(
            parsed.imports.posture_for_source("tgstorytime"),
            RobotsPosture::Strict,
            "this source is narrowed to strict"
        );
        assert_eq!(
            parsed.imports.posture_for_source("ao3"),
            RobotsPosture::MetadataOnly,
            "and this one to metadata_only"
        );
        assert_eq!(
            parsed.imports.posture_for_source("royalroad"),
            RobotsPosture::Permissive,
            "a source with no override runs the instance posture — an override is per source, \
             and one source's narrowing must not reach another's fetches"
        );
    }

    #[test]
    fn a_widening_override_stops_startup() {
        // Refused rather than clamped. A clamp is indistinguishable from
        // compliance: the operator set `permissive` for one source, the instance
        // stayed `strict`, the source was crawled strictly, and nothing anywhere
        // said the setting was not in force.
        let error = load_from(
            "override-widening",
            "environment = \"development\"\n\
             [imports]\n\
             robots_posture = \"strict\"\n\
             \n\
             [imports.robots_posture_overrides.tgstorytime]\n\
             posture = \"permissive\"\n",
        )
        .expect_err("a widening override does not load");
        let text = error.to_string();
        assert!(
            text.contains("tgstorytime") && text.contains("may only narrow"),
            "the startup error names the source and the rule: {text}"
        );
    }

    #[test]
    fn an_override_with_no_posture_key_is_dropped_rather_than_defaulted() {
        // A table entry with nothing in it is a mistake, not an instruction. It
        // is dropped rather than defaulted to `strict`, because defaulting would
        // apply a narrowing the operator never wrote to one source — a silent
        // change in behaviour, in the safe direction, which is exactly the kind
        // of change nobody reports.
        let parsed = load_from(
            "override-empty",
            "environment = \"development\"\n\
             [imports]\n\
             [imports.robots_posture_overrides.ao3]\n\
             persistent = true\n",
        )
        .expect("the file still loads");
        assert!(
            parsed.imports.robots_posture_overrides.is_empty(),
            "an entry with no posture is not an override: {:?}",
            parsed.imports.robots_posture_overrides
        );
        assert_eq!(
            parsed.imports.posture_for_source("ao3"),
            parsed.imports.resolved_robots_posture(),
            "so the source runs the instance posture, unchanged"
        );
    }

    #[test]
    fn an_override_key_is_matched_case_insensitively() {
        use lorehaven_scrapers::robots::RobotsPosture;

        // `SourceKey` normalises to lowercase, so a config naming `AO3` must
        // match the `ao3` adapter or the override would be silently inert — the
        // worst shape of no-op, because the file says the source is strict and
        // the instance is permissive.
        let parsed = load_from(
            "override-case",
            "environment = \"development\"\n\
             [imports]\n\
             robots_posture = \"permissive\"\n\
             [imports.robots_posture_overrides.\"  AO3  \"]\n\
             posture = \"strict\"\n",
        )
        .expect("the key is trimmed and lowercased");
        assert_eq!(
            parsed.imports.posture_for_source("ao3"),
            RobotsPosture::Strict,
            "a padded, upper-case key still reaches the adapter"
        );
    }

    #[test]
    fn a_config_that_never_mentions_overrides_still_loads() {
        // The regression net for `#[serde(default)]`.
        //
        // Overrides are a purely ADDITIVE option, and adding a non-`Option` map
        // to a `deny_unknown_fields` struct without a default makes every
        // existing config fail to parse. The first version of this feature did
        // exactly that: seven unrelated tests failed with "missing field
        // robots_posture_overrides", and nothing about a per-source posture
        // override has anything to do with solver URLs or archive fallback.
        //
        // The point is not that one TOML fragment parses. It is that the ABSENCE
        // of a key a feature added is still a valid configuration, which is the
        // property no test of the feature's own happy path would catch.
        let parsed = load_from(
            "no-overrides",
            "environment = \"development\"\n\
             [imports]\n\
             solver_url = \"http://127.0.0.1:8191\"\n\
             archive_fallback = true\n",
        )
        .expect("a config with no overrides section is an ordinary config");
        assert!(
            parsed.imports.robots_posture_overrides.is_empty(),
            "and it has no overrides"
        );
        // Also the fully bare case: the section present, the key absent.
        let bare = load_from("bare-imports", "environment = \"development\"\n[imports]\n")
            .expect("an empty [imports] section is an ordinary config");
        assert!(bare.imports.robots_posture_overrides.is_empty());
    }

    #[test]
    fn a_run_scope_holds_an_override_only_while_the_run_lives() {
        use crate::config::RunScope;
        use lorehaven_scrapers::robots::RobotsPosture;

        // The expiry mechanism IS the scope's lifetime. There is no clock, so
        // "expired" and "the run ended" are the same event and cannot drift
        // apart — which is the property this test exists to pin. A design with
        // an `expires_at` timestamp would need a second test for "the clock
        // passed" and would still have to handle the "never expires" case
        // separately; both of those are the bugs this avoids by construction.
        let mut scope = RunScope::none();
        assert!(scope.is_empty(), "a run with no grants overrides nothing");
        assert_eq!(scope.posture_for("tgstorytime"), None);

        scope.grant("tgstorytime", RobotsPosture::Strict);
        assert_eq!(
            scope.posture_for("tgstorytime"),
            Some(RobotsPosture::Strict),
            "inside the run the grant is in force"
        );
        assert_eq!(
            scope.posture_for("ao3"),
            None,
            "and it is per source: another source in the SAME run is unaffected"
        );

        // The run ends. This is the whole mechanism — not a call to `expire()`,
        // not a field flipping, just the value going out of scope.
        drop(scope);
        assert!(
            RunScope::none().is_empty(),
            "the next run starts with nothing, because it got a new empty scope"
        );
    }

    #[test]
    fn a_run_grant_cannot_widen_what_the_config_allows() {
        use crate::config::RunScope;
        use lorehaven_scrapers::robots::RobotsPosture;

        // A run grant is not refused when it is made — it is OUTRANKED when it
        // is read. Which means a caller that tries to get past a refusal by
        // granting itself `permissive` gets the configured posture anyway, and
        // the run cannot become a hole in the rule.
        let config = config_with_posture(RobotsPosture::MetadataOnly);
        let mut scope = RunScope::none();

        // Wider grant: outranked.
        scope.grant("ao3", RobotsPosture::Permissive);
        assert_eq!(
            config.posture_for_source_in("ao3", Some(&scope)),
            RobotsPosture::MetadataOnly,
            "a run asking for permissive gets the configured posture, not the wider one"
        );

        // Narrower grant: honoured, because it costs nothing and was asked for.
        scope.grant("ao3", RobotsPosture::Strict);
        assert_eq!(
            config.posture_for_source_in("ao3", Some(&scope)),
            RobotsPosture::Strict,
            "a run asking for something stricter gets it"
        );

        // No run at all: the file's answer, unchanged.
        assert_eq!(
            config.posture_for_source_in("ao3", None),
            RobotsPosture::MetadataOnly,
            "and with no run in scope the config answers for itself"
        );
    }

    #[test]
    fn a_run_scope_matches_its_source_key_the_way_the_config_does() {
        use crate::config::RunScope;
        use lorehaven_scrapers::robots::RobotsPosture;

        // If the scope trimmed/lowercased and the config did not (or the other
        // way round), a grant would be recorded and never found, and the run
        // would silently behave as though nothing had been granted. Same
        // normalisation, same answer.
        let mut scope = RunScope::none();
        scope.grant("  AO3  ", RobotsPosture::Strict);
        assert_eq!(scope.posture_for("ao3"), Some(RobotsPosture::Strict));
        assert_eq!(scope.posture_for("AO3"), Some(RobotsPosture::Strict));
    }

    /// A config whose instance posture is `posture` and which has no overrides.
    fn config_with_posture(posture: lorehaven_scrapers::robots::RobotsPosture) -> ImportsConfig {
        ImportsConfig {
            robots_posture: Some(posture),
            ..ImportsConfig::default()
        }
    }

    #[test]
    fn tts_defaults_to_local_piper() {
        // Local-first is the decision: an instance narrates with the binary on
        // its own machine rather than sending a reader's text to a service.
        let config = Config::development_defaults();
        assert_eq!(config.tts.engine, "piper");
        assert!(config.tts.piper_path.is_none());
        assert!(config.tts.piper_voice_model.is_none());
        assert!(
            config.tts.monthly_spend_cap_cents.is_none(),
            "no instance has a cloud budget until it configures one"
        );
    }

    #[test]
    fn a_tts_section_is_read_from_the_file() {
        let parsed = load_from(
            "tts",
            "environment = \"development\"\n\
             [tts]\n\
             engine = \"piper\"\n\
             piper_path = \"/opt/piper/piper\"\n\
             piper_voice_model = \"/opt/piper/en_US-lessac-medium.onnx\"\n\
             default_voice = \"en_US-lessac-medium\"\n",
        )
        .expect("a [tts] section parses");
        assert_eq!(parsed.tts.engine, "piper");
        assert_eq!(
            parsed.tts.piper_path.expect("path"),
            PathBuf::from("/opt/piper/piper")
        );
        assert_eq!(
            parsed.tts.piper_voice_model.expect("model"),
            PathBuf::from("/opt/piper/en_US-lessac-medium.onnx")
        );
        assert_eq!(
            parsed.tts.default_voice.as_deref(),
            Some("en_US-lessac-medium")
        );
    }

    #[test]
    fn an_empty_tts_engine_is_unset_rather_than_an_empty_name() {
        // `engine = ""` would otherwise become an engine the builder cannot
        // name, and the operator would meet the failure at the first narration
        // instead of at startup. The default is what an empty value means.
        let parsed = load_from(
            "tts-empty",
            "environment = \"development\"\n[tts]\nengine = \"  \"\n",
        )
        .expect("an empty engine is tolerated");
        assert_eq!(parsed.tts.engine, "piper");
    }

    #[test]
    fn an_unknown_key_in_the_tts_section_is_refused() {
        // `deny_unknown_fields`, like every other section: a typo in a config
        // file must be an error naming the file, not a setting that is silently
        // ignored.
        let error = load_from(
            "tts-typo",
            "environment = \"development\"\n[tts]\npiper_voice = \"x\"\n",
        )
        .expect_err("an unknown tts key is refused");
        assert!(
            error.to_string().contains("tts") || error.to_string().contains("unknown field"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn the_shipped_example_configuration_parses() {
        // The example is what an operator copies, so a section it documents and
        // the parser does not accept is a startup failure handed to every new
        // instance. Every other config test writes its own file; this one reads
        // the one that ships.
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lorehaven.toml.example");
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("the example config must exist: {error}"));
        // `load` needs a real path, and the example is not writable in a
        // checkout, so it is parsed through the same entry point by copying it.
        let dir = std::env::temp_dir().join(format!("lorehaven-example-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let target = dir.join("lorehaven.toml");
        std::fs::write(&target, &body).expect("copy the example");
        let parsed = Config::load(&GlobalArgs {
            config: Some(target),
            ..GlobalArgs::default()
        })
        .expect("the shipped example must be a configuration this build accepts");

        // Named settings the example documents, asserted so a rename in the
        // parser cannot leave the example describing something that no longer
        // exists — `deny_unknown_fields` would not catch a *comment*.
        assert!(parsed.imports.honour_robots);
        assert!(parsed.imports.solver_url.is_none());
        assert!(!parsed.imports.archive_fallback);
        // And the sections themselves: a section that exists only in the parser
        // leaves every operator copying the example without the setting.
        assert_eq!(parsed.tts.engine, "piper");
        assert!(parsed.tts.piper_path.is_none());
        for section in [
            "[decisions]",
            "[site]",
            "[server]",
            "[database]",
            "[storage]",
            "[security]",
            "[logging]",
            "[assets]",
            "[imports]",
            "[tts]",
            "[forum]",
            "[dev]",
        ] {
            assert!(
                body.contains(section),
                "the example config must document {section}"
            );
        }
    }

    #[test]
    fn a_solver_url_is_read_from_the_config_file() {
        let parsed = load_from(
            "solver",
            "environment = \"development\"\n\
             [imports]\n\
             solver_url = \"http://127.0.0.1:8191\"\n\
             archive_fallback = true\n",
        )
        .expect("a solver on loopback is a valid configuration");
        assert_eq!(
            parsed.imports.solver_url.as_deref(),
            Some("http://127.0.0.1:8191")
        );
        assert!(parsed.imports.archive_fallback);
    }

    #[test]
    fn a_malformed_solver_url_stops_the_instance() {
        // A typo in an operator's config should stop the instance rather than
        // surface hours later as an import that cannot read one source.
        for (name, url) in [("typo", "not a url"), ("scheme", "ftp://solver")] {
            let body =
                format!("environment = \"development\"\n[imports]\nsolver_url = \"{url}\"\n");
            assert!(
                load_from(name, &body).is_err(),
                "a solver URL of {url:?} was accepted"
            );
        }
    }

    #[test]
    fn an_empty_solver_url_means_unset_rather_than_the_empty_url() {
        let parsed = load_from(
            "empty",
            "environment = \"development\"\n[imports]\nsolver_url = \"  \"\n",
        )
        .expect("an empty value is not an error");
        assert!(parsed.imports.solver_url.is_none());
    }

    #[test]
    fn unknown_config_keys_are_refused() {
        let dir = std::env::temp_dir().join(format!("lorehaven-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("lorehaven.toml");
        std::fs::write(&path, "[server]\nprot = 7000\n").expect("write config");

        let args = GlobalArgs {
            config: Some(path),
            ..GlobalArgs::default()
        };
        let error = Config::load(&args).expect_err("must refuse the typo");
        assert!(
            format!("{error:#}").contains("prot"),
            "the error should name the offending key: {error:#}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_explicit_config_file_is_an_error() {
        let args = GlobalArgs {
            config: Some(PathBuf::from("/nonexistent/lorehaven.toml")),
            ..GlobalArgs::default()
        };
        assert!(Config::load(&args).is_err());
    }

    #[test]
    fn the_sqlite_path_is_extracted_for_backup_and_doctor_checks() {
        let mut config = Config::development_defaults();
        config.database.url = "sqlite://./data/lorehaven.sqlite?mode=rwc".to_owned();
        assert_eq!(
            config.sqlite_file().expect("path"),
            PathBuf::from("./data/lorehaven.sqlite")
        );

        config.database.url = "postgres://user@host/lorehaven".to_owned();
        assert!(config.sqlite_file().is_none());

        config.database.url = "sqlite://:memory:".to_owned();
        assert!(config.sqlite_file().is_none());
    }

    #[test]
    fn theme_defaults_to_thematic_with_operator_topics() {
        // Spec §0.4.6: the default is thematic — an instance that declares
        // topics is assumed to want them to matter.
        let config = Config::development_defaults();
        assert_eq!(config.theme.mode, "thematic");
        assert!(config.theme.allow_user_opt_out);
        assert_eq!(config.theme.theme_dial_floor_bp, 1000);
        assert_eq!(config.theme.adaptive_max_drift_bp, 0);
        assert_eq!(config.theme.influence_sources.len(), 1);
        assert_eq!(
            config.theme.influence_sources[0].kind,
            InfluenceSourceKind::OperatorTopics
        );
    }

    #[test]
    fn media_resilience_section_is_parsed_from_toml() {
        let config = load_from(
            "media_resilience-parsed",
            r#"
[media_resilience]
enabled = false
perceptual_hash_algorithm = "whash"
perceptual_match_threshold = 9
require_curator_confirmation_below = 4
require_curator_confirmation_above = 1
audio_fingerprint = "acoustid"
"#,
        )
        .expect("loads");
        assert!(!config.media_resilience.enabled);
        assert_eq!(
            config.media_resilience.perceptual_hash_algorithm.as_str(),
            "whash"
        );
        assert_eq!(config.media_resilience.perceptual_match_threshold, 9);
        assert_eq!(
            config.media_resilience.require_curator_confirmation_below,
            4
        );
        assert_eq!(
            config.media_resilience.require_curator_confirmation_above,
            1
        );
        assert_eq!(
            config.media_resilience.audio_fingerprint.as_str(),
            "acoustid"
        );
    }

    #[test]
    fn media_resilience_defaults_match_the_spec_block() {
        let config = Config::load(&GlobalArgs::default()).expect("loads");
        // Spec §32.7.2 states these defaults; the earlier keys are the
        // long-standing curator-credit and health-check values.
        assert!(config.media_resilience.enabled);
        // `dhash`, not `phash`: dHash is the only algorithm this build
        // computes, so a default naming another one would advertise a dedup the
        // stock instance cannot perform. Spec §32.7.2's example is updated to
        // match.
        assert_eq!(
            config.media_resilience.perceptual_hash_algorithm.as_str(),
            "dhash"
        );
        assert_eq!(config.media_resilience.perceptual_match_threshold, 6);
        assert_eq!(
            config.media_resilience.require_curator_confirmation_below,
            3
        );
        assert_eq!(
            config.media_resilience.require_curator_confirmation_above,
            0
        );
        assert_eq!(
            config.media_resilience.audio_fingerprint.as_str(),
            "chromaprint"
        );
    }

    #[test]
    fn the_default_image_fingerprint_is_one_this_build_actually_computes() {
        // A default of `phash` while the only implemented algorithm is dHash
        // means a stock instance advertises a perceptual dedup it cannot
        // perform. The default has to name something real.
        let d = MediaResilienceConfig::default();
        assert_eq!(
            d.perceptual_hash_algorithm,
            PerceptualHashAlgorithm::Dhash,
            "the default must be the algorithm the fetcher implements"
        );
    }

    #[test]
    fn an_unknown_perceptual_hash_algorithm_is_refused() {
        // A typo'd algorithm must not load silently: the instance would then
        // claim to deduplicate with a hash it never computes.
        let err = load_from(
            "media_resilience-unknown-algorithm",
            r#"
[media_resilience]
perceptual_hash_algorithm = "nonsense"
"#,
        )
        .expect_err("refuses an unknown algorithm");
        assert!(
            err.to_string().contains("perceptual_hash_algorithm"),
            "unhelpful error: {err}"
        );
    }

    #[test]
    fn an_out_of_range_perceptual_threshold_is_refused() {
        let err = load_from(
            "media_resilience-bad-threshold",
            r#"
[media_resilience]
perceptual_match_threshold = 64
"#,
        )
        .expect_err("refuses a threshold above the 32-bit domain");
        assert!(
            err.to_string().contains("perceptual_match_threshold"),
            "unhelpful error: {err}"
        );
    }

    #[test]
    fn theme_section_is_parsed_from_toml() {
        let config = load_from(
            "theme",
            r#"
[theme]
mode = "generic"
allow_user_opt_out = false
theme_dial_floor_bp = 2500
adaptive_max_drift_bp = 500

[[theme.influence_sources]]
kind = "operator_topics"

[[theme.influence_sources]]
kind = "long_term_users"
min_tenure_days = 120
min_contributions = 8
"#,
        )
        .expect("loads");
        assert_eq!(config.theme.mode, "generic");
        assert!(!config.theme.allow_user_opt_out);
        assert_eq!(config.theme.theme_dial_floor_bp, 2500);
        assert_eq!(config.theme.adaptive_max_drift_bp, 500);
        assert_eq!(config.theme.influence_sources.len(), 2);
        assert_eq!(
            config.theme.influence_sources[0].kind,
            InfluenceSourceKind::OperatorTopics
        );
        assert_eq!(
            config.theme.influence_sources[1].kind,
            InfluenceSourceKind::LongTermUsers
        );
        assert_eq!(config.theme.influence_sources[1].min_tenure_days, 120);
        assert_eq!(config.theme.influence_sources[1].min_contributions, 8);
    }

    #[test]
    fn theme_unknown_mode_is_still_loaded_but_named() {
        // The mode is a string; an unknown value is not a parse error (the
        // operator may be on a newer spec), but the discovery route treats
        // anything it does not recognise as generic — no gravity.
        let config = load_from(
            "theme-unknown",
            r#"
[theme]
mode = "quantum"
"#,
        )
        .expect("loads");
        assert_eq!(config.theme.mode, "quantum");
    }

    #[test]
    fn rec_mode_loads_each_named_value() {
        // §16.1a: three named values, and a config file is how an operator sets
        // one. Round-tripped through the real file loader rather than assigned in
        // Rust, so this covers the path an operator actually takes.
        for (spelling, expected) in [
            ("legacy", RecMode::Legacy),
            ("pluggable", RecMode::Pluggable),
            ("shadow", RecMode::Shadow),
        ] {
            let config = load_from(
                "rec-mode-valid",
                &format!("[discovery]\nmode = \"{spelling}\"\n"),
            )
            .expect("a named rec mode loads");
            assert_eq!(config.discovery.rec_mode, expected);
        }
    }

    #[test]
    fn a_misspelled_rec_mode_is_refused_at_load() {
        // The reason `rec_mode` is a typed enum rather than a String. The route
        // dispatches on the value, so `Pluggable`, a trailing space or
        // `shadow-mode` would all reach the legacy branch — and an operator who
        // typed `plugggable` would have no way to tell a typo from a bug. A
        // refusal at load names the problem instead.
        let error = load_from("rec-mode-bad", "[discovery]\nrec_mode = \"plugggable\"\n")
            .expect_err("a misspelling is refused");
        let text = format!("{error:#}");
        assert!(
            text.contains("plugggable"),
            "the error names what was read: {text}"
        );
    }

    #[test]
    fn rec_mode_is_case_and_space_insensitive() {
        // An operator's TOML should not fail on a capital letter. The
        // normalisation lives in `parse` so both the file loader and any other
        // caller get it.
        assert_eq!(
            RecMode::parse("  Shadow  ").expect("normalised"),
            RecMode::Shadow
        );
        assert_eq!(
            RecMode::parse("PLUGGABLE").expect("normalised"),
            RecMode::Pluggable
        );
    }

    #[test]
    fn the_default_instance_is_public() {
        // Spec §7 requires anonymous reading of suitable public fiction to stay
        // available, so a fresh instance must not be closed to readers.
        let config = Config::development_defaults();
        assert_eq!(config.instance.mode, InstanceMode::Public);
        assert!(
            config
                .instance
                .mode
                .access_policy()
                .anonymous_reading_enabled
        );
    }

    #[test]
    fn a_walled_garden_closes_anonymous_reading_and_nothing_else() {
        // The rating ceilings are the same as the default: a sign-in wall is a
        // wall in front of the same library, not a different library.
        let public = InstanceMode::Public.access_policy();
        let walled = InstanceMode::WalledGarden.access_policy();
        assert!(public.anonymous_reading_enabled);
        assert!(!walled.anonymous_reading_enabled);
        assert_eq!(walled.anonymous_max_rating, public.anonymous_max_rating);
        assert_eq!(walled.adult_max_rating, public.adult_max_rating);
    }

    #[test]
    fn private_mode_is_at_least_as_closed_as_a_walled_garden() {
        assert!(
            !InstanceMode::Private
                .access_policy()
                .anonymous_reading_enabled
        );
    }

    #[test]
    fn instance_modes_round_trip_through_their_config_spelling() {
        for mode in [
            InstanceMode::Public,
            InstanceMode::WalledGarden,
            InstanceMode::Private,
        ] {
            let parsed = InstanceMode::parse(mode.as_str()).expect("parses");
            assert_eq!(parsed, mode);
        }
    }

    #[test]
    fn an_unrecognised_instance_mode_is_a_startup_error() {
        // A typo that fell back to `public` would open an instance the operator
        // believed closed, so this must refuse rather than default.
        let error = InstanceMode::parse("wall_garden").expect_err("must refuse");
        assert!(
            error.to_string().contains("walled_garden"),
            "the message must name the accepted values: {error}"
        );
    }

    #[test]
    fn the_instance_mode_loads_from_the_configuration_file() {
        let config = load_from(
            "walled",
            r#"
[instance]
mode = "walled_garden"
"#,
        )
        .expect("loads");
        assert_eq!(config.instance.mode, InstanceMode::WalledGarden);
        assert!(
            !config
                .instance
                .mode
                .access_policy()
                .anonymous_reading_enabled
        );
    }

    #[test]
    fn a_misspelled_mode_in_the_configuration_file_refuses_to_load() {
        let error = load_from(
            "wall-typo",
            r#"
[instance]
mode = "wall_garden"
"#,
        )
        .expect_err("must refuse");
        assert!(
            error.to_string().contains("walled_garden"),
            "the message must name the accepted values: {error}"
        );
    }

    #[test]
    fn the_preset_survives_being_read_alongside_the_mode() {
        // The two instance settings are independent: adding a mode must not
        // cost an operator the preset they already declared.
        let config = load_from(
            "both",
            r#"
[instance]
preset = "genre_haven"
mode = "private"
"#,
        )
        .expect("loads");
        assert_eq!(config.instance.preset, "genre_haven");
        assert_eq!(config.instance.mode, InstanceMode::Private);
    }

    // -----------------------------------------------------------------------
    // Amendment `calibrated-decision-models.md` — [decisions]
    // -----------------------------------------------------------------------

    /// A `NaN` threshold is refused, not clamped and not accepted.
    ///
    /// This is the reason `check_thresholds` exists and the reason its
    /// comparison is `!(a <= b)`. Every comparison against `NaN` is false, so a
    /// validation written as `consult_floor < accept_threshold` is TRUE for a
    /// pair of NaNs and quietly accepts them. The instance then starts, every
    /// test passes, and `reconcile` holds every acceptance forever because
    /// `p < NaN` is false even for `p = 1.0` — the direction inverts with no
    /// visible symptom anywhere.
    #[test]
    fn a_nan_threshold_is_refused_rather_than_silently_accepted() {
        for (field, value) in [("accept_threshold", f64::NAN), ("consult_floor", f64::NAN)] {
            let config = DecisionsConfig {
                accept_threshold: if field == "accept_threshold" {
                    value
                } else {
                    0.9
                },
                consult_floor: if field == "consult_floor" { value } else { 0.1 },
                ..DecisionsConfig::default()
            };
            assert!(
                config.check_thresholds().is_err(),
                "a NaN {field} must stop the instance from starting"
            );
        }
    }

    /// An infinite threshold is refused for the same reason and by the same
    /// comparison — `INFINITY <= x` is false for every finite `x`.
    #[test]
    fn an_infinite_threshold_is_refused() {
        let config = DecisionsConfig {
            accept_threshold: f64::INFINITY,
            ..DecisionsConfig::default()
        };
        assert!(config.check_thresholds().is_err());
    }

    /// A threshold outside `0.0..=1.0` is refused: `1.5` is not a probability,
    /// and clamping it to `1.0` would make the model maximally cautious
    /// without anybody choosing that.
    #[test]
    fn a_threshold_outside_the_unit_interval_is_refused() {
        for threshold in [-0.1, 1.5, 2.0] {
            let config = DecisionsConfig {
                accept_threshold: threshold,
                ..DecisionsConfig::default()
            };
            assert!(
                config.check_thresholds().is_err(),
                "{threshold} is not a probability"
            );
        }
    }

    /// A floor above the threshold is refused, because such a configuration
    /// asks about nothing — and does so silently, which is worse.
    #[test]
    fn a_floor_above_the_threshold_is_refused() {
        let config = DecisionsConfig {
            accept_threshold: 0.5,
            consult_floor: 0.6,
            ..DecisionsConfig::default()
        };
        let error = config
            .check_thresholds()
            .expect_err("a floor above the ceiling asks about nothing");
        assert!(error.contains("must not exceed"), "{error}");
    }

    /// The shipped example's `[decisions]` section parses to the values it
    /// documents.
    ///
    /// The shipped-example test above proves the *file* parses; this proves the
    /// section means what its comments say. An example that parses into
    /// defaults is an operator who copied it, set `provider = "calibrated"`,
    /// and got a deterministic instance with a threshold they never chose.
    #[test]
    fn the_shipped_example_decisions_section_means_what_it_says() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lorehaven.toml.example");
        let body = std::fs::read_to_string(&path).expect("the example ships");
        let file: toml::Value = toml::from_str(&body).expect("the example parses");
        let section = file
            .get("decisions")
            .expect("the example documents [decisions]");

        // The real parse, on the real file. Asserting against a hand-built
        // `DecisionsConfig` instead would prove the example is well-formed TOML
        // and say nothing about what the instance would do with it.
        let loaded = load_from("decisions-example", &body)
            .expect("the example loads")
            .decisions;
        assert_eq!(loaded.provider, DecisionProvider::Deterministic);
        assert_eq!(
            loaded.provider.as_str(),
            section["provider"].as_str().unwrap()
        );
        assert_eq!(loaded.base_url, section["base_url"].as_str().unwrap());
        assert_eq!(loaded.model, section["model"].as_str().unwrap());
        assert_eq!(
            loaded.accept_threshold,
            section["accept_threshold"].as_float().unwrap()
        );
        assert_eq!(
            loaded.consult_floor,
            section["consult_floor"].as_float().unwrap()
        );
        // The key is not in the file, so the example must not read one.
        assert!(
            section.get("api_key").is_none(),
            "the example must not ship a key field; the environment carries it"
        );
    }

    /// The defaults are the defaults, and they are the safe ones.
    #[test]
    fn an_instance_that_configured_nothing_is_deterministic() {
        let config = DecisionsConfig::default();
        assert_eq!(
            config.provider,
            DecisionProvider::Deterministic,
            "an instance that has not opted in must behave exactly as it did \
             before the feature existed"
        );
        assert_eq!(config.provider.as_str(), "deterministic");
        config
            .check_thresholds()
            .expect("the shipped defaults are valid");
    }

    /// The consult floor brackets the useful range, and the two ends are the
    /// ones worth pinning: a posterior the deterministic path already knows,
    /// and one the model's opinion cannot change.
    #[test]
    fn a_posterior_the_deterministic_path_already_knows_is_not_worth_a_request() {
        let config = DecisionsConfig::default();
        // Below the floor: the model would only confirm what a rejection or a
        // hold already said.
        assert!(!config.worth_asking_about(0.05));
        // Above the threshold: `reconcile` would keep the acceptance unchanged,
        // so the request buys nothing.
        assert!(!config.worth_asking_about(0.95));
        assert!(!config.worth_asking_about(1.0));
        // In between: this is the only range where the model's answer changes
        // the outcome, and it is the only range worth paying for.
        assert!(config.worth_asking_about(0.5));
    }

    /// A `NaN` posterior is never worth asking about, because it fails every
    /// comparison in the bracket above and would otherwise be waved through by
    /// a test that only checks the range.
    #[test]
    fn a_nan_posterior_is_never_worth_a_request() {
        let config = DecisionsConfig::default();
        assert!(!config.worth_asking_about(f64::NAN));
    }
}
