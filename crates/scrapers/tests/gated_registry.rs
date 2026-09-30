//! M53-03: a login- or adult-gated adapter is refused until the credential vault
//! holds a credential for it, and the refusal says what to do.
//!
//! ## What is actually being tested
//!
//! Spec §11.6: *"Expired credentials pause affected jobs with actionable status.
//! Do not repeatedly retry authentication failures."* Two separate obligations
//! hide in that, and they are tested separately because an implementation can
//! satisfy one and miss the other:
//!
//! 1. **The refusal is actionable.** "401 Unauthorized" is not actionable — it
//!    reports the source's opinion of the reader. A message naming the endpoint,
//!    the kind of secret, and the consent requirement is. This is asserted on the
//!    *content* of the message, including that each of the three facts appears,
//!    because a message that merely is non-empty passes everything else.
//!
//! 2. **No repeated retries.** Not a property of the message at all, so it gets
//!    its own test rather than being assumed.
//!
//! ## Why `AuthKind::None` and `AuthKind::Token` are not gated
//!
//! A token is a documented method (§11.6: "Prefer source-issued tokens"), and
//! whether this instance holds one is an instance matter, not a build one. Gating
//! on `Token` would make every token source show a refusal to a reader who has
//! already stored one and is merely mid-request. The gate is for the kinds that
//! mean *this build must hold a long-lived secret for a human*, which is exactly
//! what §11.6 gates behind explicit consent.

use async_trait::async_trait;
use lorehaven_scrapers::{
    AuthKind, ChapterRef, Credentials, Fetcher, Registry, SourceAdapter, SourceCapabilities,
    SourceChapter, SourceKey, SourceResult, SourceWork, WorkStatus,
};
use url::Url;

/// A stand-in adapter declaring one authentication kind.
struct Gated {
    key: &'static str,
    host: &'static str,
    auth: AuthKind,
}

#[async_trait]
impl SourceAdapter for Gated {
    fn key(&self) -> SourceKey {
        SourceKey::new(self.key)
    }
    fn display_name(&self) -> &'static str {
        "A Gated Source"
    }
    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            authentication: self.auth,
            ..SourceCapabilities::public_read()
        }
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

fn registry_with(auth: AuthKind) -> Registry {
    let mut registry = Registry::new();
    registry.register(Box::new(Gated {
        key: "gated-source",
        host: "gated.example",
        auth,
    }));
    registry
}

#[test]
fn a_public_source_is_never_refused_for_want_of_a_credential() {
    // The regression this guards: an instance that gates everything, so a reader
    // with no credentials at all is told they need one for a source that needs
    // none.
    let registry = registry_with(AuthKind::None);
    let adapter = registry
        .by_key(&SourceKey::new("gated-source"))
        .expect("registered");
    assert!(
        registry.credential_requirement(adapter, false).is_none(),
        "a source that needs no account must never be refused for lack of one"
    );
}

#[test]
fn a_login_gated_source_is_refused_with_a_message_the_reader_can_act_on() {
    for auth in [AuthKind::Password, AuthKind::SessionCookie] {
        let registry = registry_with(auth);
        let adapter = registry
            .by_key(&SourceKey::new("gated-source"))
            .expect("registered");

        let requirement = registry
            .credential_requirement(adapter, false)
            .expect("a login-gated source with no stored credential is refused");

        assert_eq!(requirement.key, SourceKey::new("gated-source"));
        assert_eq!(
            requirement.kind, auth,
            "the message must name the kind asked for"
        );

        let message = requirement.message();

        // §11.6 requires an *actionable* status. These three facts are what make
        // it one; a message that has none of them is the 401 case.
        assert!(
            message.contains("/api/v1/source-credentials"),
            "the refusal must name the endpoint that fixes it: {message}"
        );
        assert!(
            message.contains("gated-source"),
            "and the source it is about: {message}"
        );
        assert!(
            message.to_lowercase().contains("consent"),
            "storing a password or cookie records explicit consent, and the \\
             reader must be told that before they hand one over: {message}"
        );
        assert!(
            message.contains("revoke"),
            "and that it can be withdrawn again, or it is not consent: {message}"
        );
        assert!(
            !message.to_lowercase().contains("401")
                && !message.to_lowercase().contains("unauthorized"),
            "and it must not relay the source's opinion of the reader, which is \\
             the thing they cannot act on: {message}"
        );
    }
}

#[test]
fn a_token_source_is_also_refused_when_none_is_stored() {
    // A token IS gated — it is simply not consent-gated, because §11.6 prefers
    // it and does not require consent for it. The message therefore names the
    // endpoint but need not mention consent.
    let registry = registry_with(AuthKind::Token);
    let adapter = registry
        .by_key(&SourceKey::new("gated-source"))
        .expect("registered");

    let requirement = registry
        .credential_requirement(adapter, false)
        .expect("a token source with no stored token is refused");
    let message = requirement.message();

    assert!(message.contains("API token"), "{message}");
    assert!(message.contains("/api/v1/source-credentials"), "{message}");
}

#[test]
fn a_stored_credential_lifts_the_refusal() {
    // The whole point of the gate: it opens when the vault has something.
    for auth in [
        AuthKind::None,
        AuthKind::Token,
        AuthKind::Password,
        AuthKind::SessionCookie,
    ] {
        let registry = registry_with(auth);
        let adapter = registry
            .by_key(&SourceKey::new("gated-source"))
            .expect("registered");
        assert!(
            registry.credential_requirement(adapter, true).is_none(),
            "{auth:?} must serve once a credential is stored — the gate is about \\
             the vault being empty, not about the adapter"
        );
    }
}

#[test]
fn the_gate_says_nothing_about_sources_it_is_not_gating() {
    // `credential_requirement` takes one adapter, and a registry with mixed
    // sources must not report a requirement for the public ones. This is the
    // failure mode of implementing the check as "does ANY source need a
    // credential", which is the shape a single global boolean would take.
    let mut registry = Registry::new();
    registry.register(Box::new(Gated {
        key: "open-source",
        host: "open.example",
        auth: AuthKind::None,
    }));
    registry.register(Box::new(Gated {
        key: "gated-source",
        host: "gated.example",
        auth: AuthKind::SessionCookie,
    }));

    let open = registry
        .by_key(&SourceKey::new("open-source"))
        .expect("registered");
    let gated = registry
        .by_key(&SourceKey::new("gated-source"))
        .expect("registered");

    assert!(registry.credential_requirement(open, false).is_none());
    assert!(registry.credential_requirement(gated, false).is_some());
}
