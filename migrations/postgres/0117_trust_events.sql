-- 0117: the trust event log (M3-03).
--
-- Spec §6.7, §23.1. A trust level that changed with no event is a defect, and
-- this table is where the proof lives.
--
-- WHY AN EVENT LOG AND NOT A COLUMN
--
-- `trust_levels` says what a level *is* now, which is the question the tier
-- gate asks on every request. It cannot say how the account got there, and it
-- cannot say that a level was *taken away*. A level that moves without a
-- record is indistinguishable from a level that was always that, and a reader
-- whose level fell deserves to be able to see that it fell and why.
--
-- So the levels table holds the current value and this table holds the
-- history. **One implementation per fact**: the current level is one fact and
-- the history is another, and this is the second.
--
-- APPEND-ONLY, AND ENFORCED
--
-- The table refuses an UPDATE and a DELETE, on both engines, with triggers
-- rather than with a comment. A trust history that can be rewritten is not a
-- history — and the estate's rule is that a property which is only stated is
-- a property which will be broken by the next person who does not read the
-- comment.
--
-- The triggers are deliberately *not* on the delete of the account: a reader
-- who is erased leaves their trust history behind, exactly as they leave the
-- audit log behind, because the history is about what the instance did and
-- not about the person who is gone. There is no foreign key to `accounts` for
-- the same reason: the event has to outlive the row it is about.
--
-- THE CAUSE VOCABULARY IS CLOSED
--
-- `cause` is a CHECK, not a free-text field. The causes are the four axes
-- trust is earned along, the three reasons a level can move without the axes
-- moving, and the fold:
--
--   tenure             the tenure axis moved the level
--   contribution       the accepted-contributions axis moved it
--   vote_score         the vote-score axis moved it
--   report_accuracy    the report-accuracy axis moved it
--   elastic            the privilege's requirement fell because nobody held it
--   demotion_spam      the one automatic demotion (spec §14.4)
--   operator           a person granted or took a level
--   fold               the account was folded and its trust was not carried over
--
-- A CHECK rather than an enum because this schema has no enum type and no
-- dialect-specific type would survive the parity check — and a closed
-- vocabulary that the database enforces is worth more than one the code
-- remembers to check.

CREATE TABLE trust_events (
    id               UUID    PRIMARY KEY,
    -- The account the event is about. **No foreign key**: the history has to
    -- outlive the row it is about, because an erased reader's trust history
    -- is a record of what the instance did and not of the person who is gone.
    account_id       UUID    NOT NULL,
    -- The level before the event, and the level after it.
    --
    -- A grant that does not move the level (an elastic allocation) records the
    -- same level twice: the event is about *what the account may do*, not
    -- about what it is.
    from_level       BIGINT  NOT NULL,
    to_level         BIGINT  NOT NULL,
    cause            TEXT    NOT NULL
        CHECK (cause IN ('tenure', 'contribution', 'vote_score', 'report_accuracy',
                         'elastic', 'demotion_spam', 'operator', 'fold')),
    -- What the event is about: a proposal id, a report id, or the name of the
    -- privilege an elastic allocation granted.
    evidence_ref     TEXT,
    -- Who caused it, when a person did: the operator who granted a level, the
    -- moderator who lifted a sanction.
    --
    -- No foreign key for the same reason as `account_id`: an operator's
    -- account can be erased, and the action they took should not vanish with
    -- them.
    actor_account_id UUID,
    created_at       TEXT    NOT NULL
);

-- The account's own trust history, newest first. The read is always "this
-- account's events, in order", so the index is on exactly that.
CREATE INDEX trust_events_account_created ON trust_events (account_id, created_at);

-- The modlog's own read: everybody's events, newest first.
CREATE INDEX trust_events_created ON trust_events (created_at);

-- APPEND-ONLY, ENFORCED.
--
-- The trigger raises rather than silently ignoring, so a caller that tries to
-- rewrite history gets an error instead of a quiet no-op — and a quiet no-op
-- is how a trust history becomes a trust *state* again.
CREATE FUNCTION trust_events_refuse_change() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'trust_events is append-only';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trust_events_refuse_update
BEFORE UPDATE ON trust_events
FOR EACH ROW EXECUTE FUNCTION trust_events_refuse_change();

CREATE TRIGGER trust_events_refuse_delete
BEFORE DELETE ON trust_events
FOR EACH ROW EXECUTE FUNCTION trust_events_refuse_change();
