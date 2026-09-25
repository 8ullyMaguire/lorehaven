# Handoff — M32-07b: perceptual hashes are computed, stored, and the default no longer lies

Date: 2026-09-25. Builds on `docs/handoffs/2026-09-25T153000+0200-m32-07a-perceptual-dedup-implemented-handoff.md`.

## What this closes

M32-07a shipped a correct Hamming-distance search and then, in its own handoff,
called the missing half "blocked on a dependency". That was wrong in the way
that matters: crates.io was reachable the whole time and no decoder was in the
lockfile. The dependency was *absent*, not *unavailable*, and adding one is
ordinary work. I checked before building on the claim.

The gap was narrower than the last handoff implied, and wider in one place:

- `perceptual_hash` was written by exactly one test and nothing else — confirmed
  by `grep -rn 'SET perceptual_hash'`, which matched no non-test code.
- There is no media job kind and no fetch path at all. `media_references` rows
  are created as a `pending` placeholder because the bytes are not known yet, and
  nothing ever replaced that placeholder.

## What was built

**`domain::media_resilience::difference_hash`** — a real 64-bit dHash. The frame
is resampled to a fixed 9×8 grid and each sampled pixel is compared with the one
to its left: 8 rows × 8 comparisons = 64 bits. The grid is fixed rather than
derived from the input, which is the only reason hashes of two differently-sized
copies of one image are comparable at all. `None` for a wrong pixel count or an
image smaller than 2×2.

**`app::media_fetch`** — the guard layer. `plan_fetch` refuses a non-HTTP scheme,
a local name, and any literal address the shared scraper guard calls forbidden.
`classify_with_length` refuses an oversized declared length from the header
alone, separates retryable (429/5xx) from permanent (404/410) failures, and
rejects a non-image content type. `MediaFingerprint` carries both hashes, and
`without_perceptual_hash` exists for a body this build cannot decode.

**`db::media_resilience::record_fingerprint`** — the write that makes the column
real, plus a local `Fingerprint` struct so the repository stays below the fetcher
instead of depending on it. An `affected == 0` is an error, not a silent success.

## Three bugs I introduced and caught

**The first dHash read a fixed window of the input.** It walked pixels until it
had 64 bits, so a 16×16 image contributed only its first five rows: two images
sharing a top half and differing entirely below it hashed identically. Every
test I wrote first — a ramp, a flat frame, a brightness shift, a wrong length —
passed against it. Only a test that varies the *bottom* of the image failed. Now
fixed, and that failure mode is the lesson: a perceptual hash that reads a
prefix is not a perceptual hash.

**`assert_eq!` was the wrong assertion for a brightness shift.** A uniform shift
clips at white, turning runs of pixels into ties, so the shifted hash differs by
three bits out of 64 instead of not at all. The test failed on correct code. The
real guarantee is `hamming_distance(base, shifted) <= threshold`, which is also
the only property the dedup search uses.

**`host_str().parse::<IpAddr>()` skipped every IPv6 literal.** The string form
keeps its brackets, so `http://[::1]/`, `http://[fe80::1]/` and
`http://[::ffff:127.0.0.1]/` all parsed to `None` and fell through the address
check entirely — precisely the loopback and metadata addresses an SSRF guard
exists to catch. Fixed by matching `url.host()` and its `url::Host` variants, with
`to_ipv4_mapped()` on top so a v4-mapped v6 address is not a costume.

## The default was lying

`perceptual_hash_algorithm` defaulted to `phash`, and phash is not implemented —
dHash is the only algorithm this build computes. A stock instance therefore
advertised a perceptual dedup it could not perform. The default is now `dhash`,
with a test pinning it, and the enum's doc, `docs/config-reference.md` and the
§32.7.2 example block were updated together. `phash`/`whash`/`ahash` are still
accepted by config so an operator can record intent, but a build asked to compute
one refuses rather than storing another algorithm's output under its name.

## What is *not* shipped

`implemented-locally-tested` on M32-07b means the hash, the guard and the
persistence are correct and tested. It does **not** mean the site deduplicates
images yet:

- **Image decoding is still absent.** A real fetched JPEG or PNG cannot be turned
  into grayscale pixels in production, so a live fetch yields the exact content
  hash with a `NULL` perceptual hash. That is the honest result — the alternative
  is a fabricated fingerprint, and a `NULL` is skipped by the search while an
  empty string would compare as distance 0 against every other one. Next: add a
  decoder and call `difference_hash` on the decoded buffer.
- **No job kind and no fetch loop.** The guard and the write are tested at their
  boundaries, not driven by a queued job. That wiring is the remaining step.
- **`audio_fingerprint` is still unapplied to audio.**
- **phash/whash/ahash remain unimplemented.**

## Verification

clippy 0/0 · fmt clean · 1766 workspace tests, 0 failed · 31 dHash domain tests
· 17 `media_fetch` tests · 5 persistence tests. No frontend change, so the
Svelte and E2E gates were not re-run; nothing in `frontend/` was touched.

## Where to look

- `crates/domain/src/media_resilience.rs` — `difference_hash`, `sample_column`,
  `sample_row`, the algorithm enum's implementation note
- `crates/app/src/media_fetch.rs` — new module
- `crates/app/tests/media_fetch.rs` — the SSRF refusal table
- `crates/db/src/media_resilience.rs` — `Fingerprint`, `record_fingerprint`
- `crates/app/src/config.rs` — the default-algorithm test
