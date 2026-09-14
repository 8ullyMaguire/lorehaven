# ADR 0018: AI crawler and scraping posture

- Status: accepted
- Date: 2026-09-14

## Context

An instance serves pages to people and to search engines that send readers.
It does not serve free bulk text to anyone else by default. AI-training
crawlers are the bulk text consumers of record.

## Decision

- `robots.txt` is **generated, not static**: public eligible content declared
  to well-behaved crawlers, AI-training crawlers disallowed by default, the
  disallow list is operator configuration with the default stated in the
  operator docs.
- **No bulk text endpoints** exist — no export, feed, scope or sitemap
  variant returns complete bodies at volume to an anonymous or broadly-scoped
  caller.
- A work carries an author-set `ai_training` assertion — allow, deny, unset —
  shown as metadata and exported with the work. The instance enforces what it
  can enforce (robots defaults, no bulk endpoints, API terms) and describes
  the assertion honestly as "this author's stated preference, recorded",
  never as "protected".
- Scraping for AI training is not an accepted use of the public API or public
  pages; access granted to a client that does it is revoked. This is a
  terms-and-abuse statement enforced through §24.4–24.5, not a technical
  guarantee, and both halves are stated.

## Consequences

- The robots generator is operator-configurable; its defaults live in the
  operator docs and are tested.
- The `ai_training` column lands on `works` (migration 0022) and exports in
  §13.1 carry it.
- Refusal of AI-training crawlers is not claimed as a technical guarantee.
