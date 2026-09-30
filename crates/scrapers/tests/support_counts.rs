//! M53-04: a `blocked-here` adapter is excluded from support counts, and the
//! exclusion is visible rather than silently folded into the total.
//!
//! ## What is actually being tested
//!
//! Spec §11.7: *"Do not promise a source count in advance. Adapter counts are an
//! outcome of verified implementation, never a marketing claim."*
//!
//! The obvious implementation — count the adapters — passes every plausible test
//! and violates the sentence. What makes the sentence mean something is that
//! `supported` is *smaller* than the number of adapters shipped, and that the gap
//! is reported rather than hidden. So these tests assert the gap exists and is
//! accounted for, not merely that a count is returned.
//!
//! ## Why this is not health
//!
//! §11.8 warns against overstating a source's state: "Do not label a source
//! unavailable because one user's credentials expired." The same reasoning applies
//! harder to a wall this *build host* cannot get past — the source may be serving
//! every other reader perfectly. A `blocked-here` adapter is therefore still
//! shipped, still enabled, and still parses its fixtures. It is *unverified*, not
//! broken. The enablement assertions below exist because collapsing this into
//! "disabled" is the most likely way to get it wrong, and a disabled adapter would
//! silently vanish from the catalogue in a way that looks like a removal.

use async_trait::async_trait;
use lorehaven_scrapers::{
    ChapterRef, Credentials, Fetcher, Registry, SourceAdapter, SourceCapabilities, SourceChapter,
    SourceKey, SourceResult, SourceWork, VerificationStatus, Wall, WorkStatus,
};
use url::Url;

/// A stand-in adapter whose verification status the test chooses.
struct Fake {
    key: &'static str,
    host: &'static str,
    verification: VerificationStatus,
}

#[async_trait]
impl SourceAdapter for Fake {
    fn key(&self) -> SourceKey {
        SourceKey::new(self.key)
    }
    fn display_name(&self) -> &'static str {
        "A Fake Source"
    }
    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::public_read()
    }
    fn verification(&self) -> VerificationStatus {
        self.verification
    }
    fn hosts(&self) -> Vec<String> {
        vec![self.host.to_owned()]
    }
    fn can_handle(&self, url: &Url) -> bool {
        url.host_str() == Some(self.host)
    }
    async fn preview(
        &self,
        _fetch: &dyn Fetcher,
        url: &Url,
        _creds: Option<&Credentials>,
    ) -> SourceResult<SourceWork> {
        Ok(SourceWork {
            source_key: self.key(),
            source_work_key: url.path().to_owned(),
            source_url: url.to_string(),
            title: "T".to_owned(),
            author_text: "A".to_owned(),
            author_url: None,
            summary: String::new(),
            word_count: None,
            language: None,
            status: WorkStatus::Unknown,
            published_at: None,
            updated_at: None,
            chapters: vec![ChapterRef {
                ordinal: 1,
                source_chapter_key: "1".to_owned(),
                title: "One".to_owned(),
            }],
            rating_text: None,
            warning_texts: Vec::new(),
            tags: Vec::new(),
        })
    }
    async fn fetch_chapters(
        &self,
        _fetch: &dyn Fetcher,
        _work: &SourceWork,
        _creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceChapter>> {
        Ok(Vec::new())
    }
}

fn verified(host: &'static str) -> Fake {
    Fake {
        key: "verified-source",
        host,
        verification: VerificationStatus::Verified,
    }
}

fn blocked(host: &'static str, reason: &'static str) -> Fake {
    Fake {
        key: "blocked-source",
        host,
        verification: VerificationStatus::BlockedHere { reason },
    }
}

#[test]
fn a_blocked_source_is_not_counted_as_supported() {
    let mut registry = Registry::new();
    registry.register(Box::new(verified("good.example")));
    registry.register(Box::new(blocked("walled.example", "Cloudflare challenge")));

    let counts = registry.support_counts();

    // The point of the row: `supported` is 1, not 2. Counting the adapters would
    // give 2 and satisfy every test that only checked a number came back.
    assert_eq!(
        counts.supported, 1,
        "a source this host cannot verify must not count as supported: {counts:?}"
    );
    assert_eq!(counts.blocked_here, 1);
    assert_eq!(
        counts.total, 2,
        "total is every adapter shipped, and the gap between it and `supported` \
         is the whole point"
    );
}

#[test]
fn the_excluded_set_is_reported_with_its_reason() {
    let mut registry = Registry::new();
    registry.register(Box::new(verified("good.example")));
    registry.register(Box::new(blocked("walled.example", "Cloudflare challenge")));

    let counts = registry.support_counts();

    // A count with no visible exclusion cannot be checked against §11.7 by
    // anyone but whoever wrote it. The reason has to travel with the number.
    assert_eq!(counts.blocked_reasons.len(), 1);
    let (key, reason) = &counts.blocked_reasons[0];
    assert_eq!(key, "blocked-source");
    assert_eq!(reason, "Cloudflare challenge");
    assert!(
        !counts.is_complete(),
        "a build with a blocked source is not complete"
    );
}

#[test]
fn a_blocked_source_is_still_shipped_and_enabled() {
    // The distinction this row exists to protect. A `blocked-here` adapter is
    // unverified from this host, not broken and not removed: it is enabled, it
    // is in the catalogue, and it still parses.
    let mut registry = Registry::new();
    registry.register(Box::new(verified("good.example")));
    registry.register(Box::new(blocked("walled.example", "Cloudflare challenge")));

    assert!(
        registry.is_enabled(&SourceKey::new("blocked-source")),
        "blocked-here is a statement about evidence, not about whether the adapter \
         may be used"
    );
    let catalogue = registry.catalogue();
    assert_eq!(catalogue.len(), 2, "it is still in the catalogue");

    let blocked_entry = catalogue
        .iter()
        .find(|e| e.key == SourceKey::new("blocked-source"))
        .expect("the blocked source is catalogued");
    assert_eq!(blocked_entry.verification.as_str(), "blocked-here");
    assert!(
        blocked_entry.disabled_reason.is_none(),
        "and it is not disabled: {:?}",
        blocked_entry.disabled_reason
    );
}

#[test]
fn a_build_with_nothing_blocked_is_complete() {
    let mut registry = Registry::new();
    registry.register(Box::new(verified("a.example")));
    registry.register(Box::new(verified("b.example")));

    let counts = registry.support_counts();
    assert_eq!(counts.supported, 2);
    assert_eq!(counts.blocked_here, 0);
    assert_eq!(counts.total, 2);
    assert!(counts.is_complete());
    assert!(counts.blocked_reasons.is_empty());
}

#[test]
fn the_real_registry_reports_a_consistent_account_of_itself() {
    // Every adapter in the shipped build. The invariant is the one that matters
    // and that a hand-written fixture registry cannot check: the three numbers
    // agree, and nothing is silently both counted and excluded.
    let counts = lorehaven_scrapers::sites::default_registry().support_counts();

    assert!(
        counts.supported > 0,
        "the shipped build ships adapters that were verified: {counts:?}"
    );
    assert_eq!(
        counts.supported + counts.blocked_here,
        counts.total,
        "every adapter is either supported or blocked-here, never neither and \
         never both: {counts:?}"
    );
    assert_eq!(
        counts.blocked_reasons.len(),
        counts.blocked_here,
        "each excluded source carries exactly one reason: {counts:?}"
    );
}

#[test]
fn a_blocked_source_does_not_declare_a_wall_of_its_own() {
    // Guards a real confusion: `Wall` is what the *source* demands of a client;
    // `BlockedHere` is what *this host* hit. An adapter claiming both has either
    // confused the two, or has honestly found a wall it could not pass — in
    // which case the reason must say which. Either way `Wall::None` with
    // `BlockedHere` is the coherent pairing for "we simply cannot get there".
    let adapter = blocked("walled.example", "geo-blocked from this region");
    assert_eq!(
        adapter.wall(),
        Wall::None,
        "BlockedHere and Wall answer different questions and must not be conflated"
    );
}
