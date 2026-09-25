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
//! Decoding lives here too, in [`fingerprint_encoded`], so a fetched body goes
//! from bytes to both hashes in one call. A body that will not decode yields
//! `None` rather than a guess, and the caller stores
//! [`MediaFingerprint::without_perceptual_hash`] — the exact content hash is
//! still correct, the perceptual one is honestly absent.
//!
//! Storing an empty perceptual hash would be worse than storing none: the dedup
//! search skips `NULL` and skips malformed values, but it would happily compare
//! two empty strings as distance 0 and merge every undecodable reference into
//! one.

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

/// Decode an encoded image body and fingerprint it.
///
/// This is the function that turns a real fetched JPEG or PNG into a real
/// perceptual hash. It returns `None` — not a guess — for anything it cannot
/// decode, and a caller should store [`MediaFingerprint::without_perceptual_hash`]
/// in that case so the exact content hash is still kept and the perceptual one
/// is honestly absent.
///
/// # Why `None` and not a best effort
///
/// An image decoder is lenient: a truncated file decodes to a partly-black
/// picture, and a body whose declared height exceeds its row data decodes to
/// fewer rows than it claims. Fingerprinting that produces a hash of a picture
/// nobody attached, and — worse — one that matches other partly-black pictures.
/// So every failure path is a refusal. The exact content hash is computed by
/// the caller from the same bytes and remains correct regardless.
///
/// # Limits
///
/// `MAX_MEDIA_BYTES` bounds the input (callers enforce it while streaming), and
/// decoding is bounded by [`MAX_DECODED_PIXELS`]: a small file can declare an
/// enormous image, and a 50000x50000 PNG of solid colour would otherwise
/// allocate 7.5 GB of grayscale buffer. A faceclaim or avatar is nowhere near
/// that limit, so refusing beyond it costs nothing real.
pub fn fingerprint_encoded(bytes: &[u8]) -> Option<MediaFingerprint> {
    if bytes.is_empty() {
        return None;
    }
    // The size limit is applied to the *decoder*, not checked after it. A
    // decompression bomb — a few KB of PNG declaring a 20000x20000 image — has
    // already allocated by the time a caller can measure the result, so a
    // post-decode check documents the intent and prevents nothing.
    //
    // `load_from_memory` applies only the crate's default `max_alloc` (512 MiB)
    // and leaves width and height unbounded, so an image under that allocation
    // cap but far past ours still gets built.
    let side = (MAX_DECODED_PIXELS as f64).sqrt() as u32;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes));
    // `Limits` is `#[non_exhaustive]`, so it is built from `Default` and the two
    // fields set; it cannot be constructed with a struct literal.
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(side);
    limits.max_image_height = Some(side);
    reader.limits(limits);
    let image = reader.with_guessed_format().ok()?.decode().ok()?;

    // `color()` rather than a `dimensions()` call: the size is read off the
    // buffer we are about to build, so the declared and actual sizes cannot
    // disagree. A decoder that honours a bogus header here would allocate for a
    // picture the file does not contain.
    let luma = image.into_luma8();
    let (width, height) = luma.dimensions();
    let pixels = (width as u64).checked_mul(height as u64)?;
    if pixels == 0 || pixels > MAX_DECODED_PIXELS {
        return None;
    }

    // `into_luma8` is the rec.601 luma transform, so a colour image and its
    // greyscale original agree — which matters, because the two are the same
    // picture and a curator would expect them to deduplicate together.
    MediaFingerprint::from_grayscale(bytes, luma.as_raw(), width, height)
}

/// The largest number of pixels a body may expand to before it is refused.
///
/// 80 megapixels is roughly an 11000x7300 photograph. It is deliberately well
/// above any avatar, faceclaim or banner an instance stores, and low enough
/// that a hostile 8 KB file cannot ask for gigabytes of grayscale buffer.
pub const MAX_DECODED_PIXELS: u64 = 80_000_000;

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
pub fn plan_fetch(url: &reqwest::Url, timeout: Duration) -> Result<FetchPlan, String> {
    plan_fetch_allowing(url, &[], timeout)
}

/// [`plan_fetch`], with a one-address allowlist that bypasses the *address* check
/// for exactly those addresses.
///
/// The scheme, local-name and resolution checks all still run. This exists so a
/// test can fetch from a loopback server without the guard being bypassed
/// wholesale: an `if` around the whole check would leave a second, untested path
/// through the most security-sensitive function in the chain, and a reviewer
/// could not tell which branch production takes. Here production passes an empty
/// allowlist and the refusal is the only path that exists for real traffic.
///
/// An allowlisted address still has to be one the server actually connects to,
/// so this cannot be used to point a fetch at a *different* host than the URL
/// names — the caller that resolves the host applies the same allowlist.
pub fn plan_fetch_allowing(
    url: &reqwest::Url,
    allow: &[IpAddr],
    _timeout: Duration,
) -> Result<FetchPlan, String> {
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
            if is_forbidden(IpAddr::V4(v4)) && !allow.contains(&IpAddr::V4(v4)) {
                return Err(format!("{v4} is not a routable public address"));
            }
        }
        url::Host::Ipv6(v6) => {
            if is_forbidden(IpAddr::V6(v6)) && !allow.contains(&IpAddr::V6(v6)) {
                return Err(format!("{v6} is not a routable public address"));
            }
        }
        url::Host::Domain(name) => {
            if lorehaven_scrapers::safety::is_local_hostname(name)
                && !allowlisted_domain(name, allow)
            {
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

/// Whether a hostname is served by one of the allowlisted addresses.
///
/// Only a test ever passes a non-empty allowlist, and only ever against a bare
/// loopback literal like `127.0.0.1` or `[::1]`, which `Url` reports as a
/// `Host::Ipv4`/`Host::Ipv6` rather than a `Domain` — so a domain reaches this
/// only when a caller allowlisted a literal, which is not the same address. The
/// conservative answer is therefore `false`: a name is never allowlisted, and the
/// test path keeps working through the address arms above.
fn allowlisted_domain(_name: &str, _allow: &[IpAddr]) -> bool {
    false
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

// ---------------------------------------------------------------------------
// Test-support image builders
// ---------------------------------------------------------------------------

/// A real 4x4 RGB PNG, for tests that need bytes a decoder accepts.
///
/// Built by hand rather than checked in as a fixture binary: a 68-byte PNG is
/// easier to review as code than as a blob, and it cannot drift from the
/// encoding it is supposed to represent. `push_chunk` writes the CRC the PNG
/// spec requires, and `zlib_store` emits *stored* (uncompressed) deflate blocks
/// so no compressor dependency is needed either.
#[must_use]
pub fn test_support_png_4x4() -> Vec<u8> {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&4u32.to_be_bytes());
    ihdr.extend_from_slice(&4u32.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(2); // colour type: truecolour RGB
    ihdr.extend_from_slice(&[0, 0, 0]);
    push_chunk(&mut png, b"IHDR", &ihdr);

    let mut raw = Vec::new();
    for row in 0..4u8 {
        raw.push(0); // filter: None
        for col in 0..4u8 {
            raw.push(col * 60); // red rises left to right
            raw.push(row * 60); // green rises top to bottom
            raw.push(128);
        }
    }
    push_chunk(&mut png, b"IDAT", &zlib_store(&raw));
    push_chunk(&mut png, b"IEND", &[]);
    png
}

/// A 16x16 greyscale PNG whose brightness rises left to right on every row, so
/// every one of dHash's 64 comparisons says "brighter to the right" and the
/// perceptual hash is `ffffffffffffffff`.
///
/// That is the one image whose perceptual hash can be checked by hand, which is
/// what makes it worth having separately from the 4x4 case.
#[must_use]
pub fn test_support_ramp_png() -> Vec<u8> {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&16u32.to_be_bytes());
    ihdr.extend_from_slice(&16u32.to_be_bytes());
    ihdr.push(8);
    ihdr.push(0); // greyscale
    ihdr.extend_from_slice(&[0, 0, 0]);
    push_chunk(&mut png, b"IHDR", &ihdr);

    let mut raw = Vec::with_capacity(16 * 17);
    for _ in 0..16u32 {
        raw.push(0);
        raw.extend((0..16u16).map(|col| (col * 17) as u8));
    }
    push_chunk(&mut png, b"IDAT", &zlib_store(&raw));
    push_chunk(&mut png, b"IEND", &[]);
    png
}

fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&png_crc(kind, data));
}

/// The PNG CRC over the chunk type and its data, per the spec's polynomial.
fn png_crc(kind: &[u8; 4], data: &[u8]) -> [u8; 4] {
    let mut crc = 0xffff_ffffu32;
    for byte in kind.iter().chain(data.iter()) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = if crc & 1 != 0 { 0xedb8_8320 } else { 0 };
            crc = (crc >> 1) ^ mask;
        }
    }
    (crc ^ 0xffff_ffff).to_be_bytes()
}

/// zlib "stored" (uncompressed) deflate: a 2-byte header, stored blocks, and
/// Adler-32. Valid zlib, so a real decoder accepts it, and small enough to
/// write out by hand.
fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    out.push(0x01); // final stored block
    out.extend_from_slice(&(data.len() as u16).to_le_bytes());
    out.extend_from_slice(&(!(data.len() as u16)).to_le_bytes());
    out.extend_from_slice(data);
    let (mut a, mut b) = (1u32, 0u32);
    for byte in data {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}
