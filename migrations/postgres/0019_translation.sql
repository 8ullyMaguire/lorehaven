-- M17 — Translation pipeline

CREATE TABLE translation_jobs (
    id TEXT PRIMARY KEY,
    source_work TEXT NOT NULL,
    source_lang TEXT NOT NULL,
    target_lang TEXT NOT NULL,
    provider TEXT NOT NULL,
    quote_transaction TEXT,
    state TEXT NOT NULL,
    created_by TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_translation_jobs_source ON translation_jobs(source_work, target_lang);
CREATE INDEX idx_translation_jobs_state ON translation_jobs(state);

CREATE TABLE translation_units (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    chapter_id TEXT NOT NULL,
    paragraph_index INTEGER NOT NULL,
    source_text TEXT NOT NULL,
    target_text TEXT,
    state TEXT NOT NULL,
    memory_hit TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_translation_units_job_para ON translation_units(job_id, chapter_id, paragraph_index);

CREATE TABLE translation_memory (
    id TEXT PRIMARY KEY,
    owner TEXT NOT NULL,
    source_lang TEXT NOT NULL,
    target_lang TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    source_text TEXT NOT NULL,
    target_text TEXT NOT NULL,
    quality_bp INTEGER NOT NULL DEFAULT 5000,
    shared INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_translation_memory_lookup ON translation_memory(owner, source_lang, target_lang, source_hash);

CREATE TABLE translation_glossaries (
    id TEXT PRIMARY KEY,
    owner TEXT NOT NULL,
    work_id TEXT,
    source_lang TEXT NOT NULL,
    target_lang TEXT NOT NULL,
    term TEXT NOT NULL,
    translation TEXT NOT NULL,
    case_sensitive INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_translation_glossaries_owner ON translation_glossaries(owner, source_lang, target_lang);

CREATE TABLE translation_reviews (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    reviewer TEXT NOT NULL,
    gate TEXT NOT NULL,
    state TEXT NOT NULL,
    notes TEXT,
    created_at TEXT NOT NULL,
    decided_at TEXT
);
CREATE INDEX idx_translation_reviews_job ON translation_reviews(job_id, gate);

CREATE TABLE translation_publications (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    work_id TEXT NOT NULL,
    published_at TEXT NOT NULL
);
