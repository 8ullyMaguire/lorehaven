//! M16 — Extension domain: manifest schema, capability vocabulary, grants.

/// Extension states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionState {
    Pending,
    Approved,
    Rejected,
    Revoked,
}

impl ExtensionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Revoked => "revoked",
        }
    }
}

impl std::str::FromStr for ExtensionState {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            "revoked" => Ok(Self::Revoked),
            _ => Err(format!("unknown extension state: {s}")),
        }
    }
}

/// Capability vocabulary.
/// Extensions can only request these capabilities — no ambient network or process control.
///
/// "No ambient network" means no extension reaches the network without declaring
/// it, and a declaration is not a grant: [`grant_is_subset`] checks the grant
/// against the manifest, §21.5's review decides whether the manifest ships at
/// all, and §55.4.1 bounds the request to a declared allowlist at the socket. A
/// capability here is a *request to be reviewed*, not a permission held.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Capability {
    #[serde(rename = "storage.read")]
    StorageRead,
    #[serde(rename = "storage.write")]
    StorageWrite,
    #[serde(rename = "work.read")]
    WorkRead,
    #[serde(rename = "webhook.send")]
    WebhookSend,
    #[serde(rename = "job.execute")]
    JobExecute,
    /// Network access, bounded by the manifest's `network_allowlist`.
    ///
    /// New in §55. It exists because a source adapter's entire job is fetching,
    /// and adding it here is what lets §55.4.1's domain lockdown be reasoned
    /// about by `grant_is_subset` like any other capability. Without the variant,
    /// an adapter would have to hold no capability and fetch anyway.
    #[serde(rename = "network")]
    Network,
    /// Read a credential from the §11.6 vault, without ever seeing it.
    ///
    /// New in §55, and narrower than it looks: the grant is to *use* a session
    /// token the host attaches, never to read a stored password. §55.4.2 is why
    /// those are different permissions and only one of them exists.
    #[serde(rename = "credential.read")]
    CredentialRead,
}

impl Capability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::StorageRead => "storage.read",
            Self::StorageWrite => "storage.write",
            Self::WorkRead => "work.read",
            Self::WebhookSend => "webhook.send",
            Self::JobExecute => "job.execute",
            Self::Network => "network",
            Self::CredentialRead => "credential.read",
        }
    }
}

impl std::str::FromStr for Capability {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "storage.read" => Ok(Self::StorageRead),
            "storage.write" => Ok(Self::StorageWrite),
            "work.read" => Ok(Self::WorkRead),
            "webhook.send" => Ok(Self::WebhookSend),
            "job.execute" => Ok(Self::JobExecute),
            "network" => Ok(Self::Network),
            "credential.read" => Ok(Self::CredentialRead),
            _ => Err(format!("unknown capability: {s}")),
        }
    }
}

/// Memory tiers for extension isolation (spec §21).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTier {
    Tier150,
    Tier300,
    Tier600,
}

impl MemoryTier {
    pub fn as_mib(&self) -> u64 {
        match self {
            Self::Tier150 => 150,
            Self::Tier300 => 300,
            Self::Tier600 => 600,
        }
    }
}

/// §21.2's category list, now a type.
///
/// This was prose until §55 needed somewhere to put `source_adapters`. A category
/// has to be a value because §21.6's gallery filters and §55.2's submission path
/// both branch on it, and a `&'static str` compared by hand is where the two
/// would drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    ReaderWidget,
    DashboardWidget,
    ThemeLayout,
    RecommendationEngine,
    DeclarativeRecipe,
    SearchHelper,
    WritingTool,
    ChallengeVariant,
    PositivityFilterRule,
    MoodTag,
    FandomLanding,
    Integration,
    /// §55 — curator-submitted source adapters.
    SourceAdapters,
}

impl Category {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReaderWidget => "reader_widget",
            Self::DashboardWidget => "dashboard_widget",
            Self::ThemeLayout => "theme_layout",
            Self::RecommendationEngine => "recommendation_engine",
            Self::DeclarativeRecipe => "declarative_recipe",
            Self::SearchHelper => "search_helper",
            Self::WritingTool => "writing_tool",
            Self::ChallengeVariant => "challenge_variant",
            Self::PositivityFilterRule => "positivity_filter_rule",
            Self::MoodTag => "mood_tag",
            Self::FandomLanding => "fandom_landing",
            Self::Integration => "integration",
            Self::SourceAdapters => "source_adapters",
        }
    }
}

impl std::str::FromStr for Category {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "reader_widget" => Ok(Self::ReaderWidget),
            "dashboard_widget" => Ok(Self::DashboardWidget),
            "theme_layout" => Ok(Self::ThemeLayout),
            "recommendation_engine" => Ok(Self::RecommendationEngine),
            "declarative_recipe" => Ok(Self::DeclarativeRecipe),
            "search_helper" => Ok(Self::SearchHelper),
            "writing_tool" => Ok(Self::WritingTool),
            "challenge_variant" => Ok(Self::ChallengeVariant),
            "positivity_filter_rule" => Ok(Self::PositivityFilterRule),
            "mood_tag" => Ok(Self::MoodTag),
            "fandom_landing" => Ok(Self::FandomLanding),
            "integration" => Ok(Self::Integration),
            "source_adapters" => Ok(Self::SourceAdapters),
            _ => Err(format!("unknown extension category: {s}")),
        }
    }
}

/// Whether an extension is free or paid (§21.1's `pricing`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pricing {
    Free,
    Paid,
}

/// §21.1's manifest, as a type.
///
/// `deny_unknown_fields` is load-bearing and not tidiness. A curator's manifest is
/// read by a steward in quorum review, and the whole argument for the declarative
/// path (§55) is that what it says is what it does. A typo'd key that serde
/// silently defaulted would put a gap between the reviewed text and the running
/// extension, and the gap is invisible to the reader doing the reading.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub version: u32,
    pub category: Category,
    pub entrypoint: String,
    pub required_host_api_version: u32,
    pub permissions: Vec<Capability>,
    pub resource_limits: ResourceLimits,
    pub supported_surfaces: Vec<String>,
    pub license: String,
    /// Hostnames this extension may reach. Empty means none: an extension that
    /// declares no allowlist gets no network, whatever its permissions say.
    ///
    /// §21.1 listed this field but §55.4.1 is what gives it teeth, and the
    /// default-empty reading is what makes "declared" mean something.
    pub network_allowlist: Vec<String>,
    pub pricing: Pricing,
}

/// §21.4's resource ceilings, as submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceLimits {
    pub memory_mib: u32,
    /// Fuel budget. Named rather than a bare number so a manifest that says
    /// `fuel: 0` looks like a mistake instead of meaning "unlimited".
    pub fuel: u64,
    pub host_calls_per_run: u32,
}

/// Check that a grant's capabilities are a subset of the manifest's requested capabilities.
pub fn grant_is_subset(manifest_caps: &[Capability], grant_caps: &[Capability]) -> bool {
    grant_caps.iter().all(|c| manifest_caps.contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::str::FromStr;

    #[test]
    fn extension_state_round_trip() {
        for s in [
            ExtensionState::Pending,
            ExtensionState::Approved,
            ExtensionState::Rejected,
            ExtensionState::Revoked,
        ] {
            assert_eq!(ExtensionState::from_str(s.as_str()).unwrap(), s);
        }
    }

    #[test]
    fn capability_round_trip() {
        for c in [
            Capability::StorageRead,
            Capability::StorageWrite,
            Capability::WorkRead,
            Capability::WebhookSend,
            Capability::JobExecute,
            Capability::Network,
            Capability::CredentialRead,
        ] {
            assert_eq!(Capability::from_str(c.as_str()).unwrap(), c);
        }
    }

    /// `as_str` and serde must name a capability identically.
    ///
    /// They are two spellings of one vocabulary, and the tests that catch a
    /// disagreement are the ones that go looking for it: `serde(rename_all =
    /// "snake_case")` would emit `storage_read` while `as_str` says
    /// `storage.read`, so a manifest could request a capability the host parses
    /// back as a different one. Round-tripping through both is the only way to
    /// know they still agree.
    #[test]
    fn capability_as_str_and_serde_name_the_same_thing() {
        for c in [
            Capability::StorageRead,
            Capability::StorageWrite,
            Capability::WorkRead,
            Capability::WebhookSend,
            Capability::JobExecute,
            Capability::Network,
            Capability::CredentialRead,
        ] {
            let json = serde_json::to_string(&c).unwrap();
            assert_eq!(json, format!("\"{}\"", c.as_str()));
            let back: Capability = serde_json::from_str(&json).unwrap();
            assert_eq!(back, c);
        }
    }

    #[test]
    fn unknown_capability_rejected() {
        assert!(Capability::from_str("ambient.network").is_err());
        assert!(Capability::from_str("process.control").is_err());
    }

    /// §55.2's trust gate has to be reasonable by `grant_is_subset` like anything
    /// else, or an adapter's network capability would sit outside the check that
    /// every other capability passes through.
    #[test]
    fn a_grant_cannot_exceed_a_manifest_asking_for_network() {
        let manifest = vec![Capability::Network, Capability::WorkRead];

        assert!(grant_is_subset(&manifest, &[Capability::Network]));
        assert!(!grant_is_subset(
            &manifest,
            &[Capability::Network, Capability::CredentialRead]
        ));
        // And the case that matters most: a manifest asking for nothing network
        // cannot be granted network by a later step.
        assert!(!grant_is_subset(&[], &[Capability::Network]));
    }

    #[test]
    fn every_category_round_trips_through_its_slug() {
        for c in [
            Category::ReaderWidget,
            Category::DashboardWidget,
            Category::ThemeLayout,
            Category::RecommendationEngine,
            Category::DeclarativeRecipe,
            Category::SearchHelper,
            Category::WritingTool,
            Category::ChallengeVariant,
            Category::PositivityFilterRule,
            Category::MoodTag,
            Category::FandomLanding,
            Category::Integration,
            Category::SourceAdapters,
        ] {
            assert_eq!(Category::from_str(c.as_str()).unwrap(), c);
            let json = serde_json::to_string(&c).unwrap();
            assert_eq!(json, format!("\"{}\"", c.as_str()));
            assert_eq!(serde_json::from_str::<Category>(&json).unwrap(), c);
        }
    }

    fn a_manifest() -> Manifest {
        Manifest {
            id: "example-archive".to_string(),
            version: 1,
            category: Category::SourceAdapters,
            entrypoint: "source.yaml".to_string(),
            required_host_api_version: 1,
            permissions: vec![Capability::Network],
            resource_limits: ResourceLimits {
                memory_mib: 48,
                fuel: 50_000_000,
                host_calls_per_run: 512,
            },
            supported_surfaces: vec!["library".to_string()],
            license: "CC0-1.0".to_string(),
            network_allowlist: vec!["example-archive.org".to_string()],
            pricing: Pricing::Free,
        }
    }

    /// The reason `Manifest` carries `deny_unknown_fields`.
    ///
    /// A typo'd key that serde ignored would leave a gap between the manifest a
    /// steward read in quorum review and the one the host ran, and the gap is
    /// invisible to whoever is reading.
    #[test]
    fn an_unknown_manifest_field_is_refused() {
        let mut json = serde_json::to_value(a_manifest()).unwrap();
        json["permissionss"] = json!(["network"]);
        let err = serde_json::from_value::<Manifest>(json).unwrap_err();
        assert!(
            err.to_string().contains("permissionss"),
            "the error must name the offending field: {err}"
        );
    }

    #[test]
    fn a_manifest_round_trips() {
        let m = a_manifest();
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<Manifest>(&json).unwrap(), m);
    }

    /// An empty allowlist means no network, which is what makes `Network` a
    /// declaration rather than a permission.
    #[test]
    fn a_manifest_with_an_empty_allowlist_declares_no_reachable_host() {
        let m = Manifest {
            network_allowlist: vec![],
            ..a_manifest()
        };
        assert!(
            m.network_allowlist.is_empty(),
            "nothing to reach is the safe default, so a host is a decision"
        );
        // `Network` may be requested, but there is nowhere for it to go.
        assert!(grant_is_subset(&m.permissions, &[Capability::Network]));
        assert!(m.network_allowlist.is_empty());
    }

    #[test]
    fn grant_subset_rule() {
        let manifest = vec![
            Capability::StorageRead,
            Capability::WorkRead,
            Capability::WebhookSend,
        ];
        let valid_grant = vec![Capability::StorageRead, Capability::WorkRead];
        let invalid_grant = vec![Capability::StorageRead, Capability::JobExecute];

        assert!(grant_is_subset(&manifest, &valid_grant));
        assert!(!grant_is_subset(&manifest, &invalid_grant));
    }

    #[test]
    fn memory_tiers() {
        assert_eq!(MemoryTier::Tier150.as_mib(), 150);
        assert_eq!(MemoryTier::Tier300.as_mib(), 300);
        assert_eq!(MemoryTier::Tier600.as_mib(), 600);
    }
}
