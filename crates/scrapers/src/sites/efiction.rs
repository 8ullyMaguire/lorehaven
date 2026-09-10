//! The eFiction archive family
//!
//! **Status: not implemented.** This module exists so the registry, the
//! catalogue and the tests agree on the source's key and its hosts while the
//! parser is written. Every read returns [`SourceError::Unsupported`], which the
//! importer reports as a refusal rather than an empty work — the failure mode a
//! stub must never have is "succeeded with zero chapters".

use crate::{Credentials, Fetcher, SourceAdapter, SourceCapabilities, SourceChapter,
            SourceError, SourceKey, SourceResult, SourceWork};

/// The eFiction archive family
#[derive(Debug, Clone)]
pub struct EFiction {
    key: SourceKey,
}

impl EFiction {
    /// A new adapter for the one source this module covers.
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: SourceKey::new("efiction"),
        }
    }
}

#[async_trait::async_trait]
impl SourceAdapter for EFiction {
    fn key(&self) -> SourceKey {
        self.key.clone()
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            metadata: false,
            chapters: false,
            per_chapter_fetch: false,
            bibliography: false,
            incremental: false,
            authentication: crate::AuthKind::None,
            min_interval_millis: Some(1_000),
        }
    }

    fn can_handle(&self, _url: &url::Url) -> bool {
        false
    }

    fn hosts(&self) -> Vec<String> {
        Vec::new()
    }

    async fn preview(
        &self,
        _fetch: &dyn Fetcher,
        _url: &url::Url,
        _creds: Option<&Credentials>,
    ) -> SourceResult<SourceWork> {
        Err(SourceError::Unsupported(
            "the efiction adapter has not been written yet".to_owned(),
        ))
    }

    async fn fetch_chapters(
        &self,
        _fetch: &dyn Fetcher,
        _work: &SourceWork,
        _creds: Option<&Credentials>,
    ) -> SourceResult<Vec<SourceChapter>> {
        Err(SourceError::Unsupported(
            "the efiction adapter has not been written yet".to_owned(),
        ))
    }

    fn preview_from_html(&self, _html: &str, _url: &url::Url) -> SourceResult<SourceWork> {
        Err(SourceError::Unsupported(
            "the efiction adapter has not been written yet".to_owned(),
        ))
    }

    fn chapters_from_html(
        &self,
        _html: &str,
        _work: &SourceWork,
    ) -> SourceResult<Vec<SourceChapter>> {
        Err(SourceError::Unsupported(
            "the efiction adapter has not been written yet".to_owned(),
        ))
    }
}
