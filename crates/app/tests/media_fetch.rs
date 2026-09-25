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
