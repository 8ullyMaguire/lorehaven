-- M59-10: a reader's own copy of an external body (spec §11.15b).
--
-- Dialect: PostgreSQL.
--
-- The counterpart of migrations/sqlite/0095_reader_body_copies.sql. Tables,
-- columns and indexes must match, because
-- `the_two_dialects_declare_the_same_columns_and_indexes` in
-- crates/db/src/migrate.rs compares exactly that. What differs between the two
-- files is the reference *types*, and 0094's header explains the rule this
-- migration follows: a column that references `accounts(id)` is TEXT in the
-- SQLite file and UUID here, so THIS file takes the cast in the DDL and the
-- other does not.
--
-- WHY THREE TABLES. `reader_body_copies` is the bytes and the state of getting
-- them; `retention_body_requests` is the record §6.2 requires of who asked and
-- when. They are separate because a copy outlives its request's relevance: a
-- reader who fetched a body in March should still have it in September without
-- a March row still being the current state, and a request log that was also
-- the storage table could not say either thing cleanly.
--
-- A COPY IS PER-READER AND THE READ PATH DOES NOT CONSULT IT. This is §6.2's
-- last bullet, and it is not visible in the DDL: the request "does not create a
-- readers'-tier around it". Joining this table into a read query "for
-- convenience" would make one reader's request narrow what every other reader
-- sees, which is the one thing §6.2 rules out. The read path reads the WORK.
--
-- `state` IS A JOB STATE, NOT A RETENTION DECISION. A `refused` row records that
-- *this request* was refused, with the code that refused it. It says nothing
-- about the work's retention mode afterwards: a reader refused once, whose
-- source was later unblocked, gets a fresh decision on a fresh request.
-- `retention_policy_changes` remains the only record of a mode change, and its
-- `actor` is `NOT NULL` precisely because every row in it is a statement that
-- an account changed the instance's policy.
--
-- `source_key` IS RECORDED, NOT REFERENCED. There is no foreign key to
-- `library_items` because a work may have SEVERAL library items -- a re-import
-- from a mirror adds one -- and which one a request belongs to is a decision the
-- route makes (the most recent non-deleted row) rather than a constraint the
-- database can express. Recording the key also means a work re-imported from a
-- different source later does not silently re-point an in-flight request at
-- another site's terms.
--
-- `chapter_key` IS THE CHAPTER LIST AT REQUEST TIME, not a reference to
-- `chapter_revisions`. A re-import may replace the revisions underneath a copy,
-- and a reader's copy should keep the bytes they asked for rather than silently
-- re-point at whatever the work holds now.
--
-- The unique index on (work_id, account_id) is what makes a second request a
-- re-fetch rather than a second row. Without it a reader who asks twice has two
-- blobs and the read path has to choose, and "prefers the older" is a bug nobody
-- writes a test for.

CREATE TABLE IF NOT EXISTS reader_body_copies (
    id             UUID    PRIMARY KEY,
    work_id        UUID    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    account_id     UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    source_key     TEXT    NOT NULL,
    chapter_key    TEXT    NOT NULL,
    state          TEXT    NOT NULL,          -- pending | ready | refused | failed
    reason_code    TEXT,                      -- a RetentionReason code, when refused
    plain_text     TEXT,                      -- the bytes, once fetched
    sanitized_html TEXT,
    requested_at   TEXT    NOT NULL,
    settled_at     TEXT,
    created_at     TEXT    NOT NULL,
    updated_at     TEXT    NOT NULL,
    version        INTEGER NOT NULL DEFAULT 1
);

-- One copy per (work, reader). A second request is a re-fetch.
CREATE UNIQUE INDEX IF NOT EXISTS idx_reader_body_copies_work_account
    ON reader_body_copies (work_id, account_id);

-- The audit surface §6.2 asks for. `trust_at_request` is recorded rather than
-- re-derived because trust moves: a record of who asked at what standing is only
-- honest if it keeps the standing they held at the time.
CREATE TABLE IF NOT EXISTS retention_body_requests (
    id              UUID    PRIMARY KEY,
    work_id         UUID    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    account_id      UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    source_key      TEXT    NOT NULL,
    trust_at_request INTEGER NOT NULL,
    requested_at    TEXT    NOT NULL,
    outcome         TEXT    NOT NULL,         -- pending | ready | refused | failed
    reason_code     TEXT
);

CREATE INDEX IF NOT EXISTS idx_retention_body_requests_account
    ON retention_body_requests (account_id, requested_at);
