-- §51: generated-content posture, and author credits that vest on reader
-- completion. M45-12, from gaps review A3.
--
-- Dialect: PostgreSQL. The rules are identical to the SQLite dialect's, and only the
-- mechanics of `works.updated_by`'s reference type and the CHECK forms differ, and
-- both dialects are verified to accept and reject the same six row shapes.
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
-- out loud rather than arriving there by omission.
--
-- NOT NULL with a CHECK rather than a nullable column: there is no state in which
-- this instance has no generated-content posture, and a NULL would be a fourth
-- value meaning "nobody has decided", which is `forbid`'s job.

CREATE TABLE IF NOT EXISTS generated_content_policy (
    id          TEXT    PRIMARY KEY,
    posture     TEXT    NOT NULL DEFAULT 'forbid'
        CHECK (posture IN ('forbid', 'disclose', 'allow')),
    -- Who changed it. ON DELETE SET NULL, not CASCADE, and for the reason 0087
    -- gives: deleting the account that made a policy decision must not delete the
    -- decision, or the instance silently reverts to the default while every UI
    -- still shows the old value.
    updated_by  UUID    REFERENCES accounts (id) ON DELETE SET NULL,
    created_at  TEXT    NOT NULL,
    updated_at  TEXT    NOT NULL,
    version     INTEGER NOT NULL DEFAULT 1
);

-- ── The posture, recorded on the work ─────────────────────────────────────────
--
-- §51.1: "The posture is recorded with the work, not looked up at read time." So
-- this is a column on `works`, not a join to the policy row, and the reason is
-- retroactive labelling. An operator who tightens `allow` -> `disclose` is making a
-- change about FUTURE writes, and joining to the policy at read time would relabel
-- every work already in the corpus, which changes terms the author agreed to under
-- a different policy.
--
-- The column is added NULLABLE, backfilled from the policy in force, then narrowed --
-- and it carries a DEFAULT of 'forbid' the whole way.
--
-- CORRECTION (2026-10-02). The first version of this migration deliberately set no
-- DEFAULT, on the reasoning that "a DEFAULT would write the literal into every row
-- and lose the distinction between 'the policy said forbid' and 'nobody has ever
-- set a policy'". That reasoning was wrong about what it protected: the policy
-- TABLE's own default is already 'forbid', so the DEFAULT and the backfill agree for
-- every row that has no policy -- there was no distinction to lose.
--
-- The real cost was this. `generated_content_posture` became NOT NULL with nothing to
-- supply it, so every INSERT that omitted the column failed on PostgreSQL with 23502
-- while SQLite stored NULL and carried on. 39 app tests passed on one engine and
-- failed on the other, across 74 test fixtures plus the production INSERT in
-- content.rs. Neither engine was wrong; only one of them enforced the constraint.
--
-- The lesson is worth more than the fix: a NOT NULL column with no DEFAULT is a
-- per-dialect trap, because SQLite's affinity accepts the NULL that PostgreSQL
-- rejects. Either every writer names the column, or the DEFAULT carries the answer.

ALTER TABLE works ADD COLUMN generated_content_posture TEXT DEFAULT 'forbid';

ALTER TABLE works
    ADD CONSTRAINT works_generated_content_posture_ck
    CHECK (generated_content_posture IN ('forbid', 'disclose', 'allow'));

-- The disclosure record: WHEN the author said so, never when a model or a moderator
-- suspected it. §51.3 forbids persisting detector output as a fact, which is why
-- there is no confidence score and no detector column here -- the absence is the
-- rule, not an oversight.
ALTER TABLE works ADD COLUMN generated_declared_at TEXT;

-- Backfill from the policy in force at migration time, then narrow.
--
-- UPDATE-then-narrow rather than a DEFAULT, because a DEFAULT would write the
-- literal into every row and lose the distinction between "the policy said forbid"
-- and "nobody has ever set a policy" -- the distinction §51.1's "recorded with the
-- work" clause exists to keep.
UPDATE works
   SET generated_content_posture = COALESCE(
           (SELECT posture FROM generated_content_policy WHERE id = 'default'),
           'forbid')
 WHERE generated_content_posture IS NULL;

ALTER TABLE works
    ALTER COLUMN generated_content_posture SET NOT NULL;

-- ── The pair rule, and why it covers BOTH accepting postures ──────────────────
--
-- A work the author declared generated carries a declaration time; a work the
-- author declared nothing about does not. The rule spans `disclose` and `allow`,
-- because the DECLARATION is the author's statement and is recorded either way --
-- what differs between those two postures is only whether the marker is SHOWN.
--
-- The first version of this constraint tested
-- `(posture = 'disclose') = (declared_at IS NOT NULL)`, which is wrong, and the
-- store caught it: under `allow` a declared work is accepted with no declaration
-- time, so the author's statement was silently discarded. §51.1's "recorded with
-- the work" is about the posture AND the declaration -- storing the posture while
-- dropping what the author said about it would make `allow` a way to erase a
-- declaration rather than a way not to display one.
--
-- The error this prevents on the read side is specific too: a disclosure marker
-- rendered from `generated_declared_at IS NOT NULL` would then show on an
-- `allow` work, which is §51.1's "allow accepts and does not label" broken.
-- Same rule as the SQLite dialect's triggers: a declaration time may not coexist
-- with `forbid`, and nothing else is constrained -- see that file for the three
-- wrong pairings this replaced and what each one refused.
ALTER TABLE works
    ADD CONSTRAINT works_generated_declaration_pair_ck CHECK (
        generated_declared_at IS NULL OR generated_content_posture <> 'forbid'
    );