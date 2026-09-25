# Handoff — M32-07d: the media fetch job, driven end to end

Date: 2026-09-25. Previous full handoff:
`docs/handoffs/2026-09-25T173000+0200-m32-07c-real-image-decoding-handoff.md`.

## What this closes

M32-07b's handoff recorded the thing plainly: `record_fingerprint` had **no
production caller** — only tests. That is the "a parser is not a feature"
shape, and it meant the perceptual dedup search read a column nothing ever
wrote, no matter how correct the guard, the decoder and the write were.

`add_media_reference` has always answered 201 with a `pending` placeholder and
a comment promising "content hash computed async by the pipeline". This is that
pipeline.

The chain is now driven, not merely assembled: confirm the reference exists →
take the first availability link by priority → refuse the URL → resolve the host
→ fetch with the instance client → classify before reading the body → read
bounded → fingerprint → record.

## What was added

- **`JobKind::MediaFetch`.** The `job_kinds!` macro emits the enum, `ALL_KINDS`,
  `as_str` and `parse` from one source, and `kind_index`/`resource_class` are
  exhaustive matches — so a new kind cannot be "handled everywhere except the
  list". A test asserts the kind is in `ALL_KINDS`, round-trips through the wire
  form, is `Interactive`, and that every `kind_index` is unique.
- **`app::media_job::enqueue_media_fetch`** and the worker's dispatch arm.
- **The enqueue call in `add_media_reference`**, and `fingerprint_job_id` in the
  201 response.
- **`HandlerError` gained `Display` and `Error`** so it formats normally.

**A job, not request work:** the URL is author-supplied and a slow or dead mirror
must not hold an author's connection open. The payload names only the reference
id — no job row carries a URL, and a corrected link is picked up by a retry.

## The decision that matters: the allowlist, not the bypass

Testing this needs a loopback HTTP server, and loopback is refused
unconditionally — correctly, since on a self-hosted instance the thing being
protected is usually on the same machine. My first cut took
`trusted_address: Option<IpAddr>` and branched *around* the guard.

It failed immediately, because the literal-address rejection lives **inside**
`plan_fetch` and the branch was never reached. It was also wrong regardless: a
bypassed guard leaves a second, untested path through the most
security-sensitive function in the chain, and a reviewer cannot tell which branch
production takes.

The working shape is `plan_fetch_allowing(url, allow: &[IpAddr], timeout)`, with
`plan_fetch` delegating to it with an empty slice. The scheme, local-name and
resolution checks all still run. Production passes an empty allowlist and has
exactly one path. The allowlist applies to **both** the literal-address check and
the DNS-resolution check, so the two cannot disagree about what is reachable.

## Classification is honoured, not lumped

- 404/410, a non-image content type, an oversized body, an unparseable URL, a
  missing reference, and a reference with no availability link → **Fatal**. A
  job with no link retrying every minute for the life of the instance is the
  failure mode to avoid.
- 429/5xx, a connect failure, a read failure, a timeout → **Transient**.
- The body is read chunk-by-chunk against `MAX_MEDIA_BYTES`, so a server that
  declares no `Content-Length` cannot send unbounded data.

A body that will not decode is still a **successful** fetch: the exact content
hash of those bytes is true, and the perceptual hash is honestly `NULL` rather
than fabricated. A test asserts exactly that, and another asserts the curator's
`find_by_perceptual_hash` finds what the worker wrote — the end-to-end property
the whole feature rests on.

## A test that tested wording

The status-classification test decided transience with
`error.to_string().contains("transient")` and failed on a correct message that
said "retrying". The behaviour was right; the test was a test of the prose. It
now matches on the error *type* and keeps the message in the assertion text, so
rewording cannot break it.

## Verification

clippy 0/0 · fmt clean · **82 suites, 0 failures** ·
12 end-to-end tests in `crates/app/tests/media_fetch_job.rs`, each standing up a
real HTTP server and serving a real PNG (or an HTML body, or a 404, or garbage
bytes declared as `image/png`).

No frontend file was touched, so the Svelte and E2E gates were not re-run.

## What is still not shipped

- **`phash`/`whash`/`ahash` remain unimplemented** and are refused at compute
  time; `dhash` is the default.
- **`audio_fingerprint` is still unapplied to audio** — the spec's audio half of
  §32.7.2 is untouched.
- **The worker is not run in the E2E suite.** These tests drive the handler
  directly against a local server. A Playwright test that watches a real
  reference go from `pending` to hashed while the worker runs would close that,
  and is the natural next step now that the chain exists.
- **Only the first availability link is tried.** The table is ordered by
  priority and a reference can have several mirrors; falling through to the next
  link on a 404 is the obvious next refinement, and the classification work is
  already in place for it.

## Where to look

- `crates/app/src/media_job.rs` — the handler, `read_bounded`, `classify_failure`
- `crates/app/tests/media_fetch_job.rs` — `TestServer`, the 12 end-to-end cases
- `crates/app/src/media_fetch.rs` — `plan_fetch_allowing`, the test PNG builders
- `crates/app/src/routes/media_resilience.rs` — the enqueue and the response
- `crates/domain/src/jobs.rs` — the `MediaFetch` variant and its three matches
