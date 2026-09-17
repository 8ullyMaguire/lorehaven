---
title: "Building Lorehaven"
subtitle: "A complete implementation tutorial: from an empty directory to a running fanfiction platform"
author: "Lorehaven contributors"
lang: en
rights: "Internal technical documentation"
---

# Building Lorehaven

A complete implementation tutorial: from an empty directory to a running
fanfiction platform.

Lorehaven is a self-hosted home for fanfiction: read without an account, write
without a publishing queue, import the library you already have, and take it
offline when you want it. It is built as one Rust binary — API, frontend, worker
and schema — with SQLite and PostgreSQL supported from the same code.

This book is for a developer who already knows the stack (Rust, Axum, SQLx,
Svelte, TypeScript) and wants to build the application. It teaches the order to
build in, the rules that must not be broken, the mistakes that are waiting, and
how to prove each part works.

## What you will build

| Part | What it covers | Checkpoint |
|---|---|---|
| 1 | Foundations: workspace, config, migrations, errors, assets | `v0.01-running-app` |
| 2 | Accounts, pseuds, sessions, privacy, age policy | `v0.03-identity` |
| 3 | Works, chapters, revisions, publishing | `v0.04-publishing` |
| 4 | The reader: ratings, reviews, notes, history, goals | `v0.05-reader` |
| 5 | Jobs, blob storage, secrets, the worker | `v0.06-jobs` |
| 6 | The positivity filter and feedback delivery | `v0.08-positivity` |
| 7 | Imports: fetching, adapters, sanitising, shelf CSVs | `v0.07-importing` |
| 8 | Library, shelves, taxonomy, the query language, search | `v0.11-search` |
| 9 | Discovery, exports, offline reading | `v0.12-discovery` |
| 10 | Comments, forums, groups, messaging, events | `v0.13-community` |
| 11 | Trust, governance, credits, fair queues | `v0.15-governance` |
| 12 | Marketplace, extensions, webhooks, themes | `v0.17-marketplace` |
| 13 | The generalized media platform: editions, derivatives, lending, narration | `v0.22-media` |
| 14 | Integrations: translation, public API, feeds, push, federation | `v0.19-integrations` |
| 15 | Operations, hardening and release | `v1.0-release` |
| Appendix | The patterns used in every part | — |

## The three rules this book will not let you break

1. **Completion means working behavior.** A feature is done when a real request
   produces a real result that a test asserts.
2. **404 over 401 for private objects.** Never confirm the existence of
   something the caller may not know about.
3. **Build the rule with the feature, and prove it with a test.** The privacy,
   age and visibility rules are not a later pass.

Everything else in the book follows from those three.

## How the reference implementation is described

Every file path, module, migration and command named in this book exists in the
reference implementation, and each part ends with the tests that prove it. Where
the reference implementation is incomplete, the part says so rather than
describing a feature that does not work — an honest gap is worth more than a
plausible paragraph, and it is the only thing that lets the next developer pick
up where you stopped.
