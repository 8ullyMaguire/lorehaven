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

// The perceptual hash algorithm an instance computes for images (spec §32.7.2).
//
// A plain comment, not a doc comment: rustdoc does not generate documentation for
// a macro invocation, so `///` here is dropped and warns as unused.
//
// All four produce 64-bit fingerprints, which is why a Hamming distance between
// two of them is meaningful at all. They differ in what they tolerate: pHash is
// the most robust to re-encoding and mild edits, dHash to structural shifts,
// wHash to scaling, aHash to the mean pixel value.
vocabulary!(PerceptualHashAlgorithm, as_str, {
    Phash => "phash",
    Dhash => "dhash",
    Whash => "whash",
    Ahash => "ahash",
});

// The audio fingerprinting scheme an instance uses (spec §32.7.2). A plain
// comment for the same reason as PerceptualHashAlgorithm above.
vocabulary!(AudioFingerprint, as_str, {
    Chromaprint => "chromaprint",
    Acoustid => "acoustid",
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

/// Whether a Hamming distance falls at or below the instance's configured
/// perceptual match threshold (spec §32.7.2). The comparison is inclusive: a
/// threshold of 6 means "distance 6 counts as the same image", which is how the
/// spec states it ("Hamming distance <= 6 = same image").
pub fn is_within_perceptual_threshold(distance: u32, threshold: u32) -> bool {
    distance <= threshold
}

/// The widest perceptual-hash width this module scores against, in bits.
///
/// pHash, dHash, wHash and aHash all produce 64-bit fingerprints, so 64 is the
/// full span of a distance between two of them. Beyond that width two hashes
/// are not the same kind of object and a confidence score would be invented.
const PERCEPTUAL_HASH_MAX_BITS: u32 = 64;

/// A curator-facing confidence score in `0.0..=1.0` for a perceptual match
/// (spec §32.7.2: "present the curator with a match confidence score").
///
/// The score is linear in the distance across the full width of a 64-bit hash,
/// so distance 0 is 1.0 and distance 64 is 0.0. A distance beyond that width
/// scores 0.0 rather than wrapping.
///
/// The score describes *hash similarity only*. It is not a probability that the
/// two images are the same work of art: pHash agrees across re-encodes, crops
/// and mild edits, and disagrees across distinct images that happen to share
/// structure. The curator confirms or rejects the linkage, which is why the
/// spec routes this through a human instead of auto-merging.
pub fn perceptual_match_confidence(distance: u32) -> f64 {
    let remaining = PERCEPTUAL_HASH_MAX_BITS.saturating_sub(distance);
    f64::from(remaining) / f64::from(PERCEPTUAL_HASH_MAX_BITS)
}

/// The Hamming distance between two hex-encoded perceptual hashes, or `None`
/// when no honest distance can be reported.
///
/// Perceptual hashes are stored as hex (spec §32.7.2: pHash/dHash/wHash produce
/// 64-bit fingerprints, conventionally written as 16 hex digits). `None` is
/// returned rather than a guess in every case where a number would be
/// misleading:
///
/// - a character that is not a hex digit, so the value is not a hash at all;
/// - an empty string, which is a missing hash rather than a hash of zero;
/// - differing digit counts, because comparing a 64-bit pHash against a 128-bit
///   wHash is not a comparison — a distance between them has no meaning.
///
/// Callers must treat `None` as "not comparable", never as "far apart". Folding
/// it into a large distance would invent a similarity verdict out of a
/// malformed value.
pub fn hamming_distance(a: &str, b: &str) -> Option<u32> {
    fn nibbles(hash: &str) -> Option<Vec<u8>> {
        let digits: Vec<u8> = hash
            .chars()
            .filter(|c| !matches!(c, ':' | '-' | ' '))
            .map(|c| c.to_digit(16).map(|d| d as u8))
            .collect::<Option<Vec<u8>>>()?;
        if digits.is_empty() {
            return None;
        }
        Some(digits)
    }

    let left = nibbles(a)?;
    let right = nibbles(b)?;
    if left.len() != right.len() {
        return None;
    }
    Some(
        left.iter()
            .zip(&right)
            .map(|(l, r)| (l ^ r).count_ones())
            .sum(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hamming_distance_of_identical_hex_hashes_is_zero() {
        assert_eq!(hamming_distance("00ff0a55", "00ff0a55"), Some(0));
    }

    #[test]
    fn hamming_distance_counts_differing_nibbles() {
        // 0x0 ^ 0xf = 4 bits, 0x0 ^ 0x0 = 0, 0xf ^ 0xa = 2 bits, 0xf ^ 0x5 = 2 bits.
        assert_eq!(hamming_distance("00ff", "f0a5"), Some(8));
    }

    #[test]
    fn hamming_distance_accepts_separators_and_case() {
        // "00:ff" and "00FF" are the same 16 bits, so the distance to "0000" is 8.
        assert_eq!(hamming_distance("00:ff", "0000"), Some(8));
    }

    #[test]
    fn hamming_distance_is_none_for_a_mismatched_length() {
        // Comparing a 64-bit pHash against a 128-bit wHash is not a comparison
        // at all: the bit widths differ, so there is no honest distance to report.
        assert_eq!(hamming_distance("00ff", "00ff00ff"), None);
    }

    #[test]
    fn hamming_distance_is_none_for_non_hex_characters() {
        assert_eq!(hamming_distance("00fg", "0000"), None);
        assert_eq!(hamming_distance("zzzz", "0000"), None);
    }

    #[test]
    fn hamming_distance_of_empty_hashes_is_none() {
        assert_eq!(hamming_distance("", "0000"), None);
    }

    #[test]
    fn confidence_is_one_for_an_identical_hash() {
        assert_eq!(perceptual_match_confidence(0), 1.0);
    }

    #[test]
    fn confidence_falls_as_the_distance_grows() {
        // A distance of 1 is the closest imperfect match; 64 (the width of a
        // 256-bit hash) is the furthest. The score must decrease monotonically
        // and stay inside 0..=1.
        let near = perceptual_match_confidence(1);
        let mid = perceptual_match_confidence(16);
        let far = perceptual_match_confidence(64);
        assert!(near > mid, "{near} should beat {mid}");
        assert!(mid > far, "{mid} should beat {far}");
        assert!((0.0..=1.0).contains(&far));
    }

    #[test]
    fn confidence_is_zero_past_a_full_width_of_difference() {
        assert_eq!(perceptual_match_confidence(65), 0.0);
        assert_eq!(perceptual_match_confidence(1000), 0.0);
    }

    #[test]
    fn a_distance_beyond_the_threshold_is_not_offered_as_a_match() {
        assert!(is_within_perceptual_threshold(3, 6));
        assert!(is_within_perceptual_threshold(6, 6));
        assert!(!is_within_perceptual_threshold(7, 6));
    }

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
    fn perceptual_hash_algorithm_round_trips() {
        for algorithm in PerceptualHashAlgorithm::ALL {
            let text = algorithm.as_str();
            let parsed: PerceptualHashAlgorithm = text.parse().unwrap();
            assert_eq!(*algorithm, parsed);
        }
        assert!("sha256".parse::<PerceptualHashAlgorithm>().is_err());
    }

    #[test]
    fn audio_fingerprint_round_trips() {
        for fingerprint in AudioFingerprint::ALL {
            let text = fingerprint.as_str();
            let parsed: AudioFingerprint = text.parse().unwrap();
            assert_eq!(*fingerprint, parsed);
        }
        assert!("audiodraft".parse::<AudioFingerprint>().is_err());
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
