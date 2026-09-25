# Handoff — M32-07c: real image decoding, so a fetched image gets a real hash

Date: 2026-09-25. Previous full handoff:
`docs/handoffs/2026-09-25T164500+0200-m32-07b-perceptual-hashes-computed-and-stored-handoff.md`.

## What this closes

M32-07b said plainly that a fetched JPEG or PNG could be content-hashed exactly
but not perceptually, because nothing in the tree could turn bytes into pixels.
That is now closed. `media_fetch::fingerprint_encoded` decodes a body to luma and
produces both hashes in one call.

The chain is now complete end to end: `plan_fetch` refuses a forbidden host →
`classify_with_length` refuses a non-image or an oversized one → decode to
pixels → `difference_hash` → `record_fingerprint` writes both hashes → the
curator's `find_by_perceptual_hash` finds it. Every link is tested at its
boundary; the job that would *drive* the chain does not exist yet (see below).

## The dependency, and the pin

`image = "=0.25.6"` with features `png,jpeg,gif,webp,bmp`. 14 transitive deps,
no `rayon`, so the build stays serial.

The pin is the interesting part. The workspace declares `rust-version = "1.82"`.
`image` 0.25.7 requires Rust 1.85, 0.25.10 requires 1.88 — both build fine on
this host's 1.98.1, so an unpinned `cargo add` would have moved the workspace's
declared MSRV floor without failing anything locally. 0.25.6 declares 1.70.

## The size limit goes on the decoder, not after it

My first version called `image::load_from_memory` and then checked
`width * height > MAX_DECODED_PIXELS`. That is decoration: the buffer is already
allocated by the time a caller can measure it, and a 20000×20000 solid-colour
PNG is 400 MB of luma before the check runs. `image`'s own defaults leave
`max_image_width`/`max_image_height` unbounded and cap only `max_alloc` at
512 MiB, so an image under that allocation cap but far past ours still gets
built.

The working version is `ImageReader::new(cursor)` → `image::Limits::default()`
with both dimension fields set → `reader.limits(...)` → `with_guessed_format()?.decode()?`.
`Limits` is `#[non_exhaustive]`, so it must be built from `Default` and mutated;
a struct literal does not compile. There is a test for the bomb and a companion
for an image inside the limit, because a bound that refuses everything is not a
bound, it is an outage.

## Two test-fixture corrections, both mine

**The re-encode test was fiction.** I built "the same image, re-encoded" by
flipping a PNG filter byte from 0 to 1 — and the byte was already 1, so the copy
was byte-identical and the test's own precondition failed. It was also wrong in
principle: changing a filter without re-encoding the pixel deltas changes the
*decoded* image, so the two would not have been the same picture. The fixture now
splices a `tEXt` chunk before IEND: different bytes, identical pixels, which is
the real-world shape of the problem (the same picture saved by a different tool).
That test now asserts the property the whole feature rests on — a byte-different
copy produces an identical perceptual hash.

**The bomb fixture timed the suite.** It emitted `width * height` zero bytes, so
its own test took 33 seconds while the decoder refused correctly throughout. A
bomb only needs its *declared* size with one real row and the rest absent. Now
0.00s.

Clippy also caught a constant-tautology assertion I had added to that test
(`MAX_DECODED_PIXELS < 20000 * 20000` is decidable at compile time); removed.

## Dead code removed

`FetchOutcome::NeedsDecoding` existed only to express "this build cannot decode
images". A decoder now exists, so nothing produces it. Removed rather than left
as a variant whose documentation lies.

## Verification

clippy 0/0 · fmt clean · **81 suites, 1773 passed, 0 failed** ·
23 tests in `crates/app/tests/media_fetch.rs` (6 decoding, 17 guard/persistence).

One caveat, stated precisely: an earlier full run in this session reported
`repeated_login_attempts_are_rate_limited` failing in `milestone_2`. That is the
documented rate-limiter flake (process-global buckets keyed by IP), it passes in
isolation (verified), and the re-run of the whole suite was clean. The change
touches four files — `Cargo.lock`, `crates/app/Cargo.toml`, and the two
`media_fetch` files — and no auth or limiter code.

No frontend file was touched, so the Svelte and E2E gates were not re-run.

## What is still not shipped

- **No media job kind and no fetch loop.** The guard, the decode and the write
  are tested at their boundaries; nothing queues a fetch and calls them in
  sequence. This is now the only structural gap in the chain.
- **`phash`/`whash`/`ahash` remain unimplemented** and are refused at compute
  time; `dhash` is the default.
- **`audio_fingerprint` is still unapplied to audio.**
- **`record_fingerprint` has no caller in production code** — only tests call it.
  That is the same "a parser is not a feature" shape the skill warns about, and
  it is why the job wiring is the next milestone rather than a polish item.

## Where to look

- `crates/app/src/media_fetch.rs` — `fingerprint_encoded`, `MAX_DECODED_PIXELS`
- `crates/app/tests/media_fetch.rs` — `tiny_png`, `png_declaring`, `reencoded_copy`
- `crates/app/Cargo.toml` — the pinned `image` line
