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
//
// Only `Dhash` is implemented (see `media_resilience::difference_hash`), and it
// is the default for that reason. The other three are accepted by config and
// stored so an operator can record their intent, but a build that is asked to
// compute one says so rather than writing a hash computed by a different
// algorithm - a column of pHash-shaped strings produced by dHash would make the
// dedup search compare numbers that do not mean what the setting claims.
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

/// A 64-bit difference hash (dHash) of a grayscale image, as 16 hex digits, or
/// `None` when the image cannot produce one.
///
/// dHash is the one perceptual algorithm in this module that needs no image
/// decoder. It downsamples the frame to a 9x8 grid, compares each sampled pixel
/// with the one to its left, and keeps the answer as a bit - so it survives
/// re-encoding, rescaling and a uniform brightness shift, the differences a
/// curator does not care about, while still separating images that are actually
/// different. pHash needs a DCT and wHash a wavelet, so both want a decoded
/// pixel buffer; dHash wants only grayscale, which is the honest starting
/// point, and the spec lists it as one of the choices
/// (`perceptual_hash_algorithm = "dhash"`).
///
/// The input is row-major grayscale, one byte per pixel, which is what a decoder
/// produces and what a caller can synthesise in a test. `width` and `height` must
/// describe `pixels` exactly; a mismatch returns `None` rather than a partial
/// hash, because a fingerprint of the wrong pixels is a fingerprint of nothing.
///
/// The grid is a fixed 9x8 regardless of the input size, which is what makes
/// hashes of two different-sized copies of one image comparable: the bit count
/// is 64 either way, so a Hamming distance between them means something. Bit *i*
/// is 1 when the pixel to the right of a comparison is brighter, emitted
/// most-significant first so the hex reads left to right.
///
/// The hash survives a uniform brightness or contrast shift only *approximately*:
/// a shift that clips at white turns runs of pixels into ties, so the two hashes
/// differ by a few bits rather than not at all. That is why
/// [`hamming_distance`] and a threshold exist instead of string equality.
pub fn difference_hash(pixels: &[u8], width: u32, height: u32) -> Option<String> {
    let (w, h) = (width as usize, height as usize);
    if w < 2 || h < 2 {
        return None;
    }
    if pixels.len() != w.checked_mul(h)? {
        return None;
    }

    // dHash compares each pixel with the one to its left on a 9x8 grid: 8 rows
    // of 9 pixels give the 8x8 = 64 comparisons that make one 64-bit hash. The
    // grid is fixed rather than derived from the input, which is what makes two
    // hashes of the same image at different sizes comparable at all.
    const COLS: usize = 9;
    const ROWS: usize = 8;

    let mut bits: u64 = 0;
    for row in 0..ROWS {
        for col in 0..COLS - 1 {
            // Nearest-neighbour sample of the cell centred on this comparison.
            // Nearest rather than an area average: averaging blurs the exact
            // edge a difference hash is looking for, and the input is already
            // downsampled grayscale by the caller.
            let x0 = sample_column(col, COLS, w);
            let x1 = sample_column(col + 1, COLS, w);
            let y = sample_row(row, ROWS, h);
            let here = pixels[y * w + x1];
            let left = pixels[y * w + x0];
            if here > left {
                bits |= 1 << (63 - (row * (COLS - 1) + col));
            }
        }
    }
    Some(format!("{bits:016x}"))
}

/// The source column that grid cell `cell` of `cells` samples, mapping the
/// cell across the full width rather than taking the first `cells` pixels.
fn sample_column(cell: usize, cells: usize, width: usize) -> usize {
    cell * (width - 1) / (cells - 1)
}

/// The source row that grid cell `cell` of `cells` samples.
fn sample_row(cell: usize, cells: usize, height: usize) -> usize {
    cell * (height - 1) / (cells - 1)
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
    fn a_monotonic_ramp_is_its_own_perceptual_hash() {
        // A dHash over a left-to-right brightness ramp: every pixel is brighter
        // than its left neighbour, so every comparison says "brighter to the
        // right" and all 64 bits are set. This is the one image a hash can be
        // checked against without decoding anything.
        let ramp: Vec<u8> = (0..256u16).map(|i| i as u8).collect();
        let hash = difference_hash(&ramp, 16, 16);
        assert_eq!(hash, Some("f".repeat(16)));
    }

    #[test]
    fn a_flat_image_is_all_zero() {
        // Every pixel equals its left neighbour, so no comparison differs.
        let flat = vec![128u8; 256];
        assert_eq!(difference_hash(&flat, 16, 16), Some("0".repeat(16)));
    }

    #[test]
    fn a_ramp_is_unchanged_by_a_uniform_brightness_shift() {
        // This is the property the whole algorithm exists for: the same image
        // saved lighter or darker must produce the same fingerprint, or
        // dedup would only ever catch byte-identical files.
        let base: Vec<u8> = (0..256u16).map(|i| i as u8).collect();
        // A brightness shift clips at white, so the top of the ramp turns into
        // runs of equal pixels and those comparisons tie. That is why a dHash
        // is compared by distance rather than by equality - the guarantee is
        // "close", not "identical". Three clipped comparisons out of 64 is well
        // inside the default threshold of 6.
        let shifted: Vec<u8> = base.iter().map(|v| v.saturating_add(7)).collect();
        let distance = hamming_distance(
            &difference_hash(&base, 16, 16).expect("base"),
            &difference_hash(&shifted, 16, 16).expect("shifted"),
        )
        .expect("both are valid hex");
        assert!(
            distance <= 6,
            "a uniform brightness shift must stay within the default match threshold, got distance {distance}"
        );
    }

    #[test]
    fn a_vertical_ramp_differs_from_a_horizontal_one() {
        let mut horizontal = Vec::new();
        let mut vertical = Vec::new();
        for y in 0..16u8 {
            for x in 0..16u8 {
                horizontal.push(x * 16);
                vertical.push(y * 16);
            }
        }
        assert_ne!(
            difference_hash(&horizontal, 16, 16),
            difference_hash(&vertical, 16, 16)
        );
    }

    #[test]
    fn a_hash_uses_the_whole_image_not_just_its_first_rows() {
        // The first four rows ramp left-to-right and the last four invert it.
        // A hash that read only the top of the image would see a plain ramp and
        // score this the same as the all-ramp image; one that resamples the
        // whole frame must not.
        let mut split = Vec::new();
        for y in 0..16u8 {
            for x in 0..16u8 {
                split.push(if y < 8 { x * 16 } else { 255 - x * 16 });
            }
        }
        let all_ramp: Vec<u8> = (0..256u16).map(|i| i as u8).collect();
        assert_ne!(
            difference_hash(&split, 16, 16),
            difference_hash(&all_ramp, 16, 16),
            "a hash that only reads the top rows is blind to the bottom half"
        );
    }

    #[test]
    fn a_hash_of_a_non_square_image_is_some() {
        // dHash resamples to 9x8, so it must not require a square input: a
        // 32x18 faceclaim is a perfectly ordinary thing to attach.
        let wide: Vec<u8> = (0..(32u32 * 18u32)).map(|i| (i % 251) as u8).collect();
        let hash = difference_hash(&wide, 32, 18).expect("a 32x18 image fingerprints");
        assert_eq!(hash.len(), 16);
    }

    #[test]
    fn a_hash_of_a_tiny_image_is_some() {
        // 2x2 is the smallest image dHash can difference, and it must still
        // produce 64 bits rather than falling back to None.
        let tiny = vec![0u8, 255, 255, 0];
        let hash = difference_hash(&tiny, 2, 2).expect("2x2 can be differenced");
        assert_eq!(hash.len(), 16);
    }

    #[test]
    fn a_hash_of_the_wrong_pixel_count_is_none() {
        // 15x16 is 240 pixels, not 256. Returning a hash anyway would compare
        // a truncated image against a whole one and invent a distance.
        let wrong = vec![10u8; 240];
        assert_eq!(difference_hash(&wrong, 16, 16), None);
    }

    #[test]
    fn a_degenerate_dimension_is_none() {
        // A 16x0 image has no rows to compare, so there is no hash to speak of.
        assert_eq!(difference_hash(&[], 16, 0), None);
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
