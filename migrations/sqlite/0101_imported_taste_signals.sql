-- Migration 0101 — imported taste signals, distinguishable from organic ones
-- (spec §49.6, §49.7, M45-20).
--
-- Dialect: SQLite.
--
-- WHY A NEW TABLE AND NOT A COLUMN ON `arena_weights`
-- -----------------------------------------------------
-- The obvious place for an import's provenance is the table the arena already
-- writes. That table cannot hold it. `arena_weights` (0066) is
--
--     UNIQUE (account_id, dimension_key)
--
-- one row per reader per dimension, holding the *aggregate*. Twenty imported
-- bookmarks and twenty ratings given today both land on the same row, so a
-- `signal_origin` column there would have to be one value for a mixed history
-- (a lie) or a delimited list (a convention wearing a column's clothes). §49.6
-- asks for imported signals to be "distinguishable ... everywhere they are
-- read", and a reader's history is made of individual signals; the aggregate is
-- not where that question can be answered.
--
-- So the origin is recorded on the *signal*, and the aggregate stays an
-- aggregate. The read path joins the two, which is what makes the distinction
-- reachable from an export or an access request rather than only at write time.
--
-- WHY THE UNIQUENESS KEY INCLUDES THE EXTERNAL ID
-- ----------------------------------------------
-- §49.8's acceptance clause is "importing the same history twice leaves the
-- profile identical to importing it once", and the plan puts it plainly: "the
-- uniqueness key must include the external id". So idempotency is a database
-- guarantee rather than a property the importer has to maintain — a re-import
-- collides on the key and the second run is a no-op, whatever order it runs in
-- and however many times. `source_signal_key` is the source's own stable id for
-- the item (an AO3 bookmark id, an FFN id, a Goodreads row), which is the only
-- thing that survives a re-fetch.
CREATE TABLE IF NOT EXISTS taste_signals (
    id                 TEXT    PRIMARY KEY,
    account_id         TEXT    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- 'organic' for a signal this instance collected, 'imported' for one that
    -- arrived through §49.6's import. A CHECK rather than a convention, because
    -- §49.7's invariant is that the two stay distinguishable *everywhere* --
    -- including in exports -- and a typo'd origin is exactly the failure a
    -- constraint catches and a comment does not.
    origin             TEXT    NOT NULL
                               CHECK (origin IN ('organic', 'imported')),
    -- What kind of signal it is. Kept as text with a CHECK rather than an enum:
    -- the spec names these as a set that will grow with new sources, and SQLite
    -- has no enum type, so a CHECK is the only spelling that means the same
    -- thing on both dialects.
    signal_kind        TEXT    NOT NULL
                               CHECK (signal_kind IN ('bookmark', 'kudos',
                                                      'rating', 'read')),
    -- The dimension this signal bears on, and the value it carries, in the same
    -- units the arena uses. A bookmark and a kudos are both weak positives and
    -- both are expressed here rather than as a pre-multiplied weight, so the
    -- discount in §49.6 is applied once, in one place, at read time.
    dimension_key      TEXT    NOT NULL,
    signal_value       REAL    NOT NULL,
    -- When the signal happened, as the source reported it. NULL rather than a
    -- default: a source that does not date its items must not be recorded as
    -- having dated them today, and §49.6's "weaker evidence" argument is about
    -- age, so inventing a date would corrupt the one field the clause is about.
    occurred_at        TEXT,
    -- The source's own stable key. NOT NULL only for imported signals, which
    -- §49.6 requires to be idempotent, and the uniqueness below depends on it.
    source_key         TEXT    NOT NULL DEFAULT '',
    source_signal_key  TEXT    NOT NULL DEFAULT '',
    -- Free-form provenance (adapter version, fetched URL). The same
    -- provenance_json convention `library_items` uses (0006).
    provenance_json    TEXT    NOT NULL DEFAULT '{}',
    created_at         TEXT    NOT NULL,
    updated_at         TEXT    NOT NULL,
    -- The idempotency guarantee, in the database. Including the external id is
    -- what makes a re-import a no-op rather than a duplicate, and including
    -- account_id and dimension_key is what keeps two readers' identical
    -- bookmarks from colliding. A signal with no external id (every organic
    -- one) collapses to the empty string here, so two organic signals of the
    -- same kind on the same dimension for the same reader are NOT duplicates
    -- and both are kept -- which is correct, and is why this is a UNIQUE
    -- constraint on a four-column key rather than a partial index on the
    -- imported rows alone: the organic side would otherwise have no key at all.
    UNIQUE (account_id, source_key, source_signal_key, dimension_key)
);

-- The import side reads by external key to decide what is new; the taste side
-- reads by account to answer "what is this reader's profile made of".
CREATE INDEX IF NOT EXISTS idx_taste_signals_account
    ON taste_signals (account_id, dimension_key);
CREATE INDEX IF NOT EXISTS idx_taste_signals_import
    ON taste_signals (account_id, source_key, source_signal_key)
    WHERE origin = 'imported';

-- The re-runnable unit: which source produced which batch, and what it landed.
-- §49.6 requires re-running to be safe, and an audit of "what did the second
-- import add" is only possible if the attempt itself is a row. `signals_added`
-- is what makes idempotency *observable* rather than merely asserted: the second
-- run records 0 added, which is the property the acceptance clause names.
CREATE TABLE IF NOT EXISTS taste_signal_imports (
    id             TEXT    PRIMARY KEY,
    account_id     TEXT    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    source_key     TEXT    NOT NULL,
    -- The `source_credentials` row this ran through, or NULL when the reader
    -- supplied the export directly. §49.6 says imports go through M53's existing
    -- credentials; allowing NULL is not a second path into the importer, it is
    -- the case where there was no credential because the data arrived another
    -- way, and recording that as NULL is more honest than inventing one.
    credential_id  TEXT,
    -- 'bookmarks' | 'kudos' | 'read' | 'mixed' -- what this batch claimed to be.
    import_kind    TEXT    NOT NULL
                           CHECK (import_kind IN ('bookmarks', 'kudos',
                                                  'read', 'mixed')),
    signals_seen   INTEGER NOT NULL DEFAULT 0,
    -- How many rows this run actually inserted. 0 on a re-import, and that is
    -- the number the idempotency test asserts on.
    signals_added  INTEGER NOT NULL DEFAULT 0,
    started_at     TEXT    NOT NULL,
    finished_at    TEXT
);

CREATE INDEX IF NOT EXISTS idx_taste_signal_imports_account
    ON taste_signal_imports (account_id, started_at DESC, id DESC);

-- §51.4 AND THE CACHED BODY
-- --------------------------
-- This migration deliberately does NOT touch `reader_body_copies` (0095), and
-- that omission is the design rather than an oversight. §49.6 refuses to
-- *violate* §51.4's visibility default and names M45-53 as its owner; M45-53 is
-- `planned`. So the import writes metadata and a link and routes the body through
-- the existing private-copy path, and the acceptance clause "an imported
-- bookmark makes the work's metadata visible and leaves its cached body private"
-- is enforced by a test that observes the outcome without this migration having
-- chosen it. The import must not be the place that decides the default.
