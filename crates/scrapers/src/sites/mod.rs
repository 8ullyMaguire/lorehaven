//! One module per source.
//!
//! # Adding an adapter
//!
//! 1. Decide the source key. It is stored on every row that names the source,
//!    so choose it once.
//! 2. Record fixtures from the real site into
//!    `tests/fixtures/<source>/<case>.html`, at least: a work page, a whole-work
//!    page, an ongoing work, and the site's own "not found" page. **Record them
//!    first.** A parser written from memory of a site's markup is a parser
//!    written from a guess, and the guess is wrong in the one place that
//!    matters — the selector that silently matches nothing.
//! 3. Implement [`crate::SourceAdapter`], with `preview_from_html` and
//!    `chapters_from_html` so the fixtures can drive it.
//! 4. Register it in [`default_registry`].
//! 5. Add a fixture test in `tests/`.
//!
//! # The rule about requests
//!
//! An adapter is handed a [`crate::Fetcher`] and uses it. It does not build one,
//! does not hold one, and does not reach the network by any other route — see
//! the crate documentation for why that is a security boundary.

pub mod ao3;
pub mod royalroad;
pub mod syosetu;

use crate::{Registry, SourceAdapter};

/// Every adapter this build ships, with every source enabled.
///
/// # Panics
/// Never: every adapter's `can_handle` is exercised against its own hosts by the
/// test below, which fails if two adapters claim the same URL.
#[must_use]
pub fn default_registry() -> Registry {
    let mut registry = Registry::new();
    registry.register(Box::new(ao3::ArchiveSoftware::new()));
    registry.register(Box::new(royalroad::RoyalRoad::new()));
    registry.register(Box::new(syosetu::Syosetu::new()));
    // An adapter that cannot read anything must not appear in the catalogue: a
    // source listed as available and answering "not implemented" is worse than
    // one that is absent, because the first wastes a reader's time and the
    // second does not. Adapters register here as they land, and the milestone
    // notes list the sources still to come.
    // in the same commit that makes it work.
    registry
}

/// The hosts every registered adapter may fetch from.
///
/// Used by the import to build a fetcher whose allow-list is the source's own
/// hosts, so a page cannot direct the importer to an unrelated origin.
#[must_use]
pub fn hosts_of(registry: &Registry, key: &str) -> Vec<String> {
    registry
        .by_key(&crate::SourceKey::new(key))
        .map(SourceAdapter::hosts)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    #[test]
    fn the_default_registry_is_not_empty() {
        let registry = default_registry();
        assert!(!registry.adapters().is_empty());
        let catalogue = registry.catalogue();
        assert!(catalogue.iter().all(|entry| entry.enabled));
    }

    #[test]
    fn no_two_adapters_claim_the_same_url() {
        // The registry routes by asking each adapter in turn, so an overlap is
        // not an error — it silently makes one adapter unreachable. This catches
        // the copy-and-forget case where a new adapter's `can_handle` was not
        // narrowed.
        let registry = default_registry();
        let sample = [
            "https://archiveofourown.org/works/1",
            "https://adastrafanfic.com/works/1",
            "https://squidgeworld.org/works/1",
        ];
        for raw in sample {
            let url = Url::parse(raw).unwrap();
            let claimants: Vec<String> = registry
                .adapters()
                .iter()
                .filter(|adapter| adapter.can_handle(&url))
                .map(|adapter| adapter.key().to_string())
                .collect();
            assert_eq!(claimants.len(), 1, "{raw} claimed by {claimants:?}");
        }
    }

    #[test]
    fn every_adapter_declares_a_key_and_capabilities() {
        for adapter in default_registry().adapters() {
            let key = adapter.key();
            assert!(!key.as_str().is_empty());
            let capabilities = adapter.capabilities();
            assert!(
                capabilities.metadata || capabilities.chapters,
                "{} claims to do nothing",
                key
            );
        }
    }

    #[test]
    fn an_adapter_without_hosts_is_reported_as_empty_rather_than_panicking() {
        let registry = default_registry();
        assert!(!hosts_of(&registry, "ao3").is_empty());
        assert!(hosts_of(&registry, "does-not-exist").is_empty());
    }
}
