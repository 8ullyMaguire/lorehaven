# Handoff — M32-07a perceptual dedup implemented; the column nothing populates

Date: 2026-09-25. Tip at write time: `fd08762` + this docs commit.
Previous full handoff:
`docs/handoffs/2026-09-25T142500+0200-false-clean-gate-three-defects-e2e-assertion-fixed-handoff.md`
— that one records the false clippy gate and the E2E assertion fix. Read it
first if you want the reasoning; nothing in it is invalidated here.

## What shipped

**M32-07A, spec §32.7.2 — threshold-based perceptual deduplication.** Two
commits, `3346144` (backend + config) and `fd08762` (admin UI).

A clippy warning from the previous session had flagged
`find_by_perceptual_hash` as accepting a `max_distance` it never used. It was
correct: the function's signature promised deduplication it did not perform, and
its doc comment said so. This implements it rather than documenting the gap.

| Piece | Where |
|---|---|
| `hamming_distance`, `perceptual_match_confidence`, `is_within_perceptual_threshold` | `crates/domain/src/media_resilience.rs` |
| `PerceptualHashAlgorithm`, `AudioFingerprint` vocabularies | same file |
| Hamming-distance search, closest first | `crates/db/src/media_resilience.rs` |
| Threshold from config, per-match confidence in the response | `crates/app/src/routes/discovery.rs` |
| `[media_resilience]` wired to TOML for the first time | `crates/app/src/config.rs` |
| Match kind, similarity, safe-to-link badges | `frontend/src/routes/AdminMirror.svelte` |

## The judgement calls, in the order they were made

**Distance in Rust, not SQL.** Neither backend can score a hex string without a
dialect-specific extension, and two dialects kept in step is worse than one
shared code path. The query narrows to `perceptual_hash IS NOT NULL` — the one
predicate worth the existing index — and the distance is computed in Rust.

**`NULL`, non-hex, and mismatched-width hashes return nothing.** A `NULL` is a
reference whose bytes were never fetched, not a hash to compare. `not-a-hash` is
not a fingerprint. A 64-bit pHash and a 128-bit wHash have no distance between
them. Each would otherwise have to fold into a large distance, which is a
similarity verdict invented from a malformed value.

**A malformed row is skipped, not fatal.** One bad row must not hide every good
match. Pinned by `a_malformed_stored_hash_does_not_match_and_does_not_fail_the_search`.

**Ties break on id.** Closest first is what a curator reads top-down, and a
stable tiebreak means a paginating caller cannot loop.

**The confidence score is linear across the full 64-bit width, and is named as
what it is.** pHash agrees across re-encodes and disagrees across distinct images
that happen to share structure. The doc comments say the score measures
fingerprints, not artworks — which is exactly why the spec routes the decision
through a curator.

**An unknown algorithm or out-of-range threshold stops the instance.** Both fail
silently otherwise: a threshold of 64 matches everything, and a typo'd algorithm
deduplicates with a hash it never computes.

## Three defects found while implementing

Not found by the new feature's tests — found by noticing what the feature needed
and was not there.

1. **`perceptual_match_threshold_is_valid` was dead code.** A tested, exported
   domain validator for a threshold no config could set. It is now called from
   `Config::validate()`.

2. **`MediaResilienceConfig` had no TOML surface.** Seven fields, no
   `[media_resilience]` section, so every value was permanently the default and
   `deny_unknown_fields` could not see the table. The seven curator-credit and
   health-check keys still have no section field — they keep their long-standing
   defaults, and that is now stated in the code rather than left ambiguous.

3. **`load_from()` in the config tests could read a stale file.** It keys its
   scratch directory on `(pid, name)` and never clears it. My two "refuses
   nonsense" tests inherited the passing parse test's directory and loaded a
   valid config with `whash` and a threshold of 9 while asserting rejection. The
   helper now removes the directory first, and the call sites have distinct
   names.

A fourth was in the test suite, not the code: `crates/app/tests/discovery.rs`
stored the string `hash123` as a perceptual hash. The new code correctly refuses
it. The fixture was fiction and is now a real fingerprint.

## Verification, all measured on this tree

| Check | Result |
|---|---|
| `cargo clippy --workspace --all-targets` (stderr captured) | **0 warnings, 0 errors** |
| `cargo test --workspace --no-fail-fast` | **1733 passed, 0 failed** across 92 binaries |
| `cargo test -p lorehaven-domain --lib` | 495 passed |
| `cargo test -p lorehaven-app --test media_resilience` | 17 passed |
| `cargo test -p lorehaven-app --test discovery` | 3 passed |
| `npx svelte-check --threshold error` | **0 errors, 0 warnings** |
| `npx vitest run` | 59 files, **308 passed** |
| `npx vite build` | clean |
| `npx playwright test` | 73/73 after the E2E fix below |

The `repeated_login_attempts_are_rate_limited` rate-limit flake did not fire this
run; it remains the known flake documented in the skill.

## Two things about the gates themselves

**vitest does not typecheck.** Four required fields were added to
`api.ReverseSearchReference`, and all 308 vitest tests stayed green. svelte-check
is what caught it — "missing the following properties from type
'ReverseSearchReference'" across 4 fixtures in 2 files. A new vitest case is
therefore not on its own a proof of a type contract; it pins the client's
handling of a shape, not the shape.

**The E2E export-delete test broke again, and this time the bug was mine from
this morning.** The previous handoff's fix took `pathname.split('/').pop()` and
called it the export id. The href is `/api/v1/exports/{id}/download`, so that
last segment is the literal string `download` — it matched every download link on
the page, and the assertion demanded 0 while two legitimate links remained. It
passed in isolation, because with one row the wrong locator still matches one
element, and failed in the full suite. It now compares the full `href` and
asserts the shape (`toContain('/api/v1/exports/')`, `endsWith('/download')`) so
the assumption is checked rather than assumed.

The generalisable form: **a locator that matches everything passes when only one
row exists.** Any test that grabs a per-row handle should assert the handle is
unique before acting on it, or it will pass in isolation and fail in a suite.

**A separate lesson, same morning, same shape: `hash123` was stored in a
perceptual-hash column.** When a test breaks after a correctness fix, check
whether the fixture was the thing that was wrong before touching the fix. Both
of this session's test failures were fixtures that had been papering over a
defect — one in the test data, one in the test's own locator.

## Next, in order

1. **M32-07B — nothing computes a perceptual hash.** The search and the config
   are correct; the column is only ever written by a test, so the feature is not
   reachable from the site. Blocked on a real dependency: image decoding is not
   in this workspace, so fingerprinting image bytes is a new crate rather than
   wiring. `audio_fingerprint` needs the same for audio. Filed in
   `docs/requirements.csv` with that reasoning.
2. **The curator merge workflow (spec §32.7.2).**
   `require_curator_confirmation_below` and `require_curator_confirmation_above`
   are parsed and validated but nothing reads them, because there is no merge
   path. Do it after 1 — a merge door around an empty column is a workflow that
   cannot be exercised.
3. **M53-03/04, login adapters** — the next planned build per
   `docs/plans/remaining-work.md`, unchanged by this session.
4. **M54-01/02, bot port.** Then **M45**, 46 rows.
5. **Deploy.** Nothing from either of this session's commits is on production.
   The tree is green and E2E passes, so this is a reasonable point to deploy if
   you want it live; nothing here is urgent to ship, since the change is
   additive and unreachable from the site until item 1 lands.

## Things not to trust

- **That the dedup feature works end to end.** It does not, and cannot: nothing
  populates the hash. `implemented-locally-tested` on M32-07a means the search,
  the scoring and the config are correct and tested — not that the site
  deduplicates anything.
- **Commit `bea1c2a`'s "359 clippy warnings cleared"**, and the older
  `docs/sessions/*.md` counts that pipe through `grep -c` without `2>&1`. The
  current tree is clean because it was measured properly, not because history
  was. See the previous handoff.
- **Nothing in the design system is missing for this feature** — I checked
  rather than assuming, after getting it wrong once mid-session. `--color-warning`
  and `--color-success` are both defined in all three themes in
  `frontend/src/styles/tokens.css`. What does *not* exist anywhere is
  `--color-success-bg` or `--color-warning-bg`, and the existing `badge-ok` rule
  already used the former, so it was rendering transparent. I changed both
  badges to the house pattern — colour on the neutral badge background, matching
  `.media-ref-health` in `WorkPage.svelte` — rather than introduce tokens the
  stylesheet does not define. A `background: var(--token-that-does-not-exist)`
  fails silently, so a missing token in a CSS custom property is invisible in
  review and in the browser.
