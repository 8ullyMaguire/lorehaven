//! A fetcher that remembers what it read (spec §10.4, §11.7).
//!
//! # Why this is a decorator and not an adapter concern
//!
//! Every adapter in `lorehaven-scrapers` fetches pages through [`Fetcher`] and
//! knows nothing about caching. That is deliberate. If conditional requests were
//! something each adapter had to opt into, then a new adapter would silently miss
//! them, and the bug would look like "the update check is slow" rather than
//! "somebody forgot a line". Wrapping the fetcher means a source gains cheap
//! update checks the moment its adapter declares nothing at all — the behaviour
//! follows from the plumbing, not from anyone's memory.
//!
//! # What the reader sees
//!
//! Nothing changes. A `304` means the source confirmed the revision we hold, so
//! the reader gets the same bytes on the same code path; the only difference is
//! that the bytes arrived without a body.
//!
//! # When it stands aside
//!
//! * A `POST` is a question rather than a page, and is never cached or answered
//!   from a cache.
//! * A cached entry whose bytes have gone falls back to a plain fetch. The
//!   reader asked for a page, not for a cache hit, so a missing blob is one
//!   wasted request rather than a failed import.
//! * A cache that cannot be read at all — the database is down, the row is
//!   corrupt — is logged and skipped. Being an optimisation, the cache must
//!   never be the reason an import fails.

use std::path::PathBuf;

use async_trait::async_trait;
use lorehaven_db::storage::BlobStore;
use lorehaven_db::{revisions, Database};
use lorehaven_scrapers::{
    ConditionalFetch, Fetched, Fetcher, RevisionValidators, SourceError, SourceResult,
};

/// How long a stored revision stays usable as a validator.
///
/// Long, and deliberately so. This is not how long a page is considered fresh —
/// a cached body is only ever served when the *source itself* answers `304`. The
/// number only decides how long we bother asking conditionally before falling
/// back to reading the page in full, which is what we would have done anyway.
/// So a long window costs nothing in correctness and saves a body on every read
/// for as long as the source keeps honouring its own validator.
pub const REVISION_TTL_SECONDS: i64 = 7 * 24 * 60 * 60;

/// Wraps a fetcher so every page it reads is remembered and re-asked about.
pub struct CachingFetcher<'a, F> {
    inner: F,
    db: &'a Database,
    store: BlobStore,
    source_key: String,
    adapter_version: String,
    security_scope: String,
}

impl<'a, F> CachingFetcher<'a, F> {
    /// Wrap `inner`, filing what it reads under this source and scope.
    ///
    /// `security_scope` must distinguish a read made with a credential from one
    /// made without: [`revisions::RevisionKey`] is keyed by it, because a page
    /// served under somebody's login must never be handed to a request that has
    /// none.
    pub fn new(
        inner: F,
        db: &'a Database,
        storage_root: PathBuf,
        source_key: &str,
        adapter_version: &str,
        security_scope: &str,
    ) -> Self {
        Self {
            inner,
            db,
            store: BlobStore::new(storage_root),
            source_key: source_key.to_owned(),
            adapter_version: adapter_version.to_owned(),
            security_scope: security_scope.to_owned(),
        }
    }

    fn key<'k>(&'k self, url: &'k str) -> revisions::RevisionKey<'k> {
        revisions::RevisionKey {
            source_key: &self.source_key,
            revision_key: url,
            adapter_version: &self.adapter_version,
            security_scope: &self.security_scope,
        }
    }

    /// The stored entry for a URL, or `None` if there is nothing usable.
    ///
    /// Every failure is a `None` with a log line: see the module docs.
    async fn cached(&self, url: &str) -> Option<revisions::RevisionEntry> {
        let key = self.key(url);
        match revisions::find_fresh(self.db, &key).await {
            Ok(entry) => entry.filter(revisions::RevisionEntry::is_usable_conditionally),
            Err(error) => {
                tracing::warn!(url, %error, "could not read the revision cache; reading the page in full");
                None
            }
        }
    }

    /// The bytes behind a checksum, or `None` if they are gone.
    async fn body_for(&self, checksum: &str) -> Option<String> {
        match self.store.get(self.db, checksum).await {
            Ok(Some(bytes)) => String::from_utf8(bytes).ok(),
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(checksum, %error, "could not read a cached revision's bytes");
                None
            }
        }
    }

    /// File a freshly read page away for next time.
    ///
    /// A page with no validators is not stored: there would be nothing to ask
    /// about, so the row would be a body nobody may serve and a cache that only
    /// grows.
    async fn remember(&self, url: &str, page: &Fetched) {
        if page.etag.is_none() && page.last_modified.is_none() {
            return;
        }
        let checksum = match self
            .store
            .put_fetch_cache(self.db, page.body.as_bytes(), "text/html")
            .await
        {
            Ok((checksum, _)) => checksum,
            Err(error) => {
                tracing::warn!(url, %error, "could not store a cached revision");
                return;
            }
        };
        let entry = revisions::RevisionEntry {
            checksum,
            etag: page.etag.clone(),
            last_modified: page.last_modified.clone(),
            expires_at: lorehaven_db::identity::in_seconds(REVISION_TTL_SECONDS),
        };
        let key = self.key(url);
        if let Err(error) = revisions::upsert(self.db, &key, &entry).await {
            tracing::warn!(url, %error, "could not record a cached revision");
        }
    }
}

#[async_trait]
impl<F: Fetcher> Fetcher for CachingFetcher<'_, F> {
    async fn get(&self, url: &str) -> SourceResult<Fetched> {
        let cached = self.cached(url).await;
        let known = cached
            .as_ref()
            .map(|entry| RevisionValidators {
                etag: entry.etag.clone(),
                last_modified: entry.last_modified.clone(),
            })
            .filter(|validators| validators.etag.is_some() || validators.last_modified.is_some());

        match self.inner.get_conditional(url, known.as_ref()).await? {
            ConditionalFetch::Fetched(page) => {
                self.remember(url, &page).await;
                Ok(page)
            }
            ConditionalFetch::NotModified => {
                // `known` was built from `cached`, and a fetcher only answers
                // `304` to a request that carried a validator, so this cannot be
                // reached with an empty cache. Reaching it anyway would mean the
                // page vanished without explanation, so it is handled as a miss
                // rather than as a panic in a request path.
                let Some(entry) = cached else {
                    return self.inner.get(url).await;
                };
                match self.body_for(&entry.checksum).await {
                    Some(body) => Ok(Fetched {
                        final_url: url.to_owned(),
                        body,
                        // The cached table does not keep the content type. It is
                        // not used to parse anything — every adapter reads HTML
                        // as text — and guessing would be worse than saying we
                        // do not know.
                        content_type: None,
                        etag: entry.etag,
                        last_modified: entry.last_modified,
                    }),
                    None => {
                        tracing::warn!(
                            url,
                            "the cached revision's bytes are gone; reading the page in full"
                        );
                        let page = self.inner.get(url).await?;
                        self.remember(url, &page).await;
                        Ok(page)
                    }
                }
            }
        }
    }

    async fn post_form(&self, url: &str, fields: &[(&str, &str)]) -> SourceResult<Fetched> {
        // Passed straight through. A form submission is a question aimed at the
        // source, and answering it from a cache would be answering a different
        // question than the one the adapter asked.
        self.inner.post_form(url, fields).await
    }

    async fn get_conditional(
        &self,
        url: &str,
        _known: Option<&RevisionValidators>,
    ) -> SourceResult<ConditionalFetch> {
        // The decorator is the layer that manages validators, so a caller asking
        // it conditionally gets the same answer as an ordinary read: either a
        // page or a confirmed-unchanged verdict, whichever the source gave.
        Ok(ConditionalFetch::Fetched(self.get(url).await?))
    }
}

/// The scope a read is filed under when no credential was used.
///
/// A constant rather than an empty string so the rows explain themselves, and so
/// a later scope can never collide with it by accident.
pub const PUBLIC_SCOPE: &str = "public";

/// Turn a fetch failure into something a caller can log without the body.
///
/// Kept here rather than inlined so the one place that decides what a cache
/// failure looks like is this module.
#[must_use]
pub fn describe(error: &SourceError) -> &'static str {
    error.category()
}
