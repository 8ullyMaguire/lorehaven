# Where the CLI and TUI live

**Decision: in this workspace, as `crates/cli` (binary `dh`), with the TUI as `dh tui` in the same
binary. Not in `lorebook`. Not in a fourth repository.**

Written after the power-user sweep (`docs/ideas/2026-11-01-idea-sweep-power-users.md`), which put
93 ideas behind one verb: a recommendation command where the user picks the fandom, the time
window, the engine and the quality metric. This document answers the placement question that
sweep raised, because the answer changes what gets built.

## The rule the family already runs on

Look at what each product owns, and the question answers itself. Every product in this estate owns
**exactly one kind of durable data**, and everything else is either an HTTP client of `/api/v1` or
a file a person carries between instances.

| Product | Owns | Never owns | Talks to Darkhold by |
|---|---|---|---|
| **darkhold** | the instance: catalogue, works, readers, ranking, federation | local ebook files; a chat session; a fetch cache | — (it is the server) |
| **lorebook** | a local Calibre library (`metadata.db` + files on disk) | any catalogue, ranking or account data | *nothing today* — and that is deliberate |
| **darkbot** (`lorebot` on disk) | a chat session and a link table | a database, works, vectors, any copy of anything Darkhold stores | `POST /me/chat/actions`, scoped token |
| **ficnexus** | a fetch/build cache | accounts, bookmarks, feeds, recommendations | not at all: separate service, separate job |
| **ascend** | builds/decks | works, readers | a **file**: a seed out, a URL claim in (`docs/NETWORK.md`) |

Two rules fall out of that table, and both are already written down:

- `docs/plans/bots-extension.md` §2 on Lorebook: *"Do not add Lorebook to the Darkhold workspace.
  Do not merge the two repos. Do not add a dependency in either direction. Do not make Darkhold
  require Lorebook. If a Darkhold feature needs a desktop surface, it goes in the Lorebook repo."*
- `docs/plans/bots-extension.md` §1 on the bot: a consumer **compiles against a shared contract**,
  never against the server (`crates/metadata` is the precedent).

A CLI is the second rule, not the first. It owns no durable data beyond a cache; it reads the
catalogue, the library, the recommendations and the discovery surface over `/api/v1`. Its natural
home is next to the contract it consumes.

## Why not Lorebook

Lorebook is a *library manager*: it owns the books, opens Calibre's `metadata.db` directly, and its
jobs are import, scan, parse, convert, and Tauri window. A `dh rec --fandom naruto --updated '>7d'`
verb mutates none of that. Putting it in Lorebook would mean Lorebook reimplementing the
recommendation client — the same HTTP calls, the same window grammar, the same saved queries — in a
repository that versions independently of the API it is tracking. The first time `/api/v1` changed,
the two would drift, and the drift would be silent because nothing in Lorebook's test suite talks
to a Darkhold instance.

There is a sharper reason too. The existing doctrine says a Darkhold feature that needs a *desktop
surface* belongs in Lorebook. A terminal is not a desktop surface: `dh` is not a GUI over Darkhold's
own screens, it is a query tool over the catalogue. The plot is thin here, and pretending
otherwise is how a "small helper" becomes a second client with its own release cadence.

## Why not its own repository either

The tempting case for a separate `dh` repo is a community client somebody else could adopt. The
counter-argument is the one that matters at this scale: **the CLI is the cheapest place to notice a
breaking API change**, and that only works if it is built and tested with the server. In-workspace,
`cargo test --workspace` fails on the day a route's shape changes. In a separate repo, the first
person to notice is a user at 23:00.

Two cheap guarantees make in-workspace acceptable despite the coupling risk:

1. **`crates/cli` may not depend on `crates/app` or `crates/db`.** It depends on the wire types
   (`crates/metadata`, `crates/domain`) and `reqwest`; if it needs more than that, the contract is
   missing a type and the type belongs in `crates/metadata`, not in an import.
2. **Its tests are contract tests.** One set runs against a router built in-process (no network),
   proving serialisation and error mapping; one set runs against the SQLite snapshot for `dh q
   --local`. Neither set reaches a database directly.

That boundary is what a separate repository would have had to invent as a versioning scheme. A
Cargo dependency edge is a stricter and cheaper version of the same promise.

## Why the TUI is a mode, not a project

`dh tui` is the same binary with a full-screen renderer (ratatui behind a `tui` feature). The
verbs, the window grammar, the saved queries and the snapshot cache are identical; only the output
layer differs. A separate TUI project would duplicate the parser, which is precisely the fork the
window grammar exists to prevent (`docs/ideas/2026-11-01-idea-sweep-power-users.md`, "The window
grammar").

## The one place the two products meet, and how

Not a crate dependency: a **file**. `GET /api/v1/catalogue/export.sqlite` produces a read-only
SQLite snapshot of the catalogue with no reader in it (implemented; `crates/db/src/catalogue_export.rs`).

- `dh q --local` queries that file, so a power user can run SQL on a train.
- Lorebook could open the same file tomorrow, because it is already a program that opens a
  SQLite metadata database and joins it to files on disk. It would gain a catalogue overlay
  without importing a line of Darkhold code.
- In the other direction, `dh lib --calibre <path>` reads Calibre's `metadata.db` **as a data
  source** to seed a reading history. Reading a documented file format is not a dependency; a Cargo
  edge or an RPC call to Lorebook would be.

Both directions are "same file format, no link", which is the Ascend precedent applied to a database
instead of a markdown seed.

## What would make this wrong

- **Lorebook grows a Darkhold sync mode.** If it starts calling `/api/v1` for catalogue or
  recommendations, then there are two clients and the split argument reopens.
- **A verb that needs the local library and the remote catalogue in one result.** `dh lib` merging
  a Calibre shelf with a server shelf is genuinely one tool over two stores; at that point the CLI
  is a library manager's companion and the case for living beside Lorebook gets real. Today nothing
  in the sweep needs it.
- **Darkhold ships a PWA that covers the desktop.** Then Lorebook is a convenience rather than the
  desktop surface (`bots-extension.md` already lists this as a way its own plan could be wrong),
  and the CLI stays where the API is.
- **Someone outside this estate adopts `dh`.** A third-party client is a different project with a
  different contract; this decision is about the first-party tool.

## Build order

1. Window grammar parser beside the wire types, used by both the route and the CLI (the sweep's
   constraint 1: one parser, no fork).
2. `crates/cli` with `dh rec`, `dh q`, `dh export` — the three verbs that need nothing but the API
   plus the snapshot.
3. `dh q --local` against the snapshot; `dh bench` for the speed story.
4. `dh tui` last, because it needs everything above to exist before it has anything to lay out.
