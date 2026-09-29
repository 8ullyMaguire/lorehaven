-- M61: the decision audit trail (spec §11.14, amendment
-- docs/spec-amendments/calibrated-decision-models.md §3.5).
--
-- Dialect: SQLite.
--
-- The counterpart of migrations/postgres/0088_decision_audit.sql. The declared
-- tables, columns and indexes must match that file exactly, because
-- `the_two_dialects_declare_the_same_columns_and_indexes` in
-- crates/db/src/migrate.rs compares them.
--
-- WHAT THIS IS. `GET /api/v1/decisions/audit` has always existed and has always
-- returned `{ "items": [] }`. It claims to be an operator-only audit trail and
-- carries nothing, so the endpoint is a promise with no content behind it. This
-- table is that content: per decision, the deterministic answer, the model's
-- posterior, the threshold in force, which one won, and which provider answered.
--
-- WHY THE SUBJECT IS AN ID AND NEVER THE TEXT. This is the load-bearing design
-- decision in the whole amendment, and it is a privacy decision rather than a
-- schema one. §12.1's commitment is that a reader's words stay with their
-- author and that the filter is a judgement, not a taking. An audit table that
-- stored the text it graded would be a SECOND COPY of that text with none of
-- the first copy's audience rules: no deletion path, no export, no visibility
-- level, no author who can see it. The audit needs to answer "what did the
-- system decide, and on what evidence" — and the answer to the second half is a
-- poster and a number, not the words. An operator debugging a misbehaving
-- threshold needs the SUBJECT's current classification and the number the model
-- gave, not the prose.
--
-- WHY THE POSTERIOR IS NULLABLE. A NULL is not "zero confidence" and must never
-- be read as it: NULL means no model was consulted, which is the deployed state
-- of every instance that has not opted in, the degradation path when a model is
-- down, and the normal case for an instance that fetches no body. Collapsing
-- those into 0.0 would report every deterministic decision as a model
-- confidently saying "no", and an operator tuning a threshold from that would
-- tune it against a fiction.
--
-- WHY `deterministic` AND `outcome` ARE BOTH STORED. They are usually equal --
-- the model can only narrow an acceptance to a hold -- and the row that proves
-- it is the row where they DIFFER. A table that stored only the outcome could
-- not show that the model's influence stayed one-directional over time, which is
-- the property the whole design rests on. Storing both is what lets an operator
-- verify it rather than trust it.

CREATE TABLE IF NOT EXISTS decision_audit (
    id             TEXT PRIMARY KEY,
    -- Which decision surface made the call: 'import_quality', 'positivity',
    -- and whatever is added later. Not an enum, because an enum here would make
    -- adding a decision surface a schema migration, and the amendment's whole
    -- point is that a new surface needs only new code.
    task           TEXT NOT NULL,
    -- What was classified: a library item id, a comment id, an account id.
    -- NEVER the classified text -- see the header.
    subject        TEXT NOT NULL,
    -- What the deterministic classifier said: 'accepted' | 'rejected' | 'held'.
    deterministic  TEXT NOT NULL,
    -- The model's probability that this is a work, in 0.0..=1.0. NULL when no
    -- model was consulted -- see the header. There is deliberately NO CHECK
    -- constraint on the range: a constraint cannot express "NULL or in range"
    -- in either dialect without dialect-specific null handling, and the client
    -- already refuses a NaN and an out-of-range posterior rather than clamping
    -- it (crates/decisions/src/lib.rs::check_probabilities), so the value that
    -- reaches this column has already been refused if it was nonsense.
    posterior      REAL,
    -- The threshold in force when the call was made, so a row remains readable
    -- after the operator lowers it. Storing the threshold a posteriori would
    -- mean re-deciding history with today's settings.
    threshold      REAL,
    -- What was actually applied: the reconciled answer.
    outcome        TEXT NOT NULL,
    -- 'deterministic' | 'calibrated'. Which provider answered, which is not the
    -- same question as which one won.
    provider       TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    version        INTEGER NOT NULL DEFAULT 1
);

-- Newest first, for the endpoint's default order. DESC because every reader of
-- this table is reading it to answer "what just happened", and a table that has
-- to be sorted to answer that question is one that will be read in the wrong
-- order under pressure.
CREATE INDEX IF NOT EXISTS idx_decision_audit_created
    ON decision_audit (created_at DESC, id DESC);

-- Per-task. The operator reads this to answer "has the calibrated path been
-- doing anything for import_quality, or is every row deterministic?".
CREATE INDEX IF NOT EXISTS idx_decision_audit_task
    ON decision_audit (task, created_at DESC);

-- Per-subject. This is the index that makes the audit worth having: "why was
-- this comment held" is the question an operator or an author will actually ask,
-- and without it that is a full scan.
CREATE INDEX IF NOT EXISTS idx_decision_audit_subject
    ON decision_audit (subject, created_at DESC);
