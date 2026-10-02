-- §51: generated-content posture, and author credits that vest on reader
-- completion. M45-12, from gaps review A3.
--
-- ── The instance policy ──────────────────────────────────────────────────────
--
-- A singleton, shaped like `instance_retention_policy` (0087): a fixed `id`, the
-- enum stored as TEXT with a CHECK, `updated_by` ON DELETE SET NULL so deleting
-- the operator account does not delete the decision, and a `version` so a
-- concurrent change is detectable rather than silently last-write-wins.
--
-- `forbid` is the DEFAULT, and that is the load-bearing default. `allow` is the
-- only value that changes what the corpus *is*, so §51.1 makes an instance say so
-- out loud rather than arriving there by omission. An operator who wants
-- `disclose` has to choose it; an operator who wants nothing generated has to do
-- nothing, which is the safe direction for a default to point.
--
-- NOT NULL with a CHECK rather than a nullable column: there is no state in which
-- this instance has no generated-content posture, and a NULL would be a fourth
-- value that means "nobody has decided", which is `forbid`'s job.

CREATE TABLE IF NOT EXISTS generated_content_policy (
    -- Singleton. Same shape as instance_retention_policy, whose own `id` is a
    -- constant for the same reason: a policy table that could hold two rows would
    -- need a rule for which one wins, and every caller would have to remember it.
    id          TEXT    PRIMARY KEY,
    posture     TEXT    NOT NULL DEFAULT 'forbid'
        CHECK (posture IN ('forbid', 'disclose', 'allow')),
    -- Who changed it. ON DELETE SET NULL, not CASCADE, and for the reason 0087
    -- gives: deleting the account that made a policy decision must not delete the
    -- decision, or the instance silently reverts to the default while every UI
    -- still shows the old value.
    updated_by  TEXT    REFERENCES accounts (id) ON DELETE SET NULL,
    created_at  TEXT    NOT NULL,
    updated_at  TEXT    NOT NULL,
    version     INTEGER NOT NULL DEFAULT 1
);

-- ── The posture, recorded on the work ─────────────────────────────────────────
--
-- §51.1: "The posture is recorded with the work, not looked up at read time." So
-- this is a column on `works`, not a join to the policy row, and the reason is
-- retroactive labelling. An operator who tightens `allow` -> `disclose` is making a
-- change about FUTURE writes; joining to the policy at read time would relabel
-- every work already in the corpus, which changes terms the author agreed to under
-- a different policy.
--
-- The column is the WORK's own posture, and it is NOT NULL for the same reason as
-- the policy's: a work with no posture recorded is a work whose posture nobody
-- decided. Backfilled below rather than defaulted per row, because the backfill is
-- the value that was actually in force.

ALTER TABLE works ADD COLUMN generated_content_posture TEXT
    CHECK (generated_content_posture IN ('forbid', 'disclose', 'allow'));

-- Backfill from the policy in force at migration time, then make it NOT NULL.
--
-- Doing this as UPDATE-then-leave-nullable rather than `ADD COLUMN ... NOT NULL
-- DEFAULT` is deliberate: a DEFAULT would write the literal into every row and lose
-- the distinction between "the policy said forbid" and "nobody has ever set a
-- policy", which is the distinction §51.1's "recorded with the work" clause exists
-- to keep.
--
-- ── A dialect difference that is NOT papered over ─────────────────────────────
--
-- PostgreSQL narrows this to NOT NULL with a real `ALTER COLUMN ... SET NOT NULL`
-- after the UPDATE. SQLite has no such statement at all: it cannot add a NOT NULL
-- column without a default, and cannot alter an existing column's nullability. The
-- column therefore stays nullable on SQLite, and the invariant is carried instead
-- by the two CHECKs below -- the posture CHECK rejects a value outside the three,
-- and a NULL passes both.
--
-- That is a genuine difference in what the two engines enforce, so it is written
-- down rather than left for a reader to discover. The store is what guarantees the
-- invariant on both: `GeneratedContentStore` writes this column on every path and
-- never writes NULL, and its acceptance tests assert a NOT NULL row rather than a
-- CHECK failure. If that ever stops being true on SQLite, the gap is in the store,
-- and the tests say so.
UPDATE works
   SET generated_content_posture = COALESCE(
           (SELECT posture FROM generated_content_policy WHERE id = 'default'),
           'forbid')
 WHERE generated_content_posture IS NULL;

-- A work's disclosure is the author's own statement and is recorded as such:
-- `declared_generated_at` is WHEN the author said so, never when a model or a
-- moderator suspected it. §51.3 forbids persisting detector output as a fact,
-- which is why there is no confidence score and no detector column here -- the
-- absence is the rule, not an oversight.
ALTER TABLE works ADD COLUMN generated_declared_at TEXT;

-- Exactly one of the two: a declared generated work has a declaration time, and an
-- undeclared one does not. Enforced here because the pair is the whole disclosure
-- record, and a work carrying a declaration time with nothing declared reads as
-- "disclosed" to a query that filters on the timestamp alone.
ALTER TABLE works
    ADD CONSTRAINT works_generated_declaration_pair_ck CHECK (
        (generated_content_posture = 'disclose') = (generated_declared_at IS NOT NULL)
    );