# How to use this book

You are going to build Lorehaven: a self-hosted home for fanfiction where a
reader can read without an account, a writer can publish without a queue, and
either of them can take their library offline when they want it.

This book assumes you already know the stack — Rust, Axum, SQLx, SQLite and
PostgreSQL, Svelte, TypeScript, HTTP, and how to write a test. It does not teach
you those. It teaches you **this** application: its rules, its shape, the order
to build it in, and the mistakes that are waiting for you.

## What you are building, in one paragraph

One Rust binary. It serves the API, serves the compiled frontend, runs a worker
that does slow work (imports, exports, OCR, transcoding, narration), and carries
its own database schema with it. There is no Node process in production and no
separate frontend deployment. Two database dialects are supported from the same
code: SQLite for a person on a laptop, PostgreSQL for an instance with more than
one reader.

## The rule this book follows

The specification this project was built from has one rule that shapes
everything else:

> Completion means working behavior. A feature is done when a real request
> produces a real result that a test asserts. Not when the code compiles, not
> when the route exists, not when the happy path works by hand.

So every part of this book ends with tests that exercise the real router against
a real database, and with an honest statement of what is verified and what is
not. If you follow along, you will never be in a position where you have to
guess whether something works.

## How the parts are organised

Each part is one vertical slice: database, domain rules, HTTP doors, frontend,
tests. You can finish a part and have a working thing. Parts build on each other
in order — the checkpoint tags at the end of each part name the commit you
should be at before starting the next.

| Part | Builds | Checkpoint after |
|---|---|---|
| 1 | Foundations: workspace, config, migrations, errors, assets | `v0.01-running-app` |
| 2 | Accounts, pseuds, sessions, privacy, age policy | `v0.03-identity` |
| 3 | Works, chapters, revisions, publishing | `v0.04-publishing` |
| 4 | Reader, ratings, reviews, notes, history, goals | `v0.05-reader` |
| 5 | Jobs, blob storage, secrets, the worker | `v0.06-jobs` |
| 6 | The positivity filter and feedback delivery | `v0.08-positivity` |
| 7 | Imports: fetching, adapters, sanitising, shelf CSVs | `v0.07-importing` |
| 8 | Library, shelves, saved views, taxonomy, search | `v0.11-search` |
| 9 | Discovery, exports, offline reading | `v0.12-discovery` |
| 10 | Comments, forums, groups, messaging, events | `v0.13-community` |
| 11 | Trust, governance, credits, fair queues | `v0.15-governance` |
| 12 | Marketplace, extensions, webhooks | `v0.17-marketplace` |
| 13 | The generalized media platform: editions, derivatives, lending, narration | `v0.22-media` |
| 14 | Operations, hardening, release | `v1.0-release` |
| Appendix | The patterns you will use in every part | — |

## Every part has the same shape

1. **Checkpoint** — where you should be starting from.
2. **What will work by the end** — the commands you will be able to run.
3. **Concepts** — the ideas this part is actually teaching you.
4. **Commands** — what to type.
5. **Exact file changes** — what appears on disk.
6. **The code that matters** — the parts worth reading carefully, and why.
7. **Tests** — what is asserted and what that proves.
8. **Expected UI behaviour** — what you should see in a browser.
9. **Troubleshooting** — the failures you are most likely to hit.
10. **Consequences** — the privacy, safety or operational cost of what you built.
11. **Checkpoint** — the tag to leave behind, and what is still owed.

## Two habits that will save you

**Write the door, the rule and the test together.** The most common way a
project like this goes wrong is that the route gets written, the happy path
works, and the rule it was supposed to enforce (who may read this? what happens
to a private draft?) is discovered later — by someone else, in production.

**Prefer 404 to 401 when the object is private.** If a caller asks for something
they are not allowed to know exists, do not tell them it exists and refuse them.
Tell them it does not exist. This is a project-wide rule, and Part 1 shows you
where it lives in the code.

## A note on the ordering of parts 6 and 7

You may notice the parts do not follow the specification's milestone numbering
exactly: the positivity filter (Part 6) is built before the importers (Part 7)
even though importing is milestone 6 and the filter is milestone 7. The reason is
that imported text has to pass through the filter, so the filter has to exist
first. Every project has a couple of dependencies like this. Find yours early,
and write them down.
