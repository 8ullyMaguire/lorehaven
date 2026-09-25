//! The SSRF-safe media fetcher (spec §32.7.2).
//!
//! When an author attaches an image, a URL arrives from a person and has to be
//! fetched by the server. That makes the fetcher a server-side request forgery
//! waiting to happen, and a self-hosted instance is where it is most dangerous:
//! what is being protected is usually on the same machine or the same private
//! network as the thing doing the protecting. So the guard here is the shared
//! one in `lorehaven_scrapers::safety` — the same list the importer uses — and
//! not a second, weaker copy that can drift away from it.
//!
//! What this module does *not* do is decode images. A dHash needs grayscale
//! pixels, and turning JPEG or PNG bytes into pixels is an image-decoder
//! dependency the workspace does not have. So the two halves are split:
//!
//! - [`MediaFingerprint::from_grayscale`] fingerprints pixels a caller already
//!   has, and is fully tested;
//! - [`FetchOutcome::NeedsDecoding`] is what a fetched-but-undecodable body
//!   produces, so a caller cannot mistake "fetched" for "fingerprinted" and
//!   store an empty hash.
//!
//! Storing an empty perceptual hash would be worse than storing none: the dedup
//! search skips `NULL` and skips malformed values, but it would happily compare
//! two empty strings as distance 0 and merge every unfingerprinted reference
//! into one.

use lorehaven_domain::media_resilience::difference_hash;
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use std::time::Duration;

/// The largest media body the fetcher will read, in bytes.
///
/// A faceclaim image is comfortably under a megabyte. The limit exists so an
/// author cannot point the fetcher at a multi-gigabyte file and fill the
/// instance's disk or memory; it is a constant rather than config because it
/// bounds memory use during a fetch, and a configurable memory bound is not a
/// bound anyone can rely on. 16 MiB covers a large uncompressed screenshot.
pub const MAX_MEDIA_BYTES: u64 = 16 * 1024 * 1024;

/// A URL that passed the scheme and address checks, with what the caller needs
/// to fetch it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPlan {
    /// The URL as given, unchanged.
    pub url: reqwest::Url,
    /// The host to resolve and check, without brackets.
    pub host: String,
    /// Plain HTTP. Allowed, because a mirror may simply serve images over HTTP,
    /// but reported so the caller can record it or refuse it.
    pub insecure: bool,
}

/// What a fetched body turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    /// The response is media of an acceptable size.
    Media,
    /// The response is not an image, so there is nothing to fingerprint.
    NotMedia { content_type: String },
    /// The body is larger than [`MAX_MEDIA_BYTES`].
    TooLarge { limit: u64 },
    /// The server failed in a way that may succeed later. The worker retries.
    Transient { status: u16 },
    /// The resource is gone for good. The worker must not retry.
    Gone { status: u16 },
    /// The body arrived but this build cannot decode it, so no perceptual hash
    /// can be computed. The exact content hash still can be.
    NeedsDecoding,
}

/// Both hashes of one media body, plus what is known about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFingerprint {
    /// `sha256:` followed by the hex digest of the original bytes. This is the
    /// exact-match key: identical bytes produce an identical string, and
    /// spec §32.7.2 auto-attaches on it.
    pub content_hash: String,
    /// The 16-hex-digit dHash of the grayscale pixels, or `None` when the image
    /// could not be decoded to pixels. `None` is not an empty string: an absent
    /// hash is skipped by the dedup search, while an empty one would compare as
    /// distance 0 against every other absent hash.
    pub perceptual_hash: Option<String>,
    pub width: u32,
    pub height: u32,
}

impl MediaFingerprint {
    /// Fingerprint already-decoded pixels.
    ///
    /// `bytes` is the original body and is hashed exactly; `pixels` is the
    /// row-major grayscale the perceptual hash is computed from. They are
    /// separate arguments on purpose: the exact hash must be of what was
    /// fetched, not of a converted copy, or a re-encode would stop matching the
    /// original it came from.
    ///
    /// Returns `None` when the dimensions do not describe `pixels`, or when the
    /// image is too small to difference-hash (a dHash needs at least 2×2).
    pub fn from_grayscale(bytes: &[u8], pixels: &[u8], width: u32, height: u32) -> Option<Self> {
        let perceptual_hash = difference_hash(pixels, width, height)?;
        Some(Self {
            content_hash: content_hash(bytes),
            perceptual_hash: Some(perceptual_hash),
            width,
            height,
        })
    }

    /// A fingerprint of bytes this build could not decode: the exact hash is
    /// real, the perceptual hash is honestly absent.
    #[must_use]
    pub fn without_perceptual_hash(bytes: &[u8]) -> Self {
        Self {
            content_hash: content_hash(bytes),
            perceptual_hash: None,
            width: 0,
            height: 0,
        }
    }
}

impl From<&MediaFingerprint> for lorehaven_db::media_resilience::Fingerprint {
    fn from(fp: &MediaFingerprint) -> Self {
        Self {
            content_hash: fp.content_hash.clone(),
            perceptual_hash: fp.perceptual_hash.clone(),
            // 0 means "not known"; the column is nullable and 0 is not a real
            // width, so a decoded image and an undecodable one are
            // distinguishable without a second column.
            width: (fp.width > 0).then_some(i64::from(fp.width)),
            height: (fp.height > 0).then_some(i64::from(fp.height)),
        }
    }
}

/// The exact content hash of a body: `sha256:` plus hex.
#[must_use]
pub fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// Decide whether a URL may be fetched, before any connection is made.
///
/// Checks the scheme and, for a literal address, the address itself. A hostname
/// is not resolved here — `plan_fetch` is synchronous and resolution is not — so
/// a caller must still pass the host through
/// `lorehaven_scrapers::safety::resolve_public` before connecting. That split is
/// deliberate: resolving in two places is how one of them ends up skipped.
///
/// # Errors
/// A message naming the reason, so the author's media row can say why it was
/// not fetched rather than failing anonymously.
pub fn plan_fetch(url: &reqwest::Url, _timeout: Duration) -> Result<FetchPlan, String> {
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("{other} is not a fetchable scheme")),
    }

    // `url.host()` rather than `host_str()`: for an IPv6 literal the string
    // form keeps its brackets, so `host_str().parse::<IpAddr>()` fails on
    // `http://[::1]/` and the address check is skipped entirely - which is
    // exactly the case it exists to catch. The typed host carries the address.
    let host = url.host().ok_or_else(|| "the url has no host".to_owned())?;

    // A literal address needs no resolver and must not get one.
    match host {
        url::Host::Ipv4(v4) => {
            if is_forbidden(IpAddr::V4(v4)) {
                return Err(format!("{v4} is not a routable public address"));
            }
        }
        url::Host::Ipv6(v6) => {
            if is_forbidden(IpAddr::V6(v6)) {
                return Err(format!("{v6} is not a routable public address"));
            }
        }
        url::Host::Domain(name) => {
            if lorehaven_scrapers::safety::is_local_hostname(name) {
                return Err(format!("{name} is a local name"));
            }
        }
    }
    let host = host.to_string();

    Ok(FetchPlan {
        url: url.clone(),
        host,
        insecure: url.scheme() == "http",
    })
}

/// Whether an address must never be connected to.
///
/// Delegates to the shared scraper guard, then adds the two cases it does not
/// cover: a v4-mapped v6 address, and the unspecified address. `::ffff:127.0.0.1`
/// is `127.0.0.1` wearing an IPv6 costume, and a check written against
/// `Ipv4Addr` alone walks straight past it.
fn is_forbidden(ip: IpAddr) -> bool {
    if let IpAddr::V6(v6) = ip {
        if let Some(mapped) = v6.to_ipv4_mapped() {
            return is_forbidden(IpAddr::V4(mapped));
        }
        if v6.is_unspecified() {
            return true;
        }
    }
    if ip.is_unspecified() {
        return true;
    }
    lorehaven_scrapers::safety::is_forbidden_ip(ip)
}

/// Whether a status and content type describe media worth reading.
pub fn classify_response(status: u16, content_type: &str) -> Result<FetchOutcome, FetchOutcome> {
    classify_with_length(status, content_type, None)
}

/// [`classify_response`], with the declared `Content-Length` when the server
/// sent one.
///
/// The length is checked *before* the content type, so a server that declares a
/// gigabyte is refused without its body being read. A server that declares
/// nothing is bounded by the caller as it streams.
pub fn classify_with_length(
    status: u16,
    content_type: &str,
    content_length: Option<u64>,
) -> Result<FetchOutcome, FetchOutcome> {
    if let Some(len) = content_length {
        if len > MAX_MEDIA_BYTES {
            return Err(FetchOutcome::TooLarge {
                limit: MAX_MEDIA_BYTES,
            });
        }
    }
    match status {
        200..=299 => {}
        404 | 410 => return Err(FetchOutcome::Gone { status }),
        // Retryable: the mirror is down, rate-limiting, or erroring temporarily.
        408 | 425 | 429 => return Err(FetchOutcome::Transient { status }),
        500..=599 => return Err(FetchOutcome::Transient { status }),
        // 3xx that reqwest did not follow, and any other 4xx: the request itself
        // is wrong, so retrying it unchanged cannot help.
        other => return Err(FetchOutcome::Gone { status: other }),
    }

    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if !mime.starts_with("image/") {
        return Err(FetchOutcome::NotMedia {
            content_type: content_type.to_owned(),
        });
    }
    Ok(FetchOutcome::Media)
}
