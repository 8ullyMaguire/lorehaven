-- 0137 — §56 WIP adoption, closure notes and lineage (M45-28).
--
-- No change to `works`, and that is the point: §56.1 says ownership never moves, and a
-- prohibition is kept by not having a path that could break it. There is no column here that
-- could be updated to move `works.owner_pseud_id`, and no route in the tree writes it.
--
-- NO foreign keys anywhere, and the reason is a type, not a preference: `works.id` and `pseuds.id`
-- are UUID on PostgreSQL while every id in this table is TEXT (the `welcome_rota` and
-- `supply_routing` convention). A `TEXT REFERENCES works(id)` fails with "foreign key constraint
-- cannot be implemented", which names the constraint rather than the type — the same wall
-- `work_character_roles` hit. Two honest ways out: make these columns UUID, or keep the house's
-- TEXT ids and no key. TEXT wins for consistency with 0135 and because it keeps the store from
-- parsing uuids on one engine and not the other. A dangling offer for a deleted work is harmless:
-- the listing joins `works`, and an offer with no work lists nowhere. The pseud columns would get
-- no key anyway, for the reason supply_routing states: §51 erases an account by REKEYING the pseud
-- rather than deleting it, so a cascade would never fire and a key would turn an erasure into a
-- failed statement.
--
-- The unique index on (work_id) WHERE status = 'open' is partial on purpose: one OPEN offer per
-- work, while withdrawn and adopted offers accumulate as history. A plain unique index would
-- forbid a second offer after a withdrawal, which contradicts "the author may withdraw at any
-- time, with no notice period".
--
-- Timestamps are canonical `YYYY-MM-DDTHH:MM:SSZ` (M63-31, migration 0132).

CREATE TABLE IF NOT EXISTS adoption_offers (
    id                  TEXT NOT NULL PRIMARY KEY,
    work_id             TEXT NOT NULL,
    offered_by_pseud_id TEXT NOT NULL,
    author_pseud_id     TEXT NOT NULL,
    adopted_by_pseud_id TEXT,
    status              TEXT NOT NULL DEFAULT 'open',
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    CONSTRAINT adoption_offers_status_check
        CHECK (status IN ('open', 'withdrawn', 'adopted'))
);

CREATE UNIQUE INDEX IF NOT EXISTS adoption_offers_open_work
    ON adoption_offers (work_id) WHERE status = 'open';

CREATE INDEX IF NOT EXISTS adoption_offers_status
    ON adoption_offers (status, created_at);

CREATE TABLE IF NOT EXISTS closure_notes (
    id              TEXT   NOT NULL PRIMARY KEY,
    work_id         TEXT   NOT NULL,
    revision_number BIGINT NOT NULL,
    body            TEXT   NOT NULL,
    author_pseud_id TEXT   NOT NULL,
    created_at      TEXT   NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS closure_notes_revision
    ON closure_notes (work_id, revision_number);
