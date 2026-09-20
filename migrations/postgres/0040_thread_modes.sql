-- M33: thread modes for creative work (spec §35.3).
--
-- Twin of the SQLite file, with the same defaults. The mode column is TEXT
-- for portability; Postgres could use an enum, but SQLite can't, and a TEXT
-- default + CHECK constraint gives us both.

ALTER TABLE forum_topics ADD COLUMN mode TEXT NOT NULL DEFAULT 'plain';

CREATE TABLE topic_schedules (
    topic_id     TEXT NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    title        TEXT NOT NULL,
    chapter_start INTEGER NOT NULL,
    chapter_end   INTEGER NOT NULL,
    unlocks_at   TEXT NOT NULL,
    PRIMARY KEY (topic_id, position)
);
CREATE INDEX topic_schedules_unlock ON topic_schedules (topic_id, unlocks_at);

CREATE TABLE topic_wiki_pins (
    topic_id    TEXT NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    post_id     TEXT NOT NULL REFERENCES forum_posts(id) ON DELETE CASCADE,
    body        TEXT NOT NULL,
    revision    INTEGER NOT NULL DEFAULT 0,
    edited_by   TEXT NOT NULL,
    edited_at   TEXT NOT NULL,
    approved_by TEXT,
    approved_at TEXT,
    PRIMARY KEY (topic_id, post_id)
);
CREATE INDEX topic_wiki_pins_topic ON topic_wiki_pins (topic_id);

CREATE TABLE critique_queue (
    topic_id   TEXT NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    pseud      TEXT NOT NULL,
    position   INTEGER NOT NULL,
    posted_at  TEXT,
    excerpt    TEXT,
    PRIMARY KEY (topic_id, pseud)
);
CREATE INDEX critique_queue_topic ON critique_queue (topic_id, position);

CREATE TABLE prompt_posts (
    topic_id     TEXT NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    prompt_date  TEXT NOT NULL,
    winner_pseud TEXT,
    awarded_at   TEXT,
    PRIMARY KEY (topic_id, prompt_date)
);

CREATE TABLE critique_participants (
    topic_id   TEXT NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    pseud      TEXT NOT NULL,
    joined_at  TEXT NOT NULL,
    PRIMARY KEY (topic_id, pseud)
);
