//! URL routing: which adapter handles this URL, and whether it is allowed to.
//!
//! Two questions live here and they are deliberately separate. *Who can read
//! this URL?* is a parsing question answered by the adapters. *May we?* is an
//! operational question — a source an operator has paused must not be touched
//! even though its adapter would happily read it (spec §11.8 lists `paused` as a
//! health state, and a paused source that still serves imports is not paused).
//!
//! The registry therefore never answers the first question without the second:
//! [`Registry::route`] returns an adapter only when the source is enabled, so
//! there is no way to hold a working adapter for a disabled source and call it
//! by accident.

use std::collections::BTreeMap;

use url::Url;

use crate::{SourceAdapter, SourceError, SourceKey, SourceResult};

/// The adapters this build knows, and the sources it may use.
pub struct Registry {
    adapters: Vec<Box<dyn SourceAdapter>>,
    /// Sources an operator has switched off. Keyed by source key.
    disabled: BTreeMap<SourceKey, String>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    /// An empty registry. Adapters are added with [`Registry::register`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            adapters: Vec::new(),
            disabled: BTreeMap::new(),
        }
    }

    /// Add an adapter. Order does not matter: routing asks each adapter in turn
    /// and the first that claims the URL wins, and no two adapters may claim the
    /// same host (asserted by a test in `sites`).
    pub fn register(&mut self, adapter: Box<dyn SourceAdapter>) -> &mut Self {
        self.adapters.push(adapter);
        self
    }

    /// Switch a source off, with the reason shown to a reader.
    ///
    /// Used for a source that is broken, paused by an operator, or subject to a
    /// policy restriction (spec §11.8's `unavailable` and `paused`). A disabled
    /// source is refused *before* an import job exists, so a pause does not
    /// leave a queue of jobs that will each fail.
    pub fn disable(&mut self, key: &SourceKey, reason: impl Into<String>) -> &mut Self {
        self.disabled.insert(key.clone(), reason.into());
        self
    }

    /// Every adapter this build has, whether or not it is enabled.
    #[must_use]
    pub fn adapters(&self) -> &[Box<dyn SourceAdapter>] {
        &self.adapters
    }

    /// Whether a source is enabled.
    #[must_use]
    pub fn is_enabled(&self, key: &SourceKey) -> bool {
        !self.disabled.contains_key(key)
    }

    /// Why a source is disabled, if it is.
    #[must_use]
    pub fn disabled_reason(&self, key: &SourceKey) -> Option<&str> {
        self.disabled.get(key).map(String::as_str)
    }

    /// Find the adapter for a URL, refusing a disabled source.
    ///
    /// # Errors
    /// * [`SourceError::Unsupported`] when no adapter claims the URL. The message
    ///   names the host, because "we do not support that site" is only useful if
    ///   it says which site.
    /// * [`SourceError::Refused`] when the adapter exists and the source is
    ///   disabled, carrying the operator's reason.
    pub fn route(&self, url: &Url) -> SourceResult<&dyn SourceAdapter> {
        let adapter = self
            .adapters
            .iter()
            .map(Box::as_ref)
            .find(|adapter| adapter.can_handle(url))
            .ok_or_else(|| {
                let host = url.host_str().unwrap_or("that host");
                SourceError::Unsupported(format!("no adapter handles {host}"))
            })?;

        let key = adapter.key();
        match self.disabled.get(&key) {
            Some(reason) => Err(SourceError::Refused(format!(
                "the {key} source is switched off: {reason}"
            ))),
            None => Ok(adapter),
        }
    }

    /// Find an adapter by key, refusing a disabled source.
    ///
    /// Used by the worker, which re-reads a job's source key rather than its URL
    /// and must reach the same answer as the request that enqueued it.
    ///
    /// # Errors
    /// [`SourceError::Unsupported`] when this build has no such adapter;
    /// [`SourceError::Refused`] when it is disabled.
    pub fn by_key(&self, key: &SourceKey) -> SourceResult<&dyn SourceAdapter> {
        if let Some(reason) = self.disabled.get(key) {
            return Err(SourceError::Refused(format!(
                "the {key} source is switched off: {reason}"
            )));
        }
        self.adapters
            .iter()
            .map(Box::as_ref)
            .find(|adapter| &adapter.key() == key)
            .ok_or_else(|| SourceError::Unsupported(format!("this build has no adapter for {key}")))
    }

    /// The source catalogue, for `GET /imports/sources` and the import page.
    #[must_use]
    pub fn catalogue(&self) -> Vec<CatalogueEntry> {
        self.adapters
            .iter()
            .map(|adapter| {
                let key = adapter.key();
                CatalogueEntry {
                    enabled: self.is_enabled(&key),
                    disabled_reason: self.disabled_reason(&key).map(str::to_owned),
                    key,
                    capabilities: adapter.capabilities(),
                }
            })
            .collect()
    }
}

/// One source as the catalogue reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueEntry {
    /// The source's key.
    pub key: SourceKey,
    /// Its declared capabilities.
    pub capabilities: crate::SourceCapabilities,
    /// Whether it may be used.
    pub enabled: bool,
    /// Why not, when it may not be.
    pub disabled_reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChapterRef, Credentials, Fetcher, SourceCapabilities, SourceChapter, SourceWork};
    use async_trait::async_trait;

    /// A stand-in adapter that claims one host and does nothing else.
    struct Fake {
        key: &'static str,
        host: &'static str,
    }

    #[async_trait]
    impl SourceAdapter for Fake {
        fn key(&self) -> SourceKey {
            SourceKey::new(self.key)
        }
        fn capabilities(&self) -> SourceCapabilities {
            SourceCapabilities::public_read()
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
                title: "T".into(),
                author_text: "A".into(),
                author_url: None,
                summary: String::new(),
                word_count: None,
                language: None,
                status: crate::WorkStatus::Unknown,
                published_at: None,
                updated_at: None,
                chapters: vec![ChapterRef {
                    ordinal: 1,
                    source_chapter_key: "1".into(),
                    title: "One".into(),
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

    fn registry() -> Registry {
        let mut registry = Registry::new();
        registry.register(Box::new(Fake {
            key: "ao3",
            host: "archiveofourown.org",
        }));
        registry.register(Box::new(Fake {
            key: "ffnet",
            host: "fanfiction.net",
        }));
        registry
    }

    fn url(raw: &str) -> Url {
        Url::parse(raw).unwrap()
    }

    #[test]
    fn routing_finds_the_adapter_by_host() {
        let registry = registry();
        assert_eq!(
            registry
                .route(&url("https://archiveofourown.org/works/1"))
                .unwrap()
                .key()
                .as_str(),
            "ao3"
        );
        assert_eq!(
            registry
                .route(&url("https://fanfiction.net/s/1"))
                .unwrap()
                .key()
                .as_str(),
            "ffnet"
        );
    }

    #[test]
    fn an_unknown_host_is_unsupported_and_says_which() {
        let registry = registry();
        let error = registry
            .route(&url("https://example.invalid/works/1"))
            .err()
            .expect("the host is unsupported");
        assert!(matches!(error, SourceError::Unsupported(_)), "{error:?}");
        assert!(error.to_string().contains("example.invalid"), "{error}");
    }

    #[test]
    fn a_disabled_source_is_refused_before_an_adapter_is_handed_out() {
        let mut registry = registry();
        registry.disable(
            &SourceKey::new("ao3"),
            "parser broken since the site change",
        );
        let error = registry
            .route(&url("https://archiveofourown.org/works/1"))
            .err()
            .expect("the source is switched off");
        assert!(matches!(error, SourceError::Refused(_)), "{error:?}");
        assert!(error.to_string().contains("parser broken"), "{error}");
        // And by key, the path the worker takes.
        let error = registry
            .by_key(&SourceKey::new("ao3"))
            .err()
            .expect("the source is switched off");
        assert!(matches!(error, SourceError::Refused(_)), "{error:?}");
        // The other source is untouched.
        assert!(registry.by_key(&SourceKey::new("ffnet")).is_ok());
        assert!(!registry.is_enabled(&SourceKey::new("ao3")));
        assert!(registry.is_enabled(&SourceKey::new("ffnet")));
    }

    #[test]
    fn the_catalogue_reports_absence_rather_than_omitting_it() {
        let mut registry = registry();
        registry.disable(&SourceKey::new("ffnet"), "policy restriction");
        let catalogue = registry.catalogue();
        assert_eq!(catalogue.len(), 2);
        let ao3 = catalogue.iter().find(|e| e.key.as_str() == "ao3").unwrap();
        assert!(ao3.enabled);
        assert!(ao3.disabled_reason.is_none());
        let ffnet = catalogue
            .iter()
            .find(|e| e.key.as_str() == "ffnet")
            .unwrap();
        assert!(!ffnet.enabled);
        assert_eq!(ffnet.disabled_reason.as_deref(), Some("policy restriction"));
    }

    #[test]
    fn an_adapter_that_this_build_lacks_is_reported_as_such() {
        let registry = registry();
        let error = registry
            .by_key(&SourceKey::new("nope"))
            .err()
            .expect("no such adapter");
        assert!(matches!(error, SourceError::Unsupported(_)), "{error:?}");
    }
}
