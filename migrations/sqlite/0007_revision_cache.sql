-- Migration 0007 — conditional requests for the source revision cache
-- (spec §10.4, §11.7).
--
-- Dialect: SQLite.
-- Timestamps are RFC 3339 UTC text; identifiers are canonical UUID text.
--
-- `source_revision_cache_entries` was created by migration 0005 and had never
-- been written to. Its key — source, revision, adapter version, security scope —
-- says which page of which source it caches, and its `checksum` points at the
-- stored bytes. What it could not hold is the pair of validators a conditional
-- request is actually made with, so a fetch could say *what* it had and not
-- *which revision* it was: `If-None-Match` needs the ETag and
-- `If-Modified-Since` needs the Last-Modified, and neither had a column.
--
-- Both are nullable, and that is the honest shape rather than a shortcut: a
-- server is free to send neither, one, or both, and a cache row that recorded a
-- guessed validator would send a conditional request the source cannot answer
-- `304` to — which is a wasted request dressed up as an optimisation.
--
-- The cache stays a *temporary* fetch cache and not a snapshot of anybody's
-- library. It carries an adapter version so a parser change invalidates its own
-- entries, and a security scope so a page fetched with one reader's credential
-- is never served to another's request. Nothing reads it except the fetcher,
-- and every row expires on its own clock.

ALTER TABLE source_revision_cache_entries ADD COLUMN etag TEXT;
ALTER TABLE source_revision_cache_entries ADD COLUMN last_modified TEXT;
