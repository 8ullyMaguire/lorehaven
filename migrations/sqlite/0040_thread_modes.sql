-- M33: thread modes for creative work (spec §35.3).
--
-- Each topic carries a mode that restructures one surface. The mode is data,
-- not code: a plain topic is unaffected, and a reading-group topic gets a
-- schedule of sections. The schema is additive — existing topics keep working
-- with the default 'plain' mode.

-- The mode itself. 'plain' is the default and needs no configuration; every
-- other mode has a supporting table below.
ALTER TABLE forum_topics ADD COLUMN mode TEXT NOT NULL DEFAULT 'plain';

-- Reading group schedule: one row per section, unlocked on a date.
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

-- Wiki pin: a collaboratively edited post pinned above the OP. Edits pass
-- through an approval queue before they become visible.
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

-- Critique circle turn queue. Members post excerpts on a turn; the server
-- enforces order and limits pile-on. Position is the turn order (0-based).
CREATE TABLE critique_queue (
    topic_id   TEXT NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    pseud      TEXT NOT NULL,
    position   INTEGER NOT NULL,
    posted_at  TEXT,
    excerpt    TEXT,
    PRIMARY KEY (topic_id, pseud)
);
CREATE INDEX critique_queue_topic ON critique_queue (topic_id, position);

-- Prompt posts: the engine writes a topic per prompt date; replies are flash
-- fiction; the community votes on winners.
CREATE TABLE prompt_posts (
    topic_id     TEXT NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    prompt_date  TEXT NOT NULL,
    winner_pseud TEXT,
    awarded_at   TEXT,
    PRIMARY KEY (topic_id, prompt_date)
);

-- Critique circle participants (private group thread mode). Joining opts the
-- member into the positivity gate's constructive-criticism requirement.
CREATE TABLE critique_participants (
    topic_id   TEXT NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    pseud      TEXT NOT NULL,
    joined_at  TEXT NOT NULL,
    PRIMARY KEY (topic_id, pseud)
);
