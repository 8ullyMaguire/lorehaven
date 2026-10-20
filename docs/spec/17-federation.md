# 17. Federation and interoperability

**Status:** reserved
**Milestone:** M25
**Source:** the corresponding section of the Darkhold specification

## 17.1 OPDS

Every catalogue surface is an OPDS feed: new, popular, by tag, by theme, by channel, by author, by
search, by shelf, and **recommendations** (the OPDS recommendations endpoint, driven by the taste
service). A reader's e-reader app is a first-class client.

## 17.2 Atom and RSS

New arrivals, per-work, per-follow, per-saved-search, per-tag, per-channel, per-mood, and the
forum's feed. Every feed is a query with a revocable token.

## 17.3 JSON-LD and Dublin Core

Every public page carries Schema.org `CreativeWork` JSON-LD and Dublin Core metadata.

## 17.4 ActivityPub

Public, **opted-in** pseud actors; follow/unfollow; work announcements; updates; deletion notices.
Signature verification, retry, replay and abuse controls, SSRF defences, a remote actor cache,
instance blocks, and opt-out documentation.

**Never federated:** private imports, reading history, hidden account linkage, private ratings,
held comments, and — **the instance theme's private seed** (the theme's *published* form may
federate as an announcement; its working seed may not).

The documentation says plainly that **remote copies may persist after a deletion notice** — a
federated delete is a request, not a guarantee.

**The one rule with a stated reason:** federated *queries* are refused. "Remote queries would be
a scraping vector wearing a protocol."

## 17.5 Cross-instance discovery — deferred, with the shape fixed

Cross-instance search, cross-instance wishlists, shared tag canonicalisation (a peer's
canonicalisation is a suggestion with provenance, applied by the instance's own quorum, never
imposed), instance trust relationships, and transfer manifests between instances.

**"Identity, provenance and permission travel with federated records, and no instance's policy,
moderation decision or canonicalisation is overridden by a peer's."**

---
