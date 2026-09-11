-- 0009 — the reader's library (spec §16 as the plan numbers it, §14 in the spec
-- text; the plan's milestone 8, tag `v0.09-library`).
--
-- This migration adds *the reader's own organisation of content*: shelves,
-- bookmarks, private tags, reading statuses, saved views and the record of an
-- update check. It does not add anything public, and nothing here is joined
-- into a query another account can reach.
--
-- Two boundaries are worth stating where they cannot be missed:
--
--   * `private_tags` is not the public taxonomy. A public tag lives in M9's
--     `work_tags` and belongs to a work; a row here belongs to an account and
--     is visible to nobody else. A single `tags` table with an `is_private`
--     flag would be leaked by the first query that forgets the flag, which is
--     why these are two tables in two migrations rather than one table with a
--     discriminator.
--   * Everything in this file cascades with the account. Deleting an account
--     takes its shelves, bookmarks, tags, statuses and views with it.
--
-- Identifiers are native UUID columns; timestamps are RFC 3339 TEXT; integers
-- the repository decodes as `i64` are BIGINT, because SQLite's INTEGER is
-- 64-bit and PostgreSQL's is not (see ADR 0004).

CREATE TABLE shelves (
    id          UUID    PRIMARY KEY,
    account_id  UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,
    description TEXT    NOT NULL DEFAULT '',
    -- A shelf is private until its owner says otherwise. The default is the
    -- private value, so a caller that forgets to set it publishes nothing.
    is_public   BOOLEAN NOT NULL DEFAULT FALSE,
    position    BIGINT  NOT NULL DEFAULT 0,
    created_at  TEXT    NOT NULL,
    updated_at  TEXT    NOT NULL,
    version     BIGINT  NOT NULL DEFAULT 1,
    UNIQUE (account_id, name)
);

CREATE INDEX shelves_account_order ON shelves (account_id, position);

CREATE TABLE shelf_items (
    id              UUID   PRIMARY KEY,
    shelf_id        UUID   NOT NULL REFERENCES shelves (id) ON DELETE CASCADE,
    -- The item is referenced, not owned: deleting a shelf removes the
    -- placement and leaves the library item alone (spec §14.1, "Deleting a
    -- shelf does not delete its works").
    library_item_id UUID   NOT NULL REFERENCES library_items (id) ON DELETE CASCADE,
    position        BIGINT NOT NULL DEFAULT 0,
    created_at      TEXT   NOT NULL,
    UNIQUE (shelf_id, library_item_id)
);

CREATE INDEX shelf_items_order ON shelf_items (shelf_id, position);

CREATE TABLE bookmarks (
    id                UUID    PRIMARY KEY,
    account_id        UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    subject_type      TEXT    NOT NULL,
    subject_id        UUID    NOT NULL,
    -- Where in the work the bookmark sits. Both parts are optional: a bookmark
    -- on the work itself has neither, and one on a chapter may have no position
    -- yet (someone marking a chapter to come back to).
    chapter_id        UUID    REFERENCES chapters (id) ON DELETE SET NULL,
    position_permille BIGINT,
    note              TEXT    NOT NULL DEFAULT '',
    -- Bookmarks default to private (spec §14.1). The column exists because
    -- spec §14's acceptance criteria require a public bookmark list to exclude
    -- private entries, which presupposes that a public one can exist.
    is_public         BOOLEAN NOT NULL DEFAULT FALSE,
    created_at        TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL,
    version           BIGINT  NOT NULL DEFAULT 1
);

CREATE INDEX bookmarks_account_subject ON bookmarks (account_id, subject_type, subject_id);

CREATE TABLE private_tags (
    id           UUID PRIMARY KEY,
    account_id   UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    subject_type TEXT NOT NULL,
    subject_id   UUID NOT NULL,
    tag          TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    -- The uniqueness is per account and per subject: two readers may both tag
    -- the same work "comfort-read" without sharing anything, and both rows are
    -- invisible to each other.
    UNIQUE (account_id, subject_type, subject_id, tag)
);

CREATE INDEX private_tags_tag ON private_tags (account_id, tag);

CREATE TABLE reading_status (
    id           UUID PRIMARY KEY,
    account_id   UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    subject_type TEXT NOT NULL,
    subject_id   UUID NOT NULL,
    status       TEXT NOT NULL
                      CHECK (status IN ('want-to-read', 'reading', 'on-hold',
                                        'dropped', 'finished')),
    started_at   TEXT,
    finished_at  TEXT,
    updated_at   TEXT NOT NULL,
    version      BIGINT NOT NULL DEFAULT 1,
    UNIQUE (account_id, subject_type, subject_id)
);

CREATE INDEX reading_status_status ON reading_status (account_id, status);

CREATE TABLE saved_views (
    id           UUID PRIMARY KEY,
    account_id   UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    -- The query, stored as a versioned JSON document so a filter language
    -- change can migrate or repair it rather than misread it (spec §14.2).
    -- Nothing queries inside this column.
    query_json   TEXT NOT NULL,
    query_version BIGINT NOT NULL DEFAULT 1,
    sort         TEXT NOT NULL DEFAULT 'recent',
    -- `library` or `public`. The scope is what decides whether the view is
    -- allowed to carry private filters at all; `validate_query` refuses to
    -- store a public view naming a shelf, a private tag or a reading status.
    scope        TEXT NOT NULL DEFAULT 'library' CHECK (scope IN ('library', 'public')),
    -- Pinned views appear in navigation and on the dashboard. Surviving a
    -- pseud switch is a client concern (the view belongs to the account).
    pinned       BOOLEAN NOT NULL DEFAULT FALSE,
    is_public    BOOLEAN NOT NULL DEFAULT FALSE,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    version      BIGINT NOT NULL DEFAULT 1,
    UNIQUE (account_id, name)
);

CREATE INDEX saved_views_account ON saved_views (account_id, pinned, name);

CREATE TABLE update_checks (
    id              UUID   PRIMARY KEY,
    account_id      UUID   NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    library_item_id UUID   NOT NULL REFERENCES library_items (id) ON DELETE CASCADE,
    checked_at      TEXT   NOT NULL,
    -- What the check found, and the per-item detail behind that number. The
    -- count is what a list renders; the report is what a detail page can show
    -- without re-fetching the source.
    found_changes   BIGINT NOT NULL DEFAULT 0,
    report_json     TEXT   NOT NULL DEFAULT '{}'
);

-- Retention: 90 days. The sweep is a job, not a trigger, so an instance that
-- stops running sweeps accumulates rows rather than losing them.
CREATE INDEX update_checks_retention ON update_checks (checked_at);
CREATE INDEX update_checks_item ON update_checks (library_item_id, checked_at DESC);
