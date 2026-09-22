-- M44 taste vectors — multi-dimensional user taste alignment (spec §16.17)
-- Replaces scalar resonance with a configurable vector of dimensions.

ALTER TABLE accounts ADD COLUMN taste_vector TEXT DEFAULT '[]';  -- JSON array of f64
ALTER TABLE accounts ADD COLUMN taste_vector_computed_at TEXT;
ALTER TABLE accounts ADD COLUMN taste_centroid_distance REAL DEFAULT 0.0;
