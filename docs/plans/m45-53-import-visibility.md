# M45-53 — Import visibility: metadata public, cached body private

Spec and plan, 2026-10-05. **Conclusion first: most of this requirement is already satisfied by
the existing design, and the correct action is to prove it and close it — not to build a
parallel visibility system.** Building one would be the same mistake as M45-51's 71-table list.

Row: `docs/requirements.csv` `M45-53`, status `planned`, notes "Gaps review G3. Biggest
reputational and copyright risk; verify with counsel."

## 0. What was measured, and what it changed

The requirement reads: *"public sees metadata and link, cached body private until author
claims."* Three claims, each checked against the tree rather than assumed.

### Claim 1 — "cached body private". **Already true, and enforced at the query, not the route.**

`db::reader_body_copies::body_for(db, work_id, account_id)` takes the account as a parameter
and filters on it:

```sql
WHERE work_id = ?1 AND account_id = ?2 AND state = 'ready' AND plain_text IS NOT NULL
```

The function's own doc comment states the reasoning: *"The account comes from the caller's
session, never from a parameter the route chose. A signature with a `work_id` and nothing else
means there is no argument for a route to get wrong."*

And the test already exists — `a_reader_cannot_read_another_readers_copy` in
`crates/app/tests/reader_body_copies.rs` — with the comment **"VERIFIED BY INJECTION before
this work was called done — the plan requires it, and a privacy test that has never failed is
not evidence."** It settles alice's copy with the literal text `ALICE'S EXCLUSIVE TEXT`, has
bob read the same work, asserts `404`, and asserts the bytes appear nowhere in his response.

That test is the strongest evidence in this repo that the requirement is met, and it predates
the requirement being written. **The privacy half of M45-53 is not planned work; it is
already-shipped work.**

### Claim 2 — "metadata private to the owner". **Also true.**

Every `library_items` read in a route takes the account from the session, never from a path or
query parameter:

| call site | account comes from |
|---|---|
| `routes/library.rs:260,284,299,333,…` | `user.account_id.to_string()` (`RequireSession`) |
| `routes/imports.rs:1451` (`list_library`, `#[allow(dead_code)]`) | `user.account_id.to_string()` |
| `routes/imports.rs:703` → `find_duplicate` | `user.account_id.to_string()` |

`grep -rln MaybeSession crates/app/src/routes/library.rs` → **nothing**. There is no
visitor-reachable library route at all.

`library_items` has **no visibility column** — not `is_public`, not `visibility`. I checked the
whole migration set. So there is no visibility *default* to get wrong, which is what the
requirement is nominally about.

### Claim 3 — "public sees metadata and link". **FALSE, and this is the real gap.**

There is no public metadata surface for an imported work. `source_url` is returned only by
session-scoped routes (`imports.rs:708,1328,1479`, `library.rs:672`). No visitor can see an
import's title, author or source link.

**And that is correct.** M45-53 was written from the perspective of a *federated* instance
where a reader's import might surface publicly. This instance's model is **per-reader private
library copies** — `library_items` is scoped to `account_id` with no publication path, and
`import_chapters.content_blob_checksum` holds the cached body privately. Publishing a stranger's
import list is not a missing feature; it is the thing this schema was designed to prevent.

## 1. So what is the actual work?

**Close the row, with the gap recorded rather than papered over.** Specifically:

1. The requirement's premise — that an import *can* be public — does not hold on this instance.
   That is a fact about the design, not a gap to close, and the row should say so.
2. The one thing genuinely missing is a **standing test that no visitor-reachable route can
   read `library_items`**. Right now the privacy property is enforced by every route *happening*
   to take the session account, and nothing fails if a future route takes an id from the path
   instead. That is the same failure mode as the `is_public`-dropped-from-a-query bug this
   project has shipped four times: an invariant held by convention, not by a gate.
3. `verify with counsel` is **not something code can discharge.** It stays open in the notes
   and is not marked done on my authority.

## 2. The work: a static gate, plus one behavioural test

### 2a. `scripts/check-library-visibility.py` — the static gate

Reads `crates/app/src/routes/*.rs` and fails if any handler that reaches `library_items` or
`reader_body_copies` obtains its account from anything other than a session.

This is the same class of gate as the existing `check-page-headings.py`, and it has the same
honest limitation, which the script must **report rather than hide**: a regex cannot decide
whether `account_id` on line 700 came from the session or from a path parameter two frames up.
So the gate's job is to find every candidate and assert the *shape* — and the script prints
what it could not prove.

**Its self-test is 6 cases and must run in CI**: three that must be caught (account from a path
param, from a query param, from a header) and three that must not (session, operator-scoped
with an explicit audit note, and a store function that takes no account at all).

### 2b. `crates/app/tests/import_visibility.rs` — the behavioural proof

- A visitor with **no session** gets 401/404 from every library route and never sees an item.
- Reader A's item id, requested by reader B, is not found — the id being *unguessable* is not
  the property; the property is that ownership is checked.
- The cached body text never appears in any response a third party can obtain.

## 3. Gates

- **Both engines** on the behavioural test.
- **The static gate's self-test must go red when a planted violation is added**, then the
  planted violation is removed and it goes green again. A gate that has never failed is a gate
  that does not work — the same standard applied to `check-page-headings.py`.
- **No new column, no new table, no new route.** If the implementation needs any of those, this
  spec is wrong and that is the signal to stop and re-read the requirement.

## 4. File layout

| file | what |
|---|---|
| `scripts/check-library-visibility.py` | the static gate + its 6-case self-test |
| `crates/app/tests/import_visibility.rs` | behavioural proof, both engines |
| `docs/requirements.csv` | M45-53 → closed, notes rewritten, counsel caveat kept |

## 5. Steps, with the command and expected output for each

**Step 1 — write the gate and fail it first.**
`python3 scripts/check-library-visibility.py --self-test` → 6 cases, **6 passed**.
Then plant a violation (change one route to take `account_id` from a path param) and re-run →
**exactly 1 red**, naming the file and line. Remove the plant, re-run → 6 passed.

**Step 2 — run the gate on the tree.**
`python3 scripts/check-library-visibility.py` → 0 violations. If it reports any, that is a real
finding about the current code, not a gate bug; investigate before continuing.

**Step 3 — the behavioural test.**
`cargo test -p lorehaven-app --test import_visibility` → all pass.
`LOREHAVEN_TEST_PG_URL=... cargo test -p lorehaven-app --test import_visibility -- --test-threads=1`
→ all pass. **`--test-threads=1`** is required on PostgreSQL; at 2 the concurrent migration
replays exhaust `/dev/shm`.

**Step 4 — mutation.**
Make the gate's session pattern match something it should not (e.g. accept any `*account_id*`
identifier) and confirm it goes red on the planted case. Restore byte-identical by `diff`.

**Step 5 — full verification.**
`cargo test --workspace -- --test-threads=2` → 0 failed (SQLite).
`cargo test --workspace --no-fail-fast -- --test-threads=1` → 0 failed (PostgreSQL).
`cargo clippy --workspace --all-targets` → 0. `cargo fmt --check` → clean. svelte-check → 0.

**Step 6 — CI wiring, and the tracker.**
Add the gate to the CI list beside `check-page-headings.py`. Set M45-53's status with an
`evidence` field citing `scripts/check-library-visibility.py` and
`crates/app/tests/import_visibility.rs` — **paths, not plan documents**, per the probe rule in
`100-ideas-remaining.md`. Keep `verify with counsel` in the notes, unresolved.

## 6. What this spec deliberately does not do

- **No publication toggle.** Adding `is_public` to `library_items` would create the exact
  surface M45-53's authors were worried about, on the strength of a requirement written for a
  different data model. If publication is wanted later it is a design decision with its own
  spec, not a default value.
- **No claim flow.** "until author claims" implies an author asserting ownership over an
  imported work. `story_identity` and `preservation` have the vocabulary for it
  (`record_crossposted_location`, `IdentityMember`), and neither is a claim mechanism. That is a
  feature, not a default, and it is **out of scope here** — noted so it is not mistaken for
  something this work completed.
