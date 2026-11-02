-- 0136 — §18.11 gift exchanges: terms and the pairing (M45-27).
--
-- Why ALTER was not enough: `events` (0014) is a name, a document and a creation
-- record. An exchange needs two facts the document cannot hold — what each
-- participant declared, and who is writing for whom — and the second is a row
-- that must be unique per recipient, which no existing table can express.
--
-- Why ONE `exchange_terms` table and not three: offers, wants and exclusions
-- share their entire shape (event, account, value, time) and differ by one word.
-- A closed CHECK keeps the vocabulary from drifting the way a comma-separated
-- list did before M6-05 dropped it.
--
-- No foreign keys, for the reason supply_routing states: §51 erases an account by
-- REKEYING it rather than deleting the row, so ON DELETE CASCADE would never
-- fire and a FK would turn an erasure into a failed statement. `account` is
-- declared rekey_account and `event_id` keep in docs/snapshot-column-policy.json.
--
-- Timestamps are canonical `YYYY-MM-DDTHH:MM:SSZ` (M63-31, migration 0132).
-- `matched_by` records which algorithm produced the pairing, because "why is
-- this person giving to that one" is unanswerable later from the pair alone.

CREATE TABLE IF NOT EXISTS exchange_terms (
    event_id   TEXT NOT NULL,
    account    TEXT NOT NULL,
    kind       TEXT NOT NULL,
    value      TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (event_id, account, kind, value),
    CONSTRAINT exchange_terms_kind_check
        CHECK (kind IN ('offers_fandom', 'wants_fandom', 'excludes_pseud'))
);

-- The matcher reads every term of one event at once; this is that scan.
CREATE INDEX IF NOT EXISTS idx_exchange_terms_event
    ON exchange_terms (event_id, account);

CREATE TABLE IF NOT EXISTS exchange_assignments (
    event_id    TEXT NOT NULL,
    giver       TEXT NOT NULL,
    recipient   TEXT NOT NULL,
    assigned_at TEXT NOT NULL,
    revealed_at TEXT,
    matched_by  TEXT NOT NULL,
    PRIMARY KEY (event_id, giver),
    CONSTRAINT exchange_assignments_matched_by_check
        CHECK (matched_by IN ('forced', 'augment', 'improved')),
    CONSTRAINT exchange_assignments_not_self
        CHECK (giver <> recipient)
);

-- One giver per recipient is the invariant, enforced here rather than trusted to
-- the matcher: a bug that shipped a double gift would be unrecoverable.
CREATE UNIQUE INDEX IF NOT EXISTS idx_exchange_assignments_recipient
    ON exchange_assignments (event_id, recipient);
