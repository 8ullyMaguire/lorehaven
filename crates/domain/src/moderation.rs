/// Moderation ladder and community health (spec §35.5).
use std::fmt;

/// Graduated response ladder (spec §35.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SanctionLevel {
    /// A private note from a moderator. No functional effect, but logged.
    VerbalWarning,
    /// Posting rate limited to 1 per hour in the affected scope.
    PostThrottle,
    /// Can read but not post. Expires automatically.
    ReadOnly,
    /// Cannot access the forum at all.
    ForumBan,
    /// Cannot access the instance at all.
    SiteBan,
}

impl SanctionLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::VerbalWarning => "verbal_warning",
            Self::PostThrottle => "post_throttle",
            Self::ReadOnly => "read_only",
            Self::ForumBan => "forum_ban",
            Self::SiteBan => "site_ban",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "verbal_warning" => Some(Self::VerbalWarning),
            "post_throttle" => Some(Self::PostThrottle),
            "read_only" => Some(Self::ReadOnly),
            "forum_ban" => Some(Self::ForumBan),
            "site_ban" => Some(Self::SiteBan),
            _ => None,
        }
    }

    /// True if the user may post at all under this sanction.
    pub fn blocks_posting(&self) -> bool {
        matches!(
            self,
            Self::ReadOnly | Self::ForumBan | Self::SiteBan
        )
    }

    /// True if the user may read the forum at all.
    pub fn blocks_reading_fine(&self) -> bool {
        matches!(self, Self::ForumBan | Self::SiteBan)
    }

    /// True if this is a site-wide ban.
    pub fn is_site_wide(&self) -> bool {
        matches!(self, Self::SiteBan)
    }
}

impl fmt::Display for SanctionLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Federation scope for a topic (spec §35.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederationScope {
    /// Visible everywhere, federated to all instances.
    Public,
    /// Never leaves this instance.
    Local,
    /// Visible publicly but not federated.
    Unlisted,
}

impl FederationScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Local => "local",
            Self::Unlisted => "unlisted",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "public" => Some(Self::Public),
            "local" => Some(Self::Local),
            "unlisted" => Some(Self::Unlisted),
            _ => None,
        }
    }

    /// True if the topic should be federated outbound.
    pub fn is_federated(&self) -> bool {
        matches!(self, Self::Public)
    }
}

/// Whether a user is currently sanctioned in a scope.
pub struct SanctionCheck {
    /// The active sanction, if any.
    pub active: bool,
    /// The level of the active sanction.
    pub level: Option<SanctionLevel>,
    /// True if posting is blocked.
    pub blocks_posting: bool,
    /// True if reading is blocked.
    pub blocks_reading: bool,
    /// When the sanction expires (None = permanent).
    pub expires_at: Option<String>,
}

impl SanctionCheck {
    pub fn none() -> Self {
        Self {
            active: false,
            level: None,
            blocks_posting: false,
            blocks_reading: false,
            expires_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanction_level_round_trip() {
        for level in [
            SanctionLevel::VerbalWarning,
            SanctionLevel::PostThrottle,
            SanctionLevel::ReadOnly,
            SanctionLevel::ForumBan,
            SanctionLevel::SiteBan,
        ] {
            let s = level.as_str();
            let parsed = SanctionLevel::from_str(s).unwrap();
            assert_eq!(parsed, level);
        }
    }

    #[test]
    fn test_sanction_blocks() {
        assert!(!SanctionLevel::VerbalWarning.blocks_posting());
        assert!(!SanctionLevel::PostThrottle.blocks_posting());
        assert!(SanctionLevel::ReadOnly.blocks_posting());
        assert!(SanctionLevel::ForumBan.blocks_posting());
        assert!(SanctionLevel::SiteBan.blocks_posting());

        assert!(!SanctionLevel::ForumBan.blocks_reading_fine());
        assert!(SanctionLevel::SiteBan.blocks_reading_fine());
    }

    #[test]
    fn test_federation_scope() {
        assert!(FederationScope::Public.is_federated());
        assert!(!FederationScope::Local.is_federated());
        assert!(!FederationScope::Unlisted.is_federated());
    }
}
