-- M32-07e: proposed perceptual matches, for a curator to confirm or reject
-- (spec §32.7.2 "Deduplication behavior").
--
-- The exact-match branch needs no table: an identical content_hash attaches a
-- new availability link to the existing reference immediately. The perceptual
-- branch is the one that needs a human, and it needs somewhere to put the
-- candidate before a curator has seen it. dHash agrees across re-encodes and
-- mild edits, so a near-match is evidence, not proof; auto-merging on it would
-- be wrong and this row is the honest shape of that.
--
-- One row per (candidate, existing) pair. The distance is stored rather than
-- recomputed because the proposal outlives the fetch that found it, and the
-- curator sees the number the search actually used.
--
-- Dialect: PostgreSQL.
CREATE TABLE media_match_proposals (
    id                       UUID    PRIMARY KEY,
    -- The reference that was just fetched and hashed.
    candidate_reference_id   UUID    NOT NULL REFERENCES media_references (id) ON DELETE CASCADE,
    -- The existing reference this candidate appears to duplicate.
    existing_reference_id    UUID    NOT NULL REFERENCES media_references (id) ON DELETE CASCADE,
    -- The exact-match signal, kept so a curator can see why the two rows exist.
    content_hash             TEXT    NOT NULL,
    perceptual_hash          TEXT,
    -- The Hamming distance the search matched on, and the confidence derived
    -- from it at proposal time.
    hamming_distance         INTEGER NOT NULL,
    match_confidence         REAL    NOT NULL,
    -- pending | confirmed | rejected. A confirmed proposal has been linked; a
    -- rejected one is kept so the same pair is not re-proposed every fetch.
    status                   TEXT    NOT NULL DEFAULT 'pending'
                             CHECK (status IN ('pending', 'confirmed', 'rejected')),
    -- The curator who acted, and why. Both nullable: a pending row has neither.
    resolved_by              TEXT,
    resolution_note          TEXT,
    created_at               TEXT    NOT NULL,
    updated_at               TEXT    NOT NULL,
    resolved_at              TEXT,
    -- One proposal per pair. A rejected pair stays as its rejection rather than
    -- being proposed again, so the same near-match is not re-surfaced forever.
    UNIQUE (candidate_reference_id, existing_reference_id)
);

CREATE INDEX idx_media_match_proposals_status
    ON media_match_proposals (status);
CREATE INDEX idx_media_match_proposals_existing
    ON media_match_proposals (existing_reference_id);
