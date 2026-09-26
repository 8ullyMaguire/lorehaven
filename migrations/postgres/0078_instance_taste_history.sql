-- Migration 0078 — history for the instance's taste knobs (gaps review F1/F2).
--
-- F1 asks for taste as a *versioned* object and F2 for "config history and
-- rollback"; both are unsatisfiable while `instance_taste_settings` holds exactly
-- one row. An operator who changes the lens and regrets it has nothing to go
-- back to, and with a singleton the previous value is overwritten before anyone
-- can read it.
--
-- So every write appends here before it overwrites. That ordering is the whole
-- point: the history is written in the same transaction as the change, so a
-- crash between the two cannot leave a change with no record of what it
-- replaced.
--
-- A row is the *state before* the write, plus who wrote what over it. So
-- `rollback` is "copy the most recent history row back into the settings row",
-- and it is itself a write, so rolling back twice walks the history backwards
-- rather than toggling between two states.

CREATE TABLE instance_taste_settings_history (
    history_id            UUID    PRIMARY KEY,
    -- The version this row *replaced*. Null for the first version, which had
    -- nothing before it -- the config file, in other words.
    replaced_version      INTEGER,
    gravity_strength      INTEGER NOT NULL,
    signal_weight_mode    TEXT    NOT NULL,
    admin_weight          INTEGER NOT NULL,
    diversity_injection_percent INTEGER NOT NULL,
    -- The change that caused this row to be written, for an operator reading
    -- the history rather than rolling back.
    changed_by            UUID,
    changed_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- A rollback is itself a write, so it belongs in the history with its own
    -- author. Without this, a rollback is invisible in the audit trail and the
    -- history claims a change nobody made.
    rolled_back_from      UUID
);

CREATE INDEX instance_taste_settings_history_changed_at ON instance_taste_settings_history (changed_at);
CREATE INDEX instance_taste_settings_history_changed_by ON instance_taste_settings_history (changed_by);
