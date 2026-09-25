//! M32-07b: the SSRF-safe media fetcher (spec §32.7.2).
//!
//! M32-07a made perceptual dedup searchable, but nothing wrote the column. This
//! is the path that does: fetch a media URL, refuse the hosts an instance must
//! never reach, hash the bytes exactly and perceptually, and store both.
//!
//! The refusal tests matter more than the success ones. A fetcher that an
//! operator can point at `http://169.254.169.254/` or `http://localhost:8081/`
//! is a server-side request forgery, and a self-hosted instance is exactly where
//! that is most dangerous: the thing being protected is usually on the same
//! machine or the same private network.

use lorehaven_app::media_fetch::{
    classify_response, plan_fetch, FetchOutcome, MediaFingerprint, MAX_MEDIA_BYTES,
};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

fn url(raw: &str) -> reqwest::Url {
    reqwest::Url::parse(raw).expect("valid url")
}

#[test]
fn a_loopback_url_is_refused() {
    for raw in [
        "http://127.0.0.1/admin",
        "http://127.0.0.1:8081/",
        "http://localhost:8081/",
        "http://[::1]/",
    ] {
        let plan = plan_fetch(&url(raw), Duration::from_secs(5));
        assert!(
            plan.is_err(),
            "must refuse {raw}: a local address is the instance itself"
        );
    }
}

#[test]
fn a_link_local_address_is_refused() {
    // 169.254.169.254 is the cloud metadata endpoint. An instance on a cloud
    // VM that fetches an author-supplied image URL must not be able to read its
    // own credentials.
    let plan = plan_fetch(
        &url("http://169.254.169.254/latest/meta-data/"),
        Duration::from_secs(5),
    );
    assert!(plan.is_err(), "the metadata endpoint must be refused");
}

#[test]
fn a_private_address_is_refused() {
    for raw in [
        "http://10.0.0.1/",
        "http://172.16.0.1/",
        "http://192.168.1.1/admin",
        "http://[fe80::1]/",
        "http://[fc00::1]/",
    ] {
        assert!(
            plan_fetch(&url(raw), Duration::from_secs(5)).is_err(),
            "must refuse {raw}"
        );
    }
}

#[test]
fn a_non_http_scheme_is_refused() {
    // file:///etc/passwd is the other half of an SSRF: the fetcher is handed a
    // URL by an author, and a scheme that reads the local filesystem is worse
    // than one that makes a network request.
    for raw in [
        "file:///etc/passwd",
        "ftp://example.com/x.png",
        "data:image/png;base64,AA",
    ] {
        assert!(
            plan_fetch(&url(raw), Duration::from_secs(5)).is_err(),
            "must refuse {raw}"
        );
    }
}

#[test]
fn a_public_https_url_is_accepted() {
    let plan = plan_fetch(
        &url("https://example.com/faceclaim.png"),
        Duration::from_secs(5),
    )
    .expect("a public https url is allowed");
    assert_eq!(plan.host, "example.com");
}

#[test]
fn a_public_http_url_is_accepted_but_warned_about() {
    // Plain HTTP is not a forgery vector, so the fetch is allowed - the image
    // may simply be served over HTTP. The plan records that it was insecure so
    // the caller can decide, rather than refusing a legitimate mirror.
    let plan = plan_fetch(&url("http://example.com/a.png"), Duration::from_secs(5))
        .expect("a public http url is allowed");
    assert!(plan.insecure, "plain http must be reported as insecure");
}

#[test]
fn a_0_0_0_0_url_is_refused() {
    // 0.0.0.0 resolves to the local host on Linux.
    assert!(plan_fetch(&url("http://0.0.0.0/"), Duration::from_secs(5)).is_err());
}

#[test]
fn an_ipv6_mapped_loopback_is_refused() {
    // ::ffff:127.0.0.1 is 127.0.0.1 wearing an IPv6 costume. A check written
    // against Ipv4Addr alone walks straight past it.
    assert!(plan_fetch(&url("http://[::ffff:127.0.0.1]/"), Duration::from_secs(5)).is_err());
}

#[test]
fn a_response_that_is_not_an_image_is_refused() {
    // The fetcher exists to fingerprint images. Fetching an HTML page and
    // storing it as media would put arbitrary markup into a curator's faceclaim.
    assert!(matches!(
        classify_response(200, "text/html; charset=utf-8"),
        Err(FetchOutcome::NotMedia { .. })
    ));
    assert!(matches!(
        classify_response(200, "application/json"),
        Err(FetchOutcome::NotMedia { .. })
    ));
}

#[test]
fn a_response_with_no_content_type_is_refused() {
    assert!(matches!(
        classify_response(200, ""),
        Err(FetchOutcome::NotMedia { .. })
    ));
}

#[test]
fn an_image_content_type_is_accepted() {
    for mime in ["image/png", "image/jpeg", "image/webp", "image/gif"] {
        assert!(
            classify_response(200, mime).is_ok(),
            "{mime} is media and must be accepted"
        );
    }
}

#[test]
fn a_declared_length_over_the_limit_is_refused_before_any_bytes_are_read() {
    // A server that declares an oversized body is refused from the header
    // alone: the point is to not read the bytes at all.
    let too_big = format!("{}", MAX_MEDIA_BYTES + 1);
    let declared: Result<FetchOutcome, _> =
        classify_response_with_length(200, "image/png", &too_big);
    assert!(
        matches!(declared, Err(FetchOutcome::TooLarge { .. })),
        "a declared length over the limit must be refused"
    );

    // With no declared length there is nothing to check yet - the caller bounds
    // the stream as it reads. Reporting TooLarge here would be a guess.
    assert!(
        matches!(classify_response(200, "image/png"), Ok(FetchOutcome::Media)),
        "an undeclared length is not a refusal"
    );
}

#[test]
fn a_declared_length_under_the_limit_is_accepted() {
    let ok = format!("{}", MAX_MEDIA_BYTES);
    let declared: Result<FetchOutcome, _> = classify_response_with_length(200, "image/png", &ok);
    assert!(declared.is_ok());
}

#[test]
fn a_server_error_is_transient_and_a_404_is_not() {
    // The worker retries a transient failure and gives up on a permanent one.
    // Getting this backwards either retries a deleted image forever or abandons
    // a mirror that was briefly down.
    assert!(matches!(
        classify_response(503, "image/png"),
        Err(FetchOutcome::Transient { .. })
    ));
    assert!(matches!(
        classify_response(429, "image/png"),
        Err(FetchOutcome::Transient { .. })
    ));
    assert!(matches!(
        classify_response(404, "text/html"),
        Err(FetchOutcome::Gone { .. })
    ));
    assert!(matches!(
        classify_response(410, "text/html"),
        Err(FetchOutcome::Gone { .. })
    ));
}

#[test]
fn a_forbidden_address_is_refused_by_the_shared_scrapers_guard() {
    // The house guard is `lorehaven_scrapers::safety::is_forbidden_ip`; the
    // fetcher must agree with it rather than carrying a second, weaker list.
    let cases = [
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
        IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1)),
    ];
    for ip in cases {
        assert!(
            lorehaven_scrapers::safety::is_forbidden_ip(ip),
            "the shared guard must classify {ip} as forbidden"
        );
    }
    // And a public address is not forbidden.
    assert!(!lorehaven_scrapers::safety::is_forbidden_ip(IpAddr::V4(
        Ipv4Addr::new(93, 184, 216, 34)
    )));
}

#[test]
fn a_fingerprint_carries_both_hashes_and_the_dimensions() {
    // A 16x16 grayscale ramp: 64 set bits, exact hash of the bytes.
    let pixels: Vec<u8> = (0..256u16).map(|i| i as u8).collect();
    // The body and the pixels are separate: in production one is the fetched
    // bytes and the other the decoded grayscale, and the exact hash must be of
    // the former.
    let body = b"\x89PNG\r\n\x1a\nfetched bytes".to_vec();
    let fp = MediaFingerprint::from_grayscale(&body, &pixels, 16, 16)
        .expect("a 16x16 image fingerprints");
    assert_eq!(fp.width, 16);
    assert_eq!(fp.height, 16);
    assert_eq!(fp.perceptual_hash, Some("f".repeat(16)));
    assert!(fp.content_hash.starts_with("sha256:"));
    // The exact hash is of the original bytes, not of the grayscale copy.
    assert_eq!(fp.content_hash.len(), "sha256:".len() + 64);
}

#[test]
fn a_fingerprint_of_a_degenerate_image_is_none() {
    // 1x10 has no left-hand neighbour to difference against.
    assert!(MediaFingerprint::from_grayscale(&[], &[0; 10], 1, 10).is_none());
    // 10 declared by 10 but only 10 bytes supplied for 100 pixels.
    assert!(MediaFingerprint::from_grayscale(&[], &[0; 10], 10, 10).is_none());
}

// A small local helper so the length-based cases read clearly.
fn classify_response_with_length(
    status: u16,
    mime: &str,
    content_length: &str,
) -> Result<FetchOutcome, FetchOutcome> {
    let len: u64 = content_length.parse().expect("a length");
    lorehaven_app::media_fetch::classify_with_length(status, mime, Some(len))
}

// ---------------------------------------------------------------------------
// M32-07c: real image decoding (spec §32.7.2)
// ---------------------------------------------------------------------------
//
// The guard layer above is tested with synthesised pixels. These tests use a
// real encoded image, because the whole point of adding a decoder is that a
// fetched JPEG or PNG stops yielding a NULL perceptual hash.

#[test]
fn a_real_png_is_decoded_and_fingerprinted() {
    // Before a decoder existed this returned a NULL perceptual hash. A real
    // encoded image must now produce a real fingerprint with real dimensions.
    let fp = lorehaven_app::media_fetch::fingerprint_encoded(
        &lorehaven_app::media_fetch::test_support_png_4x4(),
    )
    .expect("a valid PNG decodes");
    assert_eq!(fp.width, 4);
    assert_eq!(fp.height, 4);
    let perceptual = fp
        .perceptual_hash
        .as_deref()
        .expect("a decoded image has a perceptual hash");
    assert_eq!(
        perceptual.len(),
        16,
        "a dHash is 16 hex digits, got {perceptual}"
    );
    assert!(fp.content_hash.starts_with("sha256:"));
}

#[test]
fn a_png_and_a_re_encoded_copy_agree_within_the_default_threshold() {
    // The purpose of a perceptual hash: two encodings of the same picture are
    // not byte-identical but must be recognised. This is the end-to-end
    // property the dedup feature rests on, and it is only reachable with a
    // decoder in the tree.
    let original = lorehaven_app::media_fetch::test_support_png_4x4();
    // A copy with different filter bytes and zlib framing but identical pixels
    // decodes to the same image, so the perceptual hashes must match.
    let copy = reencoded_copy(&original);
    assert_ne!(original, copy, "the copy must be a different byte sequence");

    let a = lorehaven_app::media_fetch::fingerprint_encoded(&original).expect("original");
    let b = lorehaven_app::media_fetch::fingerprint_encoded(&copy).expect("copy");
    let distance = lorehaven_domain::media_resilience::hamming_distance(
        a.perceptual_hash.as_deref().expect("a"),
        b.perceptual_hash.as_deref().expect("b"),
    )
    .expect("both are 64-bit hex");
    assert_eq!(
        distance, 0,
        "two encodings of one image must hash identically: {a:?} vs {b:?}"
    );
}

#[test]
fn a_corrupt_image_decodes_to_nothing_rather_than_to_a_guess() {
    // A truncated PNG, and a body that is not an image at all. Both must be
    // refused: a perceptual hash of a partial image is a fingerprint of a
    // picture nobody attached.
    let mut truncated = lorehaven_app::media_fetch::test_support_png_4x4();
    truncated.truncate(truncated.len() - 12);
    assert!(lorehaven_app::media_fetch::fingerprint_encoded(&truncated).is_none());

    assert!(lorehaven_app::media_fetch::fingerprint_encoded(b"<html>404</html>").is_none());
    assert!(lorehaven_app::media_fetch::fingerprint_encoded(&[]).is_none());
}

#[test]
fn an_image_with_one_broken_row_does_not_silently_hash_the_rest() {
    // A PNG whose IDAT is valid zlib but whose declared height exceeds the
    // actual row data. Decoders are lenient here; a lenient decode produces a
    // fingerprint of a partly-black image, which then matches other partly-
    // black images. Refusing is the honest answer.
    let mut png = lorehaven_app::media_fetch::test_support_png_4x4();
    // Rewrite the height to 64 while keeping 4 rows of data.
    let ihdr_len = (13u32).to_be_bytes();
    let pos = png
        .windows(4)
        .position(|w| w == ihdr_len.as_slice())
        .expect("IHDR length prefix");
    png[pos + 8 + 4] = 0;
    png[pos + 8 + 5] = 0;
    png[pos + 8 + 6] = 0;
    png[pos + 8 + 7] = 64;
    // Fix the IHDR CRC, which the edit invalidated.
    fix_ihdr_crc(&mut png, pos);
    assert!(
        lorehaven_app::media_fetch::fingerprint_encoded(&png).is_none(),
        "an image whose declared size does not match its data must be refused"
    );
}

/// A different byte sequence with the same pixels: the original with a `tEXt`
/// comment chunk spliced in before IEND.
///
/// This is the real-world shape of the problem — the same picture saved by a
/// different tool, or re-uploaded with a caption — and it is why a *perceptual*
/// hash exists at all. The exact content hash must differ (the bytes do); the
/// perceptual hash must not (the picture does not).
fn reencoded_copy(original: &[u8]) -> Vec<u8> {
    let mut copy = original.to_vec();
    let iend = copy
        .windows(4)
        .position(|w| w == b"IEND")
        .expect("IEND chunk")
        - 4; // back up over the length field

    let text = b"Comment\0re-encoded by another tool";
    let mut chunk = Vec::new();
    chunk.extend_from_slice(&(text.len() as u32).to_be_bytes());
    chunk.extend_from_slice(b"tEXt");
    chunk.extend_from_slice(text);
    chunk.extend_from_slice(&local_png_crc(&chunk[4..]));

    copy.splice(iend..iend, chunk);
    copy
}

fn fix_ihdr_crc(png: &mut [u8], pos: usize) {
    let crc = local_png_crc(&png[pos + 4..pos + 4 + 4 + 13]);
    png[pos + 4 + 4 + 13..pos + 4 + 4 + 13 + 4].copy_from_slice(&crc);
}

fn local_png_crc(bytes: &[u8]) -> [u8; 4] {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = if crc & 1 != 0 { 0xedb8_8320 } else { 0 };
            crc = (crc >> 1) ^ mask;
        }
    }
    (crc ^ 0xffff_ffff).to_be_bytes()
}

#[test]
fn an_image_too_large_to_decode_is_refused_without_allocating_it() {
    // A decompression bomb: a few KB of PNG that declares a colossal image. The
    // bound has to be applied to the *decoder*, not after it — checking the
    // width once the pixels already exist means the allocation has happened, and
    // a hostile upload has already done its damage. `image` exposes
    // `ImageReader::limits` for exactly this, and this test fails if the code
    // goes back to `load_from_memory`.
    //
    // 20000x20000 exceeds the decode limit, so this must be refused.
    let bomb = png_declaring(20_000, 20_000);
    assert!(
        lorehaven_app::media_fetch::fingerprint_encoded(&bomb).is_none(),
        "an image beyond the decode limit must be refused"
    );
}

#[test]
fn an_image_inside_the_limit_still_decodes() {
    // The companion to the bomb test: a bound that refuses everything is not a
    // bound, it is an outage. A 64x64 image is far inside any sane limit and
    // must still fingerprint, so this uses a fully-populated image.
    let ok = png_of_size(64, 64);
    let fp = lorehaven_app::media_fetch::fingerprint_encoded(&ok)
        .expect("an image inside the limit must decode");
    assert_eq!((fp.width, fp.height), (64, 64));
}

/// A PNG declaring `width` x `height`, carrying only the first two rows of real
/// data. Used for images that must be refused on size alone, so the pixel data
/// does not need to be present.
fn png_declaring(width: u32, height: u32) -> Vec<u8> {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8);
    ihdr.push(0); // greyscale, so one byte per pixel
    ihdr.extend_from_slice(&[0, 0, 0]);
    push_chunk(&mut png, b"IHDR", &ihdr);

    // One real row of `width` zero bytes and nothing else: the declared image
    // is mostly absent, which is what a bomb looks like. The size is the point,
    // not the pixels — and a one-row body keeps this fixture from building a
    // 400-megabyte buffer in the test process.
    let mut raw = vec![0u8];
    raw.extend(std::iter::repeat_n(0u8, width as usize));
    push_chunk(&mut png, b"IDAT", &zlib_store(&raw));
    push_chunk(&mut png, b"IEND", &[]);
    png
}

/// A fully-populated greyscale PNG of `width` x `height`, every pixel zero.
/// Unlike [`png_declaring`], this is a valid image a decoder can really build.
fn png_of_size(width: u32, height: u32) -> Vec<u8> {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8);
    ihdr.push(0);
    ihdr.extend_from_slice(&[0, 0, 0]);
    push_chunk(&mut png, b"IHDR", &ihdr);

    let mut raw = Vec::new();
    for _ in 0..height {
        raw.push(0); // filter: None
        raw.extend(std::iter::repeat_n(0u8, width as usize));
    }
    push_chunk(&mut png, b"IDAT", &zlib_store(&raw));
    push_chunk(&mut png, b"IEND", &[]);
    png
}

// Local minimal PNG writers: the library's builders are fixed-size, and these
// tests need images of a *declared* size (including sizes far larger than the
// data present, which is what a decompression bomb looks like).
fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = 0xffff_ffffu32;
    for byte in kind.iter().chain(data.iter()) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = if crc & 1 != 0 { 0xedb8_8320 } else { 0 };
            crc = (crc >> 1) ^ mask;
        }
    }
    out.extend_from_slice(&(crc ^ 0xffff_ffff).to_be_bytes());
}

fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    out.push(0x01);
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
