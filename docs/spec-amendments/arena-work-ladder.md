# The works ladder (§12.4a)

**Status:** adopted 2026-10-27. Amends §12.4 (the arena) and supersedes the reading in T-M16-09
as first written.

## The decision in one line

The arena rates **works**, with Glicko-2, from the best/worst ballot the page already collects.
`DimensionElo` stays in the database and stops being written by ballots on this instance, because
on this catalogue it cannot learn.

## This is not the reverted pairwise rewrite

A previous session replaced `crates/app/src/routes/arena.rs` and `Arena.svelte` with a Glicko-2
pairwise arena and was reverted (`docs/handoff.md`, "M59 Phase C1"), for three reasons: §0.4.2a
names a 4-card Plackett-Luce ballot with dimensional weights, no requirement row and no plan
mentioned the rewrite, and it did not compile.

None of that applies here, and the difference is the whole point:

- **The ballot, the page and the dimensional weights are unchanged.** Four cards, best and worst,
  reason tags, `apply_arena_ballot`, `weights_from_elos` — every one of them still exists and still
  runs. This adds a *second* consumer of the same ballot, not a replacement for the first.
- **It has a plan ticket** (T-M16-09) and this amendment, which is what the handoff said a new
  arena feature needs before it is written.
- **It is tested on both engines** before deployment, and the dimensional path it sits beside is
  not merely preserved but fixed: it was recording a loss for every ballot the instance could
  receive.

## Why the dimension ladder had to go, in numbers

`DimensionElo` rates the reader's five taste dimensions. A ballot updates *one* dimension's Elo.
That design is only meaningful if the four cards differ on that dimension, and the difference is
computed by `work_vector_from_tags` (`crates/domain/src/taste_vector.rs:246`), which raises a
work's score on a dimension **only when the work carries a tag whose name is the dimension key**.

Measured on the live catalogue, 2026-10-27:

| what | count |
|---|---|
| works | 58,042 |
| `work_tags` rows (over 18,724 works) | 30,981 |
| tags naming one of `angst`, `pacing`, `prose_density`, `canon_compliance`, `trope_intensity` | **0** |

So every arena card's vector is `[0.5, 0.5, 0.5, 0.5, 0.5]`. In `apply_arena_ballot`
(`taste_vector.rs:757`):

```rust
let actual = if best_score > worst_score { 1.0 } else { 0.0 };
```

`0.5 > 0.5` is false, so `actual` is `0.0` on **every ballot this instance can ever receive**,
while `expected` starts at 0.5. Each ballot therefore subtracts 16 from the target dimension and
nothing pushes back. `weights_from_elos` clamps at 0.01, so the profile this ladder produces
converges to five equal minimum weights — a flat profile, reached by a mechanism that looks like
learning.

Two further faults, both verified:

- **It is not wired to the feed.** The ballot writes `arena_weights` (0 rows live); the feed's
  personalisation reads `taste_profiles.signals`, computed from reading history
  (`crates/db/src/discovery.rs:475`), and `discovery::blend` takes no profile at all.
- **Its output has no consumer outside its own page.** `get_arena_elos` is called only from
  `crates/app/src/routes/arena.rs`.

## Why works, and why Glicko-2

A comparison between two works is informative **by construction**. It needs no taxonomy, no
per-work dimension score, and no tag vocabulary: `A beat B` is evidence about A and B regardless
of whether anyone can say what A's prose density is.

Glicko-2 over Elo for the same reason T-M16-01 gives: this corpus is recent-heavy, so a work
nobody has compared in months must drift back toward the middle, and a work compared twice must
move further on its third comparison than one compared two hundred times. That is the rating
deviation, and Elo has no way to express it.

## What one ballot becomes

The page asks for best and worst of four cards. That is five comparisons, not one:

```
best   beats middle1, middle2, worst
middle1, middle2 beat worst
```

**One ballot is one rating period per work**, not five separate updates — that is what Glicko-2's
signature exists for (T-M16-01, `Rating::update(&[Opponent])`). So each of the four works loads
once, updates once with its own opponent list, and is written once, inside a single transaction:

```
best    : 3 comparisons (2 middles + worst)
middle1 : 2 comparisons (lost to best, beat worst)
middle2 : 2 comparisons (lost to best, beat worst)
worst   : 3 comparisons (lost to all three)
```

`matches` increments by each work's comparison count, so the column means "comparisons", not
"ballots".

## What the request carries

`POST /api/v1/arena/vote` gains an optional `card_ids: [uuid]` — the four cards the reader was
shown. Absent or short, the ballot still applies, with **best vs worst only**: two works rated
build-by-build rather than four rated partially. The middles are worth carrying because they are
free signal, not because correctness needs them.

## What does not change

- The page, its four cards and its two buttons.
- `arena_weights`, `arena_ballots`, `apply_arena_ballot`, `weights_from_elos` — no migration of
  data, no deletion. The dimension ladder is left in place for an instance whose works *do* carry
  dimension tags, and it is recorded here as inert on this one rather than silently deleted.
- **The uninformative-ballot guard** (below) applies regardless of which ladder an instance runs.

## The guard, which is separate from the choice

`apply_arena_ballot` must not score a comparison it cannot distinguish. When
`best_score == worst_score` on the target dimension it records nothing — no Elo move, no
`matches_played` increment — because a ballot between two cards that look identical is not
evidence, and counting it as a loss is how a ladder drifts. This is a correctness fix, not a
consequence of choosing the works ladder: an instance with dimension tags can still show two
cards equal on the target dimension.

## Storage

```sql
CREATE TABLE work_ratings (
    work_id    UUID PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    mu         DOUBLE PRECISION NOT NULL,
    phi        DOUBLE PRECISION NOT NULL,
    sigma      DOUBLE PRECISION NOT NULL,
    matches    BIGINT NOT NULL DEFAULT 0,
    rated_at   TEXT NOT NULL
);
CREATE INDEX work_ratings_mu ON work_ratings (mu DESC);
```

Values are on the **Glicko scale** (`MU_INIT` 1500, `PHI_INIT` 350), because that is what
`Rating::update` documents and what kindred's parity fixture records; converting on the way in
would make the two ladders' numbers disagree for no reason.

`rated_at` is TEXT, matching `bookmarks.created_at` on both engines — a `TIMESTAMPTZ` here and
TEXT there is the drift `migration_catalogue` exists to catch.

## Verification

```
cargo test -p darkhold-db work_ratings --no-fail-fast                      # exit 0
cargo test -p darkhold-app arena_ballot_writes_the_works_ladder --no-fail-fast  # exit 0
```

And on the deployed instance, after one ballot:

```sql
select count(*), sum(matches) from work_ratings;   -- 4 rows, 10 comparisons
```

The `sum(matches)` is the check that matters: 3 + 2 + 2 + 3 = 10, and a per-work count that is
uniform would mean the ballot was applied as four independent updates rather than one period.
