# Bots, local software, and the Chromium extension

Status: **proposed, not adopted.** This is the plan for the three surfaces
outside the Lorehaven site itself that `docs/goal.md` puts in scope: the bot core
(M54), the local desktop software, and a Chromium extension that does not exist
yet. `docs/spec.md` §23.2 (amended, ADR 0024) is the authority for the bots. The
extension has no spec section yet — see *The extension needs a spec section first*.

Nothing in this file is adopted into `docs/spec.md` or `docs/requirements.csv`.
Confirm with the user before implementing anything derived from it.

## 1. The bots — M54, in the spec, build to spec

Two planned rows, both port rather than invention:

- **M54-01** — the platform-neutral fanfic-archivist bot core against
  `/api/v1`: Discord, Telegram, Matrix.
- **M54-02** — the link flow: a short-lived challenge, a Lorehaven-side pseud
  and scope confirmation, and revocable tokens.

Constraints, from spec §23.2 and the row notes, that are not negotiable:

- The bot **never touches the database.** It is a separate process and a separate
  repo. It speaks HTTP to the public API or it does not exist.
- Bots **never receive passwords.** The link flow is the only way in.
- Tokens are **revocable** and scoped, not permanent bearer credentials.

`docs/plans/metadata-exchange-triage.md` and `crates/lore-metadata` are the
precedent for "a consumer that compiles against a shared contract rather than
against the server." The bot is the same shape: a platform-neutral core, its own
crate or repo, depending on the wire types rather than the database.

**Exit condition:** a Discord, a Telegram and a Matrix adapter all pass the same
core test suite; revoking a token takes effect on the next call; no code path
reaches the DB.

## 2. The local software — already built, in another repo

`~/code-local/rust/lorebook`. Not a Lorehaven crate, and not to become one.

What it is: a Tauri desktop app — Rust core (`lorebook-core`, `lorebook-calibre`,
`lorebook-interop-check`) and a SvelteKit UI — that reads and writes **Calibre's
own `metadata.db` directly**, so there is no import step and no second copy of
the books. Its own plan, spec and provenance doc are in that repo:
`docs/PLAN.md`, `docs/SPECIFICATION.md`, `docs/CALIBRE-PROVENANCE.md`.

Where it stands: **M1 of M14**, built and verified at the library level, with one
unverified hop — the Tauri window does not paint in this environment, so
"webview loads the UI, IPC round trip" is untested. `docs/PLAN.md` M1.4 records
exactly what was and was not checked.

What this means for a Lorehaven implementer:

- Do not add Lorebook to the Lorehaven workspace.
- Do not merge the two repos.
- Do not add a dependency in either direction.
- Do not make Lorehaven require Lorebook.
- If a Lorehaven feature needs a desktop surface, it goes in the Lorebook repo
  and is a separate decision.

## 3. The Chromium extension — new work, no spec yet

Nothing exists: no `manifest.json` in any repo under `~/code` or `~/code-local`
(excluding `node_modules`) at depth ≤ 3. Prior art is `~/code/js/userscripts` —
14 Tampermonkey scripts, notably `ao3_download_buttons.user.js` and
`ao3_kudosed_and_seen_history.user.js`. Those show the *interaction patterns* an
extension would formalize. They are not the architecture: a userscript is
injected into one page at a time, an extension has a background service worker, a
storage area and a popup, and it can act on the whole browser.

### The extension needs a spec section first

A Manifest V3 extension is a security surface: it holds a user's credentials
between sessions, runs with origin-scoped host permissions, and can read and
write on pages the user never intended it to touch. Lorehaven's own privacy
posture (§0.3) is a ban on reading history, progress, ratings and pseud linkage
leaving the instance — an extension that proxies a logged-in session inherits
exactly the trust that §0.3 withholds.

So: write the spec section first. It should decide, before any code:

- what the extension stores locally, and what it never stores (a session token is
  a candidate for the OS keychain, not `chrome.storage.local`);
- which origins it requests, and the narrowest set that works;
- what it can read from a Lorehaven page, and what it refuses;
- whether it can act while logged out, and what it shows then;
- how a user revokes it — an extension with a stored token needs a one-click
  revoke that reaches the server, not just a local delete.

### Shape, once specced

- **Its own repo**, its own spec section, its own test story. Not a folder in
  the Lorehaven frontend.
- **Manifest V3**, TypeScript, built with the same toolchain as the Lorehaven
  frontend so there is one `pnpm` story.
- **A background service worker** for anything that must outlive a page, and a
  **popup** for the account binding. Content scripts only for the page-scoped
  conveniences that userscripts already prove people want.
- **The extension is a client of `/api/v1`,** the same surface the bots use. One
  API, several consumers — that is the shape `crates/lore-metadata` and M54
  were built for.
- **Tests:** unit tests for the storage and permission logic, and Playwright for
  the extension loaded via `--load-extension` against a real Lorehaven.

### Build order

Lorehaven `/api/v1` surface → bot core (M54) → extension.

The extension is last because it depends on the other two: it consumes the same
API as the bots, and its permission model should be informed by what the bots
proved is needed.

## What would make this plan wrong

- A decision that the extension ships as a userscript instead of an MV3
  extension — a real option, and it drops the service-worker and revoke-once
  questions rather than answering them.
- Lorehaven shipping a PWA that covers the desktop case, which would make
  Lorebook a convenience rather than the desktop surface.
- A user decision that the M45 block outranks the M11-17 block, which moves
  everything here by a milestone.
