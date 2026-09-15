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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    StorageRead,
    StorageWrite,
    WorkRead,
    WebhookSend,
    JobExecute,
}

impl Capability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::StorageRead => "storage.read",
            Self::StorageWrite => "storage.write",
            Self::WorkRead => "work.read",
            Self::WebhookSend => "webhook.send",
            Self::JobExecute => "job.execute",
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

/// Check that a grant's capabilities are a subset of the manifest's requested capabilities.
pub fn grant_is_subset(manifest_caps: &[Capability], grant_caps: &[Capability]) -> bool {
    grant_caps.iter().all(|c| manifest_caps.contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;
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
        ] {
            assert_eq!(Capability::from_str(c.as_str()).unwrap(), c);
        }
    }

    #[test]
    fn unknown_capability_rejected() {
        assert!(Capability::from_str("ambient.network").is_err());
        assert!(Capability::from_str("process.control").is_err());
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
