CREATE TABLE comment_classifications (
    comment_id TEXT PRIMARY KEY,
    class TEXT NOT NULL,
    confidence_bp INTEGER NOT NULL,
    signals TEXT NOT NULL,
    outcome TEXT NOT NULL,
    classified_at TEXT NOT NULL
);
CREATE INDEX comment_classifications_outcome ON comment_classifications (outcome);
