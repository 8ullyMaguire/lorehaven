-- 0118: the modlog's actor, its reason, and the elastic requirements (M3-04,
-- M3-05).
--
-- Spec §14.6: "Every moderator, curator and operator action is recorded and
-- **readable by any signed-in reader**, with actor, action, target, reason and
-- timestamp."
--
-- THE ACTOR IS A PSEUDONYM, NOT AN ACCOUNT
--
-- `audit_log.actor` holds the **account** that acted, and that is right for an
-- audit: the audit is the instance's own record and it must be able to say
-- which account did the thing. But the **modlog** is a *public* surface, and
-- publishing "account 3f2a… issued a sanction" would publish the account id
-- that every other public surface refuses to publish (M2-07). A reader who
-- cannot link a pseudonym to an account cannot be traced through the modlog,
-- and the modlog is exactly where a trace would happen.
--
-- So the acting pseudonym is recorded alongside the account, and **the modlog
-- publishes the pseudonym and never the account**. The two are both there:
-- the account is the instance's record, the pseudonym is the public face.
--
-- The column is nullable because the audit log also records things with no
-- pseudonym behind them — a scheduled job, an import, a system action — and
-- **a null pseudonym is honest** where a made-up one would not be.
--
-- THE REASON IS ITS OWN COLUMN
--
-- The `document` column is the action's detail, and it is a JSON blob whose
-- shape differs per action. A modlog entry that had to dig its reason out of
-- a blob would have a reason in some entries and not in others, and the
-- contract says *every* entry has one. So the reason is a column: empty when
-- the action had none, never absent.

ALTER TABLE audit_log ADD COLUMN actor_pseud TEXT;
ALTER TABLE audit_log ADD COLUMN reason TEXT;

-- The modlog's own read: newest first, by action and by time.
CREATE INDEX audit_log_created ON audit_log (created_at);

-- ---------------------------------------------------------------------------
-- The elastic requirements (M3-04).
--
-- Spec §14.4: "When a privilege's eligible pool is **empty**, the requirement
-- falls until at least `trust.elastic_min_holders` holders exist — but never
-- below `trust.floor_level`."
--
-- The requirement is **stored, not computed per request**, for two reasons.
--
-- 1. **The cost.** The pool is a count over every account's trust level, and
--    a trust gate is asked on every request a reader makes that they are not
--    allowed to make. Recounting the pool per refused request would make the
--    refusal more expensive than the thing being refused.
-- 2. **"The next evaluation."** The specification says an elastic grant is
--    re-evaluated, and re-evaluated means *at a moment*, not *continuously*.
--    A stored requirement is evaluated when a trust level changes, which is
--    when the pool can have changed.
--
-- And storing it is also what makes the **no ratchet** property true by
-- construction: the requirement is a function of the current pool and of
-- nothing else, so it has no memory of what it decided last time. A site that
-- grows a level-4 account stops handing the queue to a level-2 on the next
-- evaluation, and the level-2 loses the privilege **without anybody deciding
-- to take it**.
--
-- One row per privilege. `basis` says whether the requirement is the ladder's
-- own (`nominal`) or fell because nobody held the privilege (`elastic`), and
-- `holders` is the count the decision was made on — so a reader who asks why
-- the queue is open can be told.

CREATE TABLE privilege_requirements (
    -- The privilege's name, from the closed vocabulary in the domain.
    privilege    TEXT    PRIMARY KEY,
    -- The level the privilege requires, right now.
    level        INTEGER NOT NULL,
    -- Whether that level is the ladder's own or fell because nobody held it.
    basis        TEXT    NOT NULL CHECK (basis IN ('nominal', 'elastic')),
    -- How many accounts hold the privilege at the level or above, as of the
    -- evaluation.
    holders      INTEGER NOT NULL DEFAULT 0,
    -- When the requirement was last evaluated.
    evaluated_at TEXT    NOT NULL
);
