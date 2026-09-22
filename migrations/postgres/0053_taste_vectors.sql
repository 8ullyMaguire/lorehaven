-- M44 taste vectors — multi-dimensional user taste alignment (spec §16.17)
-- Replaces scalar resonance with a configurable vector of dimensions.

ALTER TABLE accounts ADD COLUMN taste_vector JSONB DEFAULT '[]';
ALTER TABLE accounts ADD COLUMN taste_vector_computed_at TIMESTAMPTZ;
ALTER TABLE accounts ADD COLUMN taste_centroid_distance DOUBLE PRECISION DEFAULT 0.0;
