# Handoff — the false clean gate, three real defects it hid, and a corrected E2E assertion

Date: 2026-09-25. Tip at write time: `9cd3c71`.
Previous full handoff:
`docs/handoffs/2026-09-25T134127+0200-metadata-exchange-specified-docs-synced-handoff.md`
— read that one for the metadata-exchange specification (spec §0.3, §2.3.1,
§11.17, §15.17, §16.16.1, §19.14; 5 `planned` rows; M57 build order) and the
outstanding verification debt. Nothing about that work changed here.

## The one thing to take from this session

**A gate I reported as passing was measuring nothing.**

Earlier today I told you `cargo clippy --workspace --all-targets` was clean, with
0 warnings and 0 errors, and that the 359-warning backlog was genuinely cleared.
That was false. The command was:

```
cargo clippy --workspace --all-targets 2>/dev/null | grep -cE '^(warning|error)'
```

rustc and clippy write warnings and errors to **stderr**. The `2>/dev/null`
discarded every one of them, the pipe read an empty stdout, and `grep -c`
returned a confident zero from no data at all. Re-measured with `2>&1` the tree
had **30 warnings and 0 errors**, including three real defects and one test that
had never executed.

The lesson generalises past clippy: *a count is only evidence if you know which
stream the tool writes to.* Any time a check has never failed — not once, across
every prior run — the first hypothesis should be the check, not the tree. This
one had been green for several sessions, which is exactly what should have made
me suspicious earlier.

## Three real defects, found by reading the warnings instead of counting them

| File | Defect | Why it mattered |
|---|---|---|
| `crates/db/src/media_resilience.rs` | `find_matching_standing_bounties` accepted a `media_reference_id` and never used it, while its doc comment promised "bounties that match a given media reference" | The table has no such column — standing bounties are global by design. The comment described a per-reference match the query could never perform. Parameter dropped at both call sites; comment now says what the function does. |
| `crates/db/src/longevity.rs` | `half_life_map` built its SQL placeholders with `if i == 0 { "" } else { "" }` — **both arms empty** | It emitted `?, ?, ?` and worked by accident. Replaced with the house `library::placeholders` helper every other module already uses. |
| `crates/db/src/media_resilience.rs` | `find_by_perceptual_hash` took a `max_distance` and ignored it | The Hamming-distance search is unimplemented. A caller passing a large threshold got exact matches and nothing else, while the signature promised a fuzzy search. Parameter renamed `_max_distance`; the doc comment now says outright that the threshold is not honoured and a result must not be read as "these are similar". Both callers pass `0`, so behaviour is unchanged. |

Plus a test that had never run: `domain::spoilers::test_display` had lost its
`#[test]` attribute, so its Display assertions were dead code. Clippy flagged it
correctly. My first reaction was to assume the attribute was already there and
add a second one — the fix was a duplicate. I reverted the file, which
accidentally discarded the real fix, and only caught it because the warning
persisted. Restored properly on the next pass.

The other 21 were mechanical: five `#[allow(clippy::too_many_arguments)]`
attributes a prior pass had detached from their functions and then deleted as
formatting nits, six blank lines between outer attributes and their items, two
never-read assignments, a manual `Default` impl replaced by
`#[derive(Default)]` with an explicit `#[default]` on `Plain`, four `&mut Vec`
parameters narrowed to `&mut [_]`, and unused imports in two test files.

One clippy suggestion was **rejected rather than applied**: collapsing the
nested `if let` in `discovery::resolve_sort` into a let-chain requires edition
2024, and `lorehaven-app` is edition 2021. The combined `Option` does the same
job. Check the crate's edition before accepting a syntax-level suggestion.

## A second false signal: the E2E export-delete test

`frontend/e2e/coverage.spec.ts` asserted `"No exports yet"` after deleting one
export. The E2E account is **shared** with the export-download test above it, so
that journey leaves a second `Ready` export behind and the empty state can never
appear. The delete was always working. The assertion described the account's
history rather than the feature, and failed on a correct build — which is how it
had been passing unnoticed as a real product bug.

The row could not be matched on its title either: the list renders
`job.label || job.format`, and a worker-produced export has no label, so its
heading is `EPUB`, not the work title. Filtering by title matched zero rows.

The fix captures the export id from the download `href` — the only stable handle
— before clicking Delete, then asserts that link is gone and the row count fell
by exactly one. The `Ready` wait stays: `Queued` is visible immediately, and
deleting mid-production asks the server to remove a row the worker still owns,
which reads as a failure to delete rather than as the race it is.

**When a UI test fails, check whether the assertion describes the feature or the
fixture.** A test that can only pass on a pristine account will fail forever on
a shared one, and the failure reads as a bug in the thing under test.

## Verification, all re-measured on this tree

| Check | Result |
|---|---|
| `cargo clippy --workspace --all-targets` (stderr captured) | **0 warnings, 0 errors** |
| `cargo test --workspace --no-fail-fast` | **1775 passed, 1 failed** |
| the failure, re-run in isolation | `repeated_login_attempts_are_rate_limited` — passes 1/1 in 59s. Known process-global rate-limit bucket flake under parallel test threads; **not** caused by this change |
| effective total | 1776 passing |
| `cargo fmt` scoped to the three touched crates | clean |
| `cargo build --release` | succeeded, 5m 10s |
| `npx vite build` | clean |
| `npx playwright test` (full suite) | **73/73 passing**, 8.1m |

Effective total is 1776 because the one failure passes alone; it is counted once
rather than twice.

## Commits this session

| Commit | What |
|---|---|
| `81bb104` | `fix(db,domain,app)`: the 30 clippy warnings, three of them real defects |
| `4ae4450` | `docs(verification,handoff)`: the correction and what it hid |
| `9cd3c71` | `test(e2e)`: assert the export's own row disappears, not an empty list |

All three pushed to `origin` (Forgejo) and `github`. Working tree clean. No
release tag — that is your call alone.

## Things not to trust

- **Any clippy or test count in this repo's older docs and session logs that
  pipes through `grep -c` without `2>&1`.** The `docs/sessions/*.md` files
  contain many "clippy 0 findings" lines from sessions that may have used the
  broken form. They are not re-audited; the current tree is clean because it was
  measured properly, not because history was.
- **Commit `bea1c2a`, "clear 359 pre-existing clippy warnings".** Same
  session, same failure mode. The 359 figure is not trustworthy; the 30 measured
  afterwards are.
- **Production on thinkcentre is `active` and answering 200, on an older
  commit.** These fixes are not deployed. Deploy means SSH to thinkcentre and
  building there, never through SSHFS.
- **A passing E2E that asserts an empty state or a global count on a shared
  account.** See above.

## Next, in order

1. **Deploy.** Nothing from this session is on production. The two
   user-visible changes are cosmetic-to-none (an ignored parameter, an accidental
   SQL placeholder that already worked), so there is no urgency — but the E2E
   fix and the clippy fixes should land before the next milestone's gate claims
   anything.
2. **M53-03/04, login adapters** — the next planned build per
   `docs/plans/remaining-work.md`.
3. **M54-01/02, bot port** (`fanfic-archivist-bot` as the port source, spec
   §23.2).
4. **M45 gaps**, 46 rows still open.
5. **Re-audit the `docs/sessions/*.md` clippy claims** if you want the record to
   be trustworthy rather than merely the current tree. Low priority; the
   historical claims are about history.
6. **The `max_distance` gap in `find_by_perceptual_hash` is a real feature
   hole**, not a lint. The Hamming-distance search does not exist. It is filed
   as a warning, not a bug, because no caller can currently reach it — but it
   should become planned work rather than a comment.
