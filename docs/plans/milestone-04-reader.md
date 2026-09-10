# Milestone 4 — Reader, ratings, history and work pages

Spec §9. Tag to leave behind: `v0.05-reader`.

## Goal, stated as a journey

A signed-out visitor opens a published work, reads it chapter by chapter, and
their position is remembered when they come back. A signed-in reader can also
rate it privately, keep private notes, and see what they have been reading.

Work through the journeys in this order and check each one in a browser before
starting the next:

1. Visitor opens `/works/:id`, sees title, authors, tags and the chapter list.
2. Visitor opens chapter 1, uses *next chapter*, reaches the last chapter, and
   sees the end-of-work actions.
3. Visitor changes typography (size, width, theme) and their choice survives a
   reload.
4. A signed-in reader's position is stored, and on their second visit the work
   page offers to resume.
5. A signed-in reader rates a work privately; the rating is not on the work
   page, not in the public aggregate, and not visible to the author.
6. A signed-in reader opens `/library/history` and sees what they read, and can
   delete one entry and clear the whole log.

## What exists to build on

* `crates/db/src/content.rs` — `find_work`, `chapters_for_work`,
  `find_chapter`, `find_revision`. The reader needs no new way to load text.
* `crates/app/src/routes/works.rs::read_router()` — `/works/:id` and
  `/works/:id/chapters/:chapter` already answer anonymously and already refuse
  a draft with `404` through `can_access_content`. **Do not add a second
  eligibility check.** If the reader needs a new fact, add it to
  `ContentFacts`.
* `crates/domain/src/document.rs` — `to_sanitized_html()` is what the reader
  renders. The reading view is already served by `/works/:id/chapters/:chapter`.
* `frontend/src/routes/ChapterRead.svelte` and `WorkRead.svelte` — the pages
  exist in a first form. This milestone enriches them.

## The work, in order

### Task 1 — Migration 0004: reading progress, ratings, reviews, history, notes

Files: `migrations/sqlite/0004_reading.sql`, `migrations/postgres/0004_reading.sql`.

Tables, with the columns spec §9 names:

```text
reading_progress
    id, account_id, pseud_id (nullable), subject_type ('work'|'library_item'),
    subject_id, chapter_id (nullable), content_revision (nullable),
    paragraph_anchor (nullable), position_fraction (REAL, 0..1),
    device_id (nullable), created_at, updated_at, version
    UNIQUE (account_id, pseud_id, subject_type, subject_id, device_id)
    -- device_id nullable: SQLite and PostgreSQL both treat NULLs as distinct
    -- in a UNIQUE index, so a reader with no device id keeps one row per row
    -- rather than colliding. Document that in the migration comment.

rating
    id, account_id, pseud_id, work_id, stars (1..5), created_at, updated_at,
    version, deleted_at
    UNIQUE (pseud_id, work_id)
    -- Private by construction: no `is_public` column yet. §9.4 adds the
    -- public aggregate in a later migration only when explicitly published.

review
    id, account_id, pseud_id, work_id, body, contains_spoilers (0/1),
    is_public (0/1, default 0), published_at, created_at, updated_at, version,
    deleted_at
    UNIQUE (pseud_id, work_id)

reading_history_entry
    id, account_id, pseud_id, subject_type, subject_id, last_read_at,
    revision_seen, created_at
    UNIQUE (account_id, pseud_id, subject_type, subject_id)

reader_note
    id, account_id, pseud_id, subject_type, subject_id, anchor, body,
    created_at, updated_at, version, deleted_at

typography_preference
    account_id PRIMARY KEY, font_scale REAL, line_height REAL, measure INTEGER,
    reader_theme TEXT, distraction_free (0/1), created_at, updated_at, version
```

Deletion and retention comments to write in both files:

* `reading_progress`, `reading_history_entry`, `reader_note`,
  `typography_preference` cascade with the account: they are private reading
  data with no audit value.
* `rating` and `review` soft-delete (`deleted_at`) so a public aggregate can be
  recomputed after a deletion, and so a moderation review can still see what
  was written.
* Nothing here is ever readable by another account. State that in the comment,
  because a future contributor will be tempted to join it into a public query.

Then run `cargo test -p lorehaven-db`: `the_two_dialects_define_the_same_migration_ids`
must pass, and `lorehaven migrate` on a scratch database must apply 0004.

**Pitfall.** `position_fraction` is `REAL`/`double precision` and binds as
`f64`, which breaks the "bind only String and i64" convention. Either store it
as `position_permille INTEGER` (0–1000) — recommended, and it is enough
precision for "where was I" — or add an `f64` bind path to the two branches
that need it and say why in a comment. Pick one and be consistent.

### Task 2 — Domain: reading position and progress rules

File: `crates/domain/src/reading.rs` (new; export it from `lib.rs`).

```rust
pub struct ReadingPosition { pub revision: Option<RevisionId>, pub anchor: Option<String>,
                             pub fraction: u16 /* permille */, pub device: Option<String> }

pub enum ProgressResolution { UseStored(ReadingPosition), AskTheReader { a: ReadingPosition,
                             b: ReadingPosition }, NoPosition }

/// Spec §9.3: "When devices disagree, present a choice rather than always
/// taking the furthest position."
pub fn resolve_progress(rows: &[ReadingPosition]) -> ProgressResolution;

/// Whether a stored position still points at content the reader saw.
pub fn position_is_reliable(position: &ReadingPosition, current_revision: RevisionId) -> bool;

pub struct ReadingTime { pub minutes: u32 }
/// Spec §9.4 §9.6: 200 words per minute, floored at one minute for non-empty.
pub fn estimate_reading_time(word_count: u32) -> ReadingTime;
```

Tests to write in the same file: single position → `UseStored`; two devices
agreeing → `UseStored`; two devices differing → `AskTheReader`; a position
whose revision no longer exists → unreliable; `estimate_reading_time(0) == 0`,
`estimate_reading_time(1..200) == 1`, `estimate_reading_time(1000) == 5`.

### Task 3 — Repository: `crates/db/src/reading.rs`

Functions, each in the established dual-dialect form:

```text
save_progress(db, account, pseud, subject, position) -> ()   // upsert on the unique key
progress_for(db, account, subject) -> Vec<ReadingPosition>   // one per device
touch_history(db, account, pseud, subject, revision) -> ()   // upsert last_read_at
history_for(db, account, limit) -> Vec<HistoryRow>           // joined with work metadata
delete_history_entry(db, account, entry) -> bool
clear_history(db, account) -> u64
upsert_rating(db, account, pseud, work, stars) -> i64        // returns new version
rating_for(db, pseud, work) -> Option<Rating>
public_rating_summary(db, work) -> Option<RatingSummary>     // count + mean, ≥5 public ratings only
upsert_review(...) / review_for(...)
notes_for(db, pseud, subject) -> Vec<Note>  /  save_note(...) / delete_note(...)
typography_for(db, account) -> Typography                    // row or defaults
save_typography(db, account, expected_version, …) -> bool
```

Two things that will otherwise be got wrong:

* **A private rating never contributes to the public aggregate.** Write the
  aggregate query with `WHERE is_public = 1` and write a test that a private
  rating moves it by zero. Spec §9.4 is explicit.
* **`>= minimum_publication_threshold`.** The spec requires a minimum count
  before an aggregate is shown; put the constant in
  `crates/domain/src/content.rs` (`MIN_PUBLIC_RATINGS: i64 = 5`) and have the
  query's `HAVING` clause use it, so the rule and the query cannot drift.

### Task 4 — Routes: `crates/app/src/routes/reading.rs`

```text
GET    /works/:id/chapters/:chapterId            (existing; extend, do not duplicate)
PUT    /reading/progress                         upsert this device's position
GET    /reading/progress?subject_id=…            positions for a subject (one per device)
DELETE /reading/progress?subject_id=…            forget this device's position
GET    /library/history                          with ?cursor= pagination envelope
DELETE /library/history/:id
POST   /library/history/clear
PUT    /works/:id/rating                         { stars, expected_version? }
DELETE /works/:id/rating
GET    /works/:id/reviews                        public reviews only
PUT    /works/:id/reviews                        create or update the caller's review
GET    /notes?subject_id=…                       private notes for the acting pseud
PUT    /notes                                    create/update
DELETE /notes/:id
GET    /settings/typography                      effective values + defaults
PATCH  /settings/typography                      { expected_version, … }
```

Register the mutable ones in `crates/app/src/routes/mod.rs` and in
`build_router` under `account_routes` with `RouteClass::Write`; put
`/settings/typography` with the other settings; put the read-only ones in
`works::read_router()` or a new `reading::read_router()` merged with
`RouteClass::Default`.

Acceptance tests to add to `crates/app/tests/milestone_4.rs` (copy the harness
from `milestone_3.rs`):

* a visitor can read a published chapter; a visitor gets `404` on a draft;
* progress saved by device A does not overwrite device B's row, and the work
  page offers both;
* a rating saved by one pseud is invisible to the account's other pseud;
* a private rating does not change the public aggregate;
* the public aggregate is absent below the minimum count and present above it;
* history is per-pseud: switching pseud shows the other face's history;
* `DELETE /library/history` removes only the caller's rows;
* typography is account-scoped (it follows the account, not the pseud);
* a stale typography `PATCH` returns `409 REVISION_CONFLICT`.

### Task 5 — Frontend

New files:

```text
frontend/src/routes/Reader.svelte          the reading surface (replaces the body of ChapterRead)
frontend/src/routes/History.svelte         /library/history
frontend/src/routes/WorkPage.svelte        the enriched /works/:id
frontend/src/lib/components/ReaderSettings.svelte
frontend/src/lib/components/ResumePrompt.svelte
frontend/src/lib/components/Rating.svelte
frontend/src/lib/components/NotePanel.svelte
frontend/src/lib/reading.ts                local position cache + flush on unload
frontend/src/lib/reading.test.ts
```

Router additions in `frontend/src/lib/router.ts`: `/library/history` → `history`
(remove `/library` from `PLANNED_ROUTES`, and split it: `/library` stays
planned until M8, `/library/history` resolves now).

Behaviour to implement carefully:

* **Typography and theme are applied before first paint.** Add an inline
  bootstrap in `frontend/index.html` (like the existing theme bootstrap) that
  reads the stored preference and sets the CSS custom properties, so a reader
  does not see a flash of the wrong size.
* **A position is saved on scroll-end and on `visibilitychange`**, debounced,
  and the local copy is written to `localStorage` first so a lost request still
  leaves a position. Reuse the shape of `frontend/src/lib/autosave.ts`.
* **Whole-work mode is paginated, not one long DOM.** Spec §9.2: "Long works
  must not require rendering every paragraph at once." Render one chapter at a
  time in whole-work mode, and append the next when the reader approaches the
  end (an `IntersectionObserver` on a sentinel). Say in a comment that this is
  the reason, or someone will "simplify" it back into one render.
* **Spoiler reveal and private notes are per reader**, and a note is never
  shown in the reading text — it is a side panel.
* **End-of-work actions** (spec §9.8): next in series, rate, bookmark (M8),
  download (M7 — link to a `Planned` note until then), and *back to the
  chapter list*.
* **The interface must not offer what the server will refuse.** If the server
  says `CONTENT_RESTRICTED`, say why in the reader's words, quoting the
  request id.

### Task 6 — Verify, then write it down

* `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo fmt --all -- --check`.
* `bash frontend/scripts/fe.sh build`, `… test`.
* Drive all six journeys above in a real browser against the compiled binary
  serving its embedded bundle, and write each step into `docs/verification.md`.
* Update `docs/requirements.csv`: every `M4-*` row moves off `unsupported`,
  with a real `evidence` value.
* Commit, then `git tag v0.05-reader`.

## Acceptance for the milestone, restated as tests

Spec §9 `Acceptance` requires, and these must each be a named test:

| Requirement | Test |
|---|---|
| Position survives a chapter edit | `resuming_after_an_edit_uses_the_anchor_not_the_offset` |
| Devices that disagree offer a choice | `two_devices_that_disagree_produce_a_choice` |
| A private rating is private | `a_private_rating_changes_no_public_number` |
| The aggregate states its method and count | `the_aggregate_reports_its_count_and_method` |
| History is per pseud | `switching_pseud_shows_a_different_history` |
| A reader can erase their history | `clearing_history_removes_only_the_callers_rows` |

## Pitfalls specific to this milestone

1. **Do not put the reader behind a session.** Anonymous reading is a spec
   requirement (§7) and the page already works signed out; adding
   `RequireSession` here would silently break it, and the test suite will not
   notice unless you write the anonymous test first.
2. **Do not read `reading_progress` in a public listing query.** It is private
   reading data; joining it into anything a second account can see is a
   disclosure.
3. **`position_fraction` and float binding** — see Task 1.
4. **A chapter's `current_revision_id` can be NULL** (a chapter that has never
   been saved). The reader must show an empty chapter rather than erroring.
5. **Reading time is an estimate and must be labelled as one.** Spec §3.2:
   statistical estimates may use floats "with documented interpretation". Put
   the words per minute in the domain constant with a comment naming it as an
   assumption.
