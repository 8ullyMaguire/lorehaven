# Guest access, trust levels and self-governance — darkhold

Status: **proposal**, from the owner's request of 2026-10-16. It records intent in this repo's own
terms; nothing here is implemented. The estate-wide standard (items GS-1…GS-5, every setting
configurable, defaults and firewalls) is:

    secondbrain/10-Projects/2026-10-16-guest-access-trust-and-self-governance.md

This file is the site-specific half: what exists here today, what this site adopts, and the
firewalls that differ from the standard because this site is not the same shape as the others.

Stack: Rust platform + SvelteKit frontend (crates/app, frontend/).

## Where darkhold stands today (measured 2026-10-16)

- `karma` (99) and `trust_level` (52) already exist, with `moderator` / `is_moderator` roles — item 4's substrate is here
- consent appears only in a milestone test (`crates/app/tests/milestone_16.rs`); no guest identity, no fold

## What this site adopts

- GS-1 … GS-5, all of them; GS-4 is an upgrade of machinery that exists rather than new machinery

## Config deltas (everything else is the standard's default)

- `trust.axes` must be expressed in the existing karma terms, not a parallel score — two trust numbers that disagree is the failure mode to avoid
- `guest.mau_limit = 500`

## Firewalls specific to this site

- Reader-visible data (reading progress, bookmarks) is the guest's most valuable artifact and the most private: the fold must move it, and nothing may be *published* about a guest before they register.

## Order of work

GS-3 → GS-2 (progress/bookmarks are the reason to offer GS-1 at all) → GS-1 → GS-4 → GS-5. The
standard's §9 explains the ordering: consent first, because the guest identity *is* a cookie and the
gate has to exist before the thing it gates; the fold second, because offering guest access without
it creates work to clean up later.

## If this is implemented

Every step needs a test that fails without it, and the numbers belong in this file when they
are real — the estate's rule is that a claim of done carries the command that proves it. The
two items in the standard that nothing in this repo has yet are the elastic trust rule (§4)
and the jury drawn from the eligible pool (§5): both are cheap to fake and therefore need a
test that would pass for the wrong reason to be written, and killed, on purpose.
