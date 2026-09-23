//! M22-M39 extension: media resilience & availability guarantee (spec §32.7).
//!
//! Vocabulary types for the media reference graph: link providers, link
//! statuses, media contexts, and curator reward kinds. The vocabulary here
//! is the contract the repository and route layers read and write; a value
//! outside these sets must be refused at the edge, not stored and ignored.

use std::fmt;

macro_rules! vocabulary {
    ($name:ident, $as_str:ident, { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($variant,)+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant,)+];

            pub fn $as_str(&self) -> &'static str {
                match self {
                    $($name::$variant => $text,)+
                }
            }
        }

        impl std::str::FromStr for $name {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($text => Ok($name::$variant),)+
                    _ => Err(format!("unknown {}: {value}", stringify!($name))),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.$as_str())
            }
        }
    };
}

vocabulary!(LinkProvider, as_str, {
    Pinterest => "pinterest",
    Tumblr => "tumblr",
    Imgur => "imgur",
    InternetArchive => "archive.org",
    LocalMirror => "local_mirror",
    Ipfs => "ipfs",
    Twitter => "twitter",
    Spotify => "spotify",
    Instagram => "instagram",
    Other => "other",
});

vocabulary!(LinkStatus, as_str, {
    Healthy => "healthy",
    Degraded => "degraded",
    Dead => "dead",
    Quarantined => "quarantined",
    PendingVerification => "pending_verification",
});

vocabulary!(MediaKind, as_str, {
    Image => "image",
    Audio => "audio",
    Video => "video",
    Document => "document",
    Embed => "embed",
});

vocabulary!(MediaContextKind, as_str, {
    Faceclaim => "faceclaim",
    Moodboard => "moodboard",
    Playlist => "playlist",
    Fanart => "fanart",
    Reference => "reference",
    InlineEmbed => "inline_embed",
});

vocabulary!(CuratorAction, as_str, {
    MirrorAdd => "mirror_add",
    ArchiveAdd => "archive_add",
    Verify => "verify",
    ConfirmBroken => "confirm_broken",
    Rescue => "rescue",
    ContentNote => "content_note",
    Merge => "merge",
});

/// Validate that a priority value is in the valid range.
pub fn is_valid_priority(priority: i64) -> bool {
    (0..=10000).contains(&priority)
}

/// Perceptual hash match threshold: a Hamming distance at or below this value
/// means "same image" (configurable per instance, default 6).
pub fn perceptual_match_threshold_is_valid(threshold: i64) -> bool {
    (1..=32).contains(&threshold)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_provider_round_trips() {
        for provider in LinkProvider::ALL {
            let s = provider.as_str();
            let parsed: LinkProvider = s.parse().unwrap();
            assert_eq!(*provider, parsed);
            assert_eq!(s, parsed.as_str());
        }
    }

    #[test]
    fn link_status_round_trips() {
        for status in LinkStatus::ALL {
            let s = status.as_str();
            let parsed: LinkStatus = s.parse().unwrap();
            assert_eq!(*status, parsed);
        }
    }

    #[test]
    fn media_kind_round_trips() {
        for kind in MediaKind::ALL {
            let s = kind.as_str();
            let parsed: MediaKind = s.parse().unwrap();
            assert_eq!(*kind, parsed);
        }
    }

    #[test]
    fn curator_action_round_trips() {
        for action in CuratorAction::ALL {
            let s = action.as_str();
            let parsed: CuratorAction = s.parse().unwrap();
            assert_eq!(*action, parsed);
        }
    }

    #[test]
    fn invalid_link_provider_rejected() {
        assert!("not_a_provider".parse::<LinkProvider>().is_err());
    }

    #[test]
    fn invalid_link_status_rejected() {
        assert!("alive".parse::<LinkStatus>().is_err());
    }

    #[test]
    fn priority_validation() {
        assert!(is_valid_priority(0));
        assert!(is_valid_priority(10000));
        assert!(!is_valid_priority(-1));
        assert!(!is_valid_priority(10001));
    }

    #[test]
    fn threshold_validation() {
        assert!(perceptual_match_threshold_is_valid(1));
        assert!(perceptual_match_threshold_is_valid(32));
        assert!(!perceptual_match_threshold_is_valid(0));
        assert!(!perceptual_match_threshold_is_valid(33));
    }

    #[test]
    fn link_status_as_str() {
        assert_eq!(LinkStatus::Healthy.as_str(), "healthy");
        assert_eq!(LinkStatus::Dead.as_str(), "dead");
        assert_eq!(
            LinkStatus::PendingVerification.as_str(),
            "pending_verification"
        );
    }

    #[test]
    fn curator_action_display() {
        use std::fmt::Write;
        let mut s = String::new();
        write!(s, "{}", CuratorAction::Rescue).unwrap();
        assert_eq!(s, "rescue");
    }
}

// ---------------------------------------------------------------------------
// Phase 2 (§32.7.5): Curator verification types
// ---------------------------------------------------------------------------

vocabulary!(VerificationType, as_str, {
    ExactMatch => "exact_match",
    PerceptualMatch => "perceptual_match",
    Reverify => "reverify",
});
