-- 0131 — the catalogue's quality signal, promoted out of provenance_json.
--
-- Why these columns exist: `library_items` is a metadata catalogue of the whole
-- fanfiction web, and the operator's question about it is "what is good", not
-- "what is recent". The best real endorsement signal differs per source
-- (threadmark likes on the XenForo boards, kudos on AO3, followers on Royal
-- Road), and until now it lived only inside `provenance_json.extra`, where it
-- could not be filtered or sorted without a JSON scan per row.
--
-- `likes` is NOT NULL DEFAULT 0 for the same reason `word_count` is nullable and
-- this is not: every catalogue row has an endorsement count, and zero ("nobody
-- liked it") is a fact. `views` is genuinely nullable — several sources do not
-- publish one, and a zero there would claim they do.
--
-- The index is what makes `?source=sb&sort=quality` a range scan rather than a
-- sort of the catalogue.
ALTER TABLE library_items ADD COLUMN IF NOT EXISTS likes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE library_items ADD COLUMN IF NOT EXISTS views INTEGER;

CREATE INDEX IF NOT EXISTS idx_library_items_source_likes
    ON library_items (source_key, likes DESC);

-- The cross-source aggregate matches on normalised title + author. Without this
-- index every returned row would scan the whole catalogue; with it the aggregate
-- costs one index lookup per row on the page.
CREATE INDEX IF NOT EXISTS idx_library_items_title_author_norm
    ON library_items (lower(btrim(title)), lower(btrim(author_text)));
