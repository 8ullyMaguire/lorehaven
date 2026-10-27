-- 0129 — the works ladder (docs/spec-amendments/arena-work-ladder.md, §12.4a).
--
-- The arena's ballot rates WORKS with Glicko-2. The reader's dimension ladder in
-- `arena_weights` stays where it is: it is left unwritten rather than deleted,
-- because an instance whose works carry dimension tags can still use it.
--
-- Values are on the Glicko scale (mu 1500, phi 350, sigma 0.06), which is what
-- `darkhold_taste::arena::glicko::Rating` documents and what the kindred parity
-- fixture records. Storing reduced values here would make the two ladders'
-- numbers disagree for no reason.
--
-- `rated_at` is TEXT to match `bookmarks.created_at` on both engines: the
-- migration catalogue compares column types across dialects, and a TIMESTAMPTZ
-- here against TEXT there is exactly the drift it exists to catch.
CREATE TABLE work_ratings (
    work_id    UUID PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    mu         DOUBLE PRECISION NOT NULL,
    phi        DOUBLE PRECISION NOT NULL,
    sigma      DOUBLE PRECISION NOT NULL,
    -- Comparisons, not ballots: one ballot contributes three comparisons to the
    -- best card, two to each middle and three to the worst.
    matches    BIGINT NOT NULL DEFAULT 0,
    rated_at   TEXT NOT NULL
);

-- The leaderboard reads highest-rated first.
CREATE INDEX work_ratings_mu ON work_ratings (mu DESC);
