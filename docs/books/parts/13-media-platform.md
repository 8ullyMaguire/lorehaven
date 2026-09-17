# Part 13 — The generalized media platform: editions, derivatives, lending and narration

Checkpoint: `v0.17-marketplace`

Lorehaven is not a text site with attachments. A work can have editions in
several media; media can be derived from each other; a library can lend; a story
can be read aloud. This part is where that generalization is paid for, and it is
the longest part of the book for a reason: it is where the interesting bugs live.

## 1. Checkpoint

```bash
git checkout v0.17-marketplace
```

## 2. What will work by the end

```bash
curl localhost:8080/api/v1/media/$EDITION                # an edition, anonymous if it is public
curl 'localhost:8080/api/v1/canons/$CANON/media?limit=20&cursor=…'
curl -X POST localhost:8080/api/v1/editions/$EDITION/derivatives -d '{"kind":"ocr_text"}'
curl -X POST localhost:8080/api/v1/works/$WORK/loans -d '{"account":"…"}'
curl localhost:8080/api/v1/me/loans                       # active | expired | revoked
curl -X POST localhost:8080/api/v1/editions/$EDITION/narration
curl -X POST localhost:8080/api/v1/editions/$EDITION/narration/approve
```

## 3. Concepts

- **Work → edition → file.** A work is the idea; an edition is a rendering
  (text, audio, a translated text, a print layout); a file is bytes with a
  checksum.
- **A derivative is machine-made and says so.** It carries its kind, its parent
  checksum, the program that made it and the failure classification if it failed.
- **Nothing derived is published by the machine.** A derivative lands as a draft
  for a human to approve.
- **Lending is a window, not a transfer.** A loan has a window, an expiry and a
  state that is computed from those, not stored as "lent".
- **Narration is an edition, built by a worker, approved by a person.**
- **Every door applies the adult gate.** No exceptions, and anonymous callers get
  404 for anything they cannot read.

## 4. Commands

```bash
lorehaven migrate        # applies 0024, 0025, 0026, 0029, 0030, 0031, 0032, 0033
cargo test -p lorehaven-app --test milestone_22
cargo test -p lorehaven-app --test milestone_25
cargo test -p lorehaven-app --test milestone_26
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0024_media_generalization.sql` | media editions and their kinds |
| `migrations/sqlite/0025_media_files.sql` | files, checksums, media types |
| `migrations/sqlite/0026_canon_space.sql` | canon and space media, with positions |
| `migrations/sqlite/0029_lending.sql` | loans |
| `migrations/sqlite/0030_derivatives.sql` | derivatives, kinds, jobs |
| `migrations/sqlite/0031_media_edition_creators.sql` | machine-producer credits |
| `migrations/sqlite/0032_media_editions_audio_checksum.sql` | narration audio |
| `migrations/sqlite/0033_loan_expiry.sql` | `work_loans.expired_at` |
| `crates/domain/src/media.rs` | edition kinds, media type rules |
| `crates/domain/src/derivative.rs` | `DerivativeKind`, `is_machine_produced` |
| `crates/domain/src/lending.rs` | loan windows and liveness |
| `crates/db/src/media.rs` | editions and scoped media, with cursors |
| `crates/db/src/derivative.rs`, `db/lending.rs`, `db/narration.rs` | storage |
| `crates/app/src/derivative.rs` | the OCR and transcode worker |
| `crates/app/src/narration.rs` | the narration worker |
| `crates/app/src/tts.rs` | the engine trait and its implementations |
| `crates/app/src/routes/media.rs`, `routes/derivative.rs`, `routes/lending.rs`, `routes/narration.rs` | the doors |
| `frontend/src/routes/Media.svelte` | the media page |

## 6. The code that matters

### Editions, and the machine-producer credit

```rust
// crates/domain/src/derivative.rs
impl DerivativeKind {
    pub fn is_machine_produced(&self) -> bool   // OCR, transcode: not the author's text
    pub fn program(&self) -> &'static str       // "tesseract", "ffmpeg"
    pub fn output_media_type(&self) -> &'static str
    pub fn remedy(&self) -> &'static str        // what to install, in the doctor's words
}
```

Declaring the program, the media type, the remedy and the machine-producer flag
**on the kind** means the door, the worker and `doctor` cannot disagree. When an
instance is missing `tesseract`, the answer is not a generic 500: it is
`CONVERTER_UNAVAILABLE` naming the program to install, produced from the same
declaration `doctor` prints.

Two rules follow from `is_machine_produced`:

- A machine-produced edition is credited to the machine, plus the account that
  asked for it. It never appears as the author's own work.
- A machine-produced edition is never auto-published. It is a draft until a human
  approves it.

### Derivatives: the pipeline

```text
1. the door validates the kind, checks the parent blob exists (checksum), enqueues a job
2. the worker: temp dir (a Drop guard), run tesseract or ffmpeg, capture stderr
3. store the result as a blob; record checksum, size, media type on the derivative
4. classify failure: fatal (bad parent, unsupported kind, program missing)
                    | transient (timeout, disk pressure)
5. mark the derivative stale when its parent changes
```

Real details that matter:

- **Temp directories need a Drop guard.** Cleanup in the success path only is a
  disk-filling bug: the failing path is exactly the one that leaves files behind.
- **Transcode target**: MP4, H.264 video with AAC audio and `+faststart` so the
  file plays while it downloads. An audio-only derivative is still an MP4
  container if you want one code path.
- **OCR output is text/plain**, stored as a blob like anything else, so the
  reader can show it and the export can include it.
- **A failed build is recorded on the row** with its classification. A derivative
  stuck at `queued` forever is indistinguishable from one nobody asked for.
- **Staleness is not deletion.** When the parent changes, the derivative is marked
  stale and stays readable with a notice. Deleting a derivative because its parent
  moved is data loss the user did not ask for.

### Lending: a window with a state derived from it

```sql
work_loans (work_id, borrower_account_id, granted_at, expires_at, expired_at, revoked_at)
```

Two bugs worth building the tests for before you write the code:

1. **The unique constraint bites on re-borrow.** A reader whose loan expired
   still has a row; an unconditional insert fails with a unique violation and the
   reader gets a 500 on every attempt to borrow the same book again. The grant is
   an upsert that re-grants the row.
2. **`is_active()` must consider expiry.** A loan whose window closed is not
   active, no matter what state column says. Derive liveness from the timestamps
   and treat any stored status as a cache at best.

And an operational detail: the maintenance pass stamps `expired_at` when the
window closes, and the API reports `active|expired|revoked`. The stamp is for
reporting and analytics — **every read checks the timestamps**, because a sweep
that has not run yet must not hand someone a book they may no longer borrow.

### Narration: an engine behind a trait

```rust
pub trait TtsEngine {
    fn name(&self) -> &'static str;
    fn is_available(&self) -> bool;
    fn health(&self) -> EngineHealth;      // names what is missing, and the remedy
    fn synthesize(&self, text: &str, voice: &str) -> Result<Audio>;
}
```

Three implementations, and the third is the interesting one:

- `PiperEngine` — a local binary, invoked with **argv only, never a shell**, and
  a temporary file that is cleaned up.
- `SilentEngine` — a valid WAV whose duration follows the text length. It exists
  so the whole pipeline can be exercised on a machine with no synthesizer, and in
  CI. A test fixture that is a real file is worth ten mocks.
- `MissingEngine` — `health()` names the missing program and the fix, so the
  request door can refuse up front instead of queueing a job that cannot succeed.

Splicing audio is where the subtle bug is:

```rust
// concat_audio parses the RIFF chunks and rewrites the sizes.
// [a, b].concat() is not a playable file — the header still describes a.
```

Chunking the text is the other one: split on sentence boundaries first, and never
in the middle of a multi-byte character. A Chinese novel narrated with mangled
boundaries is a bug report you will not enjoy.

Narration lands as a draft edition with its `media_editions.audio_checksum` set,
and `approve_narration_edition` refuses an edition with no audio — so you cannot
publish a silent chapter by accident.

### Scoped media, and the pagination that has to be right

```text
GET /api/v1/canons/{id}/media?limit=&cursor=
cursor = "<position>|<created_at>|<id>"        limit ≤ 200
```

Same rule as Part 8: the cursor carries the whole ordering key, and
`next_cursor` is returned only for a full page. A canon is a curated, ordered
list — the ordering matters and the position is what users reorder.

### The adult gate on every door, including the new ones

```text
any door that can return media, editions, derivatives, narration
  → MaybeSession, then the visibility rule
  → explicit content: 404 for anyone not eligible
```

This is the part of the project where new doors are added fastest, and therefore
where an ungated door is most likely. Make it a checklist item in every commit:
*does this new door have a `MaybeSession` extractor and a visibility check?*
Two projects' worth of incidents fit in that question. The right pattern for a
new door is to call the same helper the work page uses — never to re-implement
the check.

### The author's own numbers

`GET /api/v1/me/dashboard` returns the acting pseud's own works and what readers
did with them. The design rules are the point of this section:

```json
{
  "works":   { "total": 3, "published": 2, "unpublished": 1, "chapters": 12, "words": 34000 },
  "readers": { "bookmarks": 8, "ratings": "fewer_than_5", "reviews": 0 },
  "privacy": { "floor": 5, "note": "reader-facing counts below the floor are reported as a band" }
}
```

- The author's **own inventory is exact**: how many works they wrote is a fact
  about them.
- **Reader-facing counts are banded** at a floor (5 in this implementation): a
  count below it is a string, never a number a client would render as an exact
  figure. One bookmark is a specific person's act; "fewer than five" is not.
- **No "held by the filter" counter.** The author's view is framed as what
  arrived. A scoreboard of what was withheld turns the filter into a grievance
  machine.
- **No reader identity** appears anywhere in the payload, and the SQL is written
  so it cannot: aggregates only, over the caller's own works.

## 7. Tests

`milestone_22.rs` (scoped media):

- a two-page cursor walk at `limit=2` returns three pages, each item once, in
  position order;
- `limit=0` and a malformed cursor are refused;
- an anonymous caller gets 404 for media on a work they cannot read.

`milestone_25.rs` (derivatives, lending, archive mode, Dublin Core):

- requesting a derivative whose program is absent is refused with the actionable
  code, and enqueues nothing;
- an OCR result is stored with its checksum and media type, and is a draft;
- a parent change marks the derivative stale without deleting it;
- re-borrowing after expiry succeeds (the upsert), and the loan list reports
  `expired` for the old one;
- a loan whose window closed is not active even before the sweep runs.

`milestone_26.rs` (narration and the adult gates):

- the narration request refuses when no engine is usable, naming the engine;
- the chunker never splits a multi-byte character, and the spliced WAV is valid
  (RIFF sizes rewritten);
- approving an edition with no audio is refused;
- an anonymous reader gets 404 — not 401 — for a narration of a work they cannot
  read;
- an age-ineligible reader gets 404 for explicit media on every door, including
  the media list, the derivative list and the audio endpoint.

## 8. Expected UI behaviour

- A work page lists its editions with their kind and language.
- Requesting OCR shows a job and, when it finishes, a draft edition with a
  "machine produced" credit.
- A borrowed work shows its remaining window, and the list keeps expired loans
  with their dates instead of hiding them.
- Narration plays in the browser once approved; before approval, only the author
  sees it.
- Media that the reader may not see is simply absent — no lock icon, no teaser.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Re-borrowing a book is a 500 | unique (work, borrower) violated by an expired row | upsert the grant |
| An expired loan still reads as active | liveness read from a stored state | derive from timestamps |
| Narration audio does not play | spliced WAV header still describes the first chunk | rewrite RIFF and `data` sizes |
| Narration fails on non-Latin text | the chunker split inside a character | split on char boundaries, sentence-first |
| Derivative stuck at `queued` | the failure was recorded nowhere | record the failure and its classification on the row |
| Temp files fill the disk | cleanup only on success | a Drop guard around the temp dir |
| A new media door leaks explicit content | the extractor was `RequireSession`, or no visibility check | `MaybeSession` + the shared helper, in the same commit as the door |

## 10. Consequences

- **Derived content is still someone's work.** OCR text and a transcode carry the
  author's rights; credit them, and keep the machine-producer marker visible.
- **Lending has a legal shape** in some jurisdictions. Decide what your instance
  claims before you enable it, and make it configurable.
- **Narration voices are people.** Some engines clone voices; if yours can, the
  consent question is yours, not the user's.
- **Banded counts protect readers, not the author's vanity.** Do not remove the
  floor to make a dashboard look fuller.

## 11. Checkpoint

```bash
git tag v0.22-media
```

Verified by `milestone_22.rs`, `milestone_25.rs` and `milestone_26.rs` on both
dialects, plus a hand pass with `ffmpeg` and `tesseract` installed: an OCR of a
scanned page, a transcode, a scratch loan cycle and a narration rendered by the
silent engine with the pipeline end to end.
