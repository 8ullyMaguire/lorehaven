-- Seed default forum categories (spec §17.4).
-- A "general" category is required for the forum to function at all.

INSERT INTO forum_categories (id, name, position, min_trust) VALUES ('general', 'General', 1, 0)
ON CONFLICT (id) DO NOTHING;
