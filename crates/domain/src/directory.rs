//! Resource Directory domain (spec §39) — pure rules, no I/O.
//!
//! The directory is a set of curated lists whose entries are ranked by
//! community votes. Every ranking input is a named community vote, never
//! traffic, never money, and never the administrator's private taste
//! disclosed (§0.3): weights are applied, never shown.

use std::net::IpAddr;

/// Seed categories (spec §39.2). Operators extend this list through
/// `[directory].extra_categories`.
pub const SEED_CATEGORIES: [&str; 8] = [
    "fanfiction_archive",
    "discord_server",
    "author_platform",
    "writing_tool",
    "community",
    "podcast_newsletter",
    "lorehaven_instance",
    "other",
];

/// How votes are weighted (spec §39.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoteWeighting {
    /// Every vote weighs 1 — the weightless directory. Always available.
    Flat,
    /// Trust multiplier only.
    Trust,
    /// Trust and taste multipliers (the default).
    TrustAndTaste,
}

impl VoteWeighting {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "flat" => Some(Self::Flat),
            "trust" => Some(Self::Trust),
            "trust_and_taste" => Some(Self::TrustAndTaste),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Trust => "trust",
            Self::TrustAndTaste => "trust_and_taste",
        }
    }
}

/// Default trust multipliers per rung of the §19.1 ladder (TL0..TL6).
pub const DEFAULT_TRUST_VOTE_WEIGHTS: [f64; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

/// Default floor/ceiling the taste affinity (0..=1) maps onto.
pub const DEFAULT_TASTE_FLOOR: f64 = 0.75;
pub const DEFAULT_TASTE_CEILING: f64 = 1.25;

/// A validation error carrying a named reason (spec §39.3: refusals name
/// the reason).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryError {
    pub reason: String,
}

impl DirectoryError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// Validate an external entry URL (spec §39.3): absolute http(s) only, and
/// the host must not be a loopback or private address (the §14.9 SSRF
/// posture). Refusals carry a named reason.
pub fn validate_url(raw: &str) -> Result<(), DirectoryError> {
    let url = url::Url::parse(raw).map_err(|_| DirectoryError::new("url_not_absolute"))?;
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(DirectoryError::new(format!("url_scheme_not_http: {other}"))),
    }
    // A userinfo component (`http://user@host/`) is a classic SSRF trick.
    if !url.username().is_empty() || url.password().is_some() {
        return Err(DirectoryError::new("url_userinfo_refused"));
    }
    match url.host() {
        Some(url::Host::Domain(domain)) => validate_host(domain),
        Some(url::Host::Ipv4(ip)) => {
            if is_private_or_loopback(IpAddr::V4(ip)) {
                Err(DirectoryError::new("url_host_is_private_address"))
            } else {
                Ok(())
            }
        }
        Some(url::Host::Ipv6(ip)) => {
            if is_private_or_loopback(IpAddr::V6(ip)) {
                Err(DirectoryError::new("url_host_is_private_address"))
            } else {
                Ok(())
            }
        }
        None => Err(DirectoryError::new("url_has_no_host")),
    }
}

/// Host part of [`validate_url`], split out so hosts parsed elsewhere can
/// be checked with the same rules.
pub fn validate_host(host: &str) -> Result<(), DirectoryError> {
    let lowered = host.to_ascii_lowercase();
    if lowered == "localhost" || lowered.ends_with(".localhost") {
        return Err(DirectoryError::new("url_host_is_localhost"));
    }
    if let Ok(ip) = lowered.parse::<IpAddr>() {
        if is_private_or_loopback(ip) {
            return Err(DirectoryError::new("url_host_is_private_address"));
        }
        return Ok(());
    }
    Ok(())
}

fn is_private_or_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.octets()[0] == 169 && v4.octets()[1] == 254
        }
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unspecified() || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

/// Entry titles: 1–120 chars after trim.
pub fn validate_title(raw: &str) -> Result<(), DirectoryError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(DirectoryError::new("title_empty"));
    }
    if trimmed.chars().count() > 120 {
        return Err(DirectoryError::new("title_too_long"));
    }
    Ok(())
}

/// Descriptions: ≤ 500 chars.
pub fn validate_description(raw: &str) -> Result<(), DirectoryError> {
    if raw.chars().count() > 500 {
        return Err(DirectoryError::new("description_too_long"));
    }
    Ok(())
}

/// One tag: lowercase, trimmed, whitespace collapsed; `None` when empty or
/// over 32 chars.
pub fn normalize_tag(raw: &str) -> Option<String> {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() || collapsed.chars().count() > 32 {
        return None;
    }
    Some(collapsed.to_lowercase())
}

/// A tag list: normalize, drop invalid, dedupe, sort.
pub fn normalize_tags(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = raw.iter().filter_map(|t| normalize_tag(t)).collect();
    out.sort();
    out.dedup();
    out
}

/// The trust multiplier for a vote (spec §39.4). `trust_level` is the
/// §19.1 rung 0..=6; `weights` is the operator's retuned ladder (seven
/// multipliers, defaults in [`DEFAULT_TRUST_VOTE_WEIGHTS`]).
pub fn trust_multiplier(trust_level: u8, weights: &[f64; 7]) -> f64 {
    let idx = (trust_level as usize).min(6);
    weights[idx]
}

/// The taste multiplier for a vote (spec §39.4). `affinity` is the voter's
/// affinity to the administrator's taste profile, 0..=1 (the §9.7.4
/// signal); 0 maps to the floor, 1 to the ceiling.
pub fn taste_multiplier(affinity: f64, floor: f64, ceiling: f64) -> f64 {
    let clamped = affinity.clamp(0.0, 1.0);
    floor + (ceiling - floor) * clamped
}

/// The weight a vote carries under the configured weighting. The taste
/// component is only read in `trust_and_taste` mode, and is never surfaced.
pub fn vote_weight(
    weighting: VoteWeighting,
    trust_level: u8,
    trust_weights: &[f64; 7],
    taste_affinity: f64,
    taste_floor: f64,
    taste_ceiling: f64,
) -> f64 {
    match weighting {
        VoteWeighting::Flat => 1.0,
        VoteWeighting::Trust => trust_multiplier(trust_level, trust_weights),
        VoteWeighting::TrustAndTaste => {
            trust_multiplier(trust_level, trust_weights)
                * taste_multiplier(taste_affinity, taste_floor, taste_ceiling)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_must_be_absolute_http() {
        assert!(validate_url("https://example.com").is_ok());
        assert!(validate_url("http://example.com/path?q=1").is_ok());
        assert_eq!(
            validate_url("ftp://example.com").unwrap_err().reason,
            "url_scheme_not_http: ftp"
        );
        assert_eq!(
            validate_url("not a url").unwrap_err().reason,
            "url_not_absolute"
        );
    }

    #[test]
    fn loopback_and_private_hosts_are_refused() {
        assert_eq!(
            validate_url("http://127.0.0.1/x").unwrap_err().reason,
            "url_host_is_private_address"
        );
        assert_eq!(
            validate_url("http://localhost/x").unwrap_err().reason,
            "url_host_is_localhost"
        );
        assert_eq!(
            validate_url("http://app.localhost/x").unwrap_err().reason,
            "url_host_is_localhost"
        );
        assert_eq!(
            validate_url("http://192.168.1.1/").unwrap_err().reason,
            "url_host_is_private_address"
        );
        assert_eq!(
            validate_url("http://10.0.0.1/").unwrap_err().reason,
            "url_host_is_private_address"
        );
        assert_eq!(
            validate_url("http://172.16.0.1/").unwrap_err().reason,
            "url_host_is_private_address"
        );
        assert_eq!(
            validate_url("http://[::1]/").unwrap_err().reason,
            "url_host_is_private_address"
        );
    }

    #[test]
    fn userinfo_tricks_are_refused() {
        assert_eq!(
            validate_url("http://user@127.0.0.1/").unwrap_err().reason,
            "url_userinfo_refused"
        );
        assert_eq!(
            validate_url("http://user:pass@example.com/")
                .unwrap_err()
                .reason,
            "url_userinfo_refused"
        );
    }

    #[test]
    fn public_hosts_pass() {
        assert!(validate_url("https://archiveofourown.org").is_ok());
        assert!(validate_url("https://8.8.8.8/dns").is_ok());
    }

    #[test]
    fn titles_are_1_to_120_chars() {
        assert!(validate_title("A Discord server").is_ok());
        assert_eq!(validate_title("   ").unwrap_err().reason, "title_empty");
        assert_eq!(
            validate_title(&"x".repeat(121)).unwrap_err().reason,
            "title_too_long"
        );
        assert!(validate_title(&"x".repeat(120)).is_ok());
    }

    #[test]
    fn descriptions_cap_at_500_chars() {
        assert!(validate_description("").is_ok());
        assert!(validate_description(&"y".repeat(500)).is_ok());
        assert_eq!(
            validate_description(&"y".repeat(501)).unwrap_err().reason,
            "description_too_long"
        );
    }

    #[test]
    fn tags_normalize_dedupe_and_sort() {
        assert_eq!(normalize_tag("  Fan Fiction  "), Some("fan fiction".into()));
        assert_eq!(normalize_tag(""), None);
        assert_eq!(normalize_tag(&"t".repeat(33)), None);
        assert_eq!(
            normalize_tags(&["B".into(), " a ".into(), "b".into(), "".into()]),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn trust_multiplier_follows_the_ladder() {
        let w = &DEFAULT_TRUST_VOTE_WEIGHTS;
        assert_eq!(trust_multiplier(0, w), 0.5);
        assert_eq!(trust_multiplier(4, w), 1.5);
        assert_eq!(trust_multiplier(6, w), 2.0);
        // Out-of-range rungs clamp rather than panic.
        assert_eq!(trust_multiplier(9, w), 2.0);
    }

    #[test]
    fn taste_multiplier_maps_affinity_onto_floor_ceiling() {
        assert_eq!(taste_multiplier(0.0, 0.75, 1.25), 0.75);
        assert_eq!(taste_multiplier(1.0, 0.75, 1.25), 1.25);
        assert_eq!(taste_multiplier(0.5, 0.75, 1.25), 1.0);
        // Out-of-range affinity clamps.
        assert_eq!(taste_multiplier(2.0, 0.75, 1.25), 1.25);
    }

    #[test]
    fn vote_weight_respects_the_mode() {
        let tw = &DEFAULT_TRUST_VOTE_WEIGHTS;
        // Flat: everything weighs 1.
        assert_eq!(
            vote_weight(VoteWeighting::Flat, 0, tw, 0.0, 0.75, 1.25),
            1.0
        );
        assert_eq!(
            vote_weight(VoteWeighting::Flat, 6, tw, 1.0, 0.75, 1.25),
            1.0
        );
        // Trust only: taste is ignored.
        assert_eq!(
            vote_weight(VoteWeighting::Trust, 4, tw, 1.0, 0.75, 1.25),
            1.5
        );
        // Trust and taste (default): both multiply.
        let w = vote_weight(VoteWeighting::TrustAndTaste, 4, tw, 0.5, 0.75, 1.25);
        assert!((w - 1.5).abs() < 1e-9);
        let w0 = vote_weight(VoteWeighting::TrustAndTaste, 0, tw, 0.0, 0.75, 1.25);
        assert!((w0 - 0.375).abs() < 1e-9);
    }

    #[test]
    fn a_tl4_vote_moves_the_score_more_than_a_tl0_vote() {
        let tw = &DEFAULT_TRUST_VOTE_WEIGHTS;
        let tl4 = vote_weight(VoteWeighting::TrustAndTaste, 4, tw, 0.5, 0.75, 1.25);
        let tl0 = vote_weight(VoteWeighting::TrustAndTaste, 0, tw, 0.5, 0.75, 1.25);
        assert!(tl4 > tl0);
    }

    #[test]
    fn weighting_parses() {
        assert_eq!(VoteWeighting::parse("flat"), Some(VoteWeighting::Flat));
        assert_eq!(
            VoteWeighting::parse("trust_and_taste"),
            Some(VoteWeighting::TrustAndTaste)
        );
        assert_eq!(VoteWeighting::parse("other"), None);
    }
}
