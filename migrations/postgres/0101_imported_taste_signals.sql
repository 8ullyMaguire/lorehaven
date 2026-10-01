-- Migration 0101 — imported taste signals, distinguishable from organic ones
-- (spec §49.6, §49.7, M45-20).
--
-- Dialect: PostgreSQL.
--
-- Mirrors migrations/sqlite/0101_imported_taste_signals.sql statement for
-- statement, in the same order, with the same columns and the same CHECKs. The
-- full reasoning is in the SQLite file; this one carries only the notes that are
-- specific to this engine. The parity test compares declared column SETS and
-- index column lists and is blind to types, so the two files are matched by hand
-- and the store carries the casts -- the failure mode migration 0100's comment
-- describes.
--
-- Type notes, each one read from the sibling table rather than assumed:
--
--   * `taste_signals.id` is TEXT, not UUID. `tasting_samples.id` and
--     `tasting_responses.id` (0099) are both TEXT on both dialects, and a signal
--     is the same kind of object as a tasting response. Using UUID here and TEXT
--     there would mean every cross-join between them needs a cast, for no gain.
--   * `account_id` is **UUID**, and this one was got wrong first. The draft
--     declared it TEXT, copied from `tasting_samples.account_id` (0099), and
--     migration 0101 then failed to apply on PostgreSQL with
--
--         foreign key constraint "taste_signals_account_id_fkey" cannot be implemented
--
--     because a TEXT column cannot reference a UUID column. The rule the schema
--     actually follows, checked against the two precedents rather than guessed:
--     **every** `account_id` that carries a REFERENCES to `accounts(id)` is UUID
--     here -- `library_items` (0006) and `reader_body_copies` (0095) both are --
--     and the TEXT ones (`tasting_samples`, `tasting_responses`, 0099) carry no
--     foreign key at all. So the type follows the foreign key, not the
--     neighbouring table, and the cost of getting it backwards is a migration
--     that will not apply on half the engines it declares.
--
--     Which means the store's casts are *not* uniform within this feature, and
--     that is worth stating plainly: `taste_signals` and `taste_signal_imports`
--     need `$n::uuid` on their account columns, while 0099's tasting tables need
--     none. crates/db/src/taste_import.rs says so at each query rather than
--     leaving the next reader to work it out from a failed migration.
--   * `signal_value` is DOUBLE PRECISION to match `arena_weights.weight` (0066),
--     NOT `REAL` -- which is not a PostgreSQL type at all, and using it would
--     fail only here.
--   * `created_at`/`updated_at`/`occurred_at` are TEXT, matching 0099's
--     `tasting_samples.created_at` and `tasting_responses.created_at`, which are
--     TEXT on **both** dialects. An earlier draft of migration 0100 asserted a
--     TIMESTAMPTZ asymmetry that does not exist, and reading 0099 is what caught
--     it. RFC 3339 strings on both sides mean the store binds one value and no
--     timestamp cast appears anywhere in the importer.
--   * `signals_seen`/`signals_added` are INTEGER, matching 0006's
--     `library_items.word_count INTEGER` style for counts, not BIGINT.
CREATE TABLE IF NOT EXISTS taste_signals (
    id                 TEXT    PRIMARY KEY,
    account_id         UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    origin             TEXT    NOT NULL
                               CHECK (origin IN ('organic', 'imported')),
    signal_kind        TEXT    NOT NULL
                               CHECK (signal_kind IN ('bookmark', 'kudos',
                                                      'rating', 'read')),
    dimension_key      TEXT    NOT NULL,
    signal_value       DOUBLE PRECISION NOT NULL,
    occurred_at        TEXT,
    source_key         TEXT    NOT NULL DEFAULT '',
    source_signal_key  TEXT    NOT NULL DEFAULT '',
    provenance_json    TEXT    NOT NULL DEFAULT '{}',
    created_at         TEXT    NOT NULL,
    updated_at         TEXT    NOT NULL,
    UNIQUE (account_id, source_key, source_signal_key, dimension_key)
);

CREATE INDEX IF NOT EXISTS idx_taste_signals_account
    ON taste_signals (account_id, dimension_key);

-- Partial, matching the SQLite file: only imported rows are looked up by their
-- external key, and keeping organic rows out of this index keeps it small.
CREATE INDEX IF NOT EXISTS idx_taste_signals_import
    ON taste_signals (account_id, source_key, source_signal_key)
    WHERE origin = 'imported';

CREATE TABLE IF NOT EXISTS taste_signal_imports (
    id             TEXT    PRIMARY KEY,
    account_id     UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    source_key     TEXT    NOT NULL,
    credential_id  TEXT,
    import_kind    TEXT    NOT NULL
                           CHECK (import_kind IN ('bookmarks', 'kudos',
                                                  'read', 'mixed')),
    signals_seen   INTEGER NOT NULL DEFAULT 0,
    signals_added  INTEGER NOT NULL DEFAULT 0,
    started_at     TEXT    NOT NULL,
    finished_at    TEXT
);

CREATE INDEX IF NOT EXISTS idx_taste_signal_imports_account
    ON taste_signal_imports (account_id, started_at DESC, id DESC);
