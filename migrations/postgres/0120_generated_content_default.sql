-- 0120: the generated-content posture's default is `allow` (M4-07).
--
-- Spec §6.9: "**generated content** | `forbid` · `disclose` · `allow` | `allow`
-- (D-4/N-4 — the archive indexes and recommends AI-generated works, labelled)".
--
-- THE DEFAULT MOVES, AND THE ROW DOES NOT.
--
-- The policy table is a singleton, and an instance that has *decided* its
-- posture keeps it: this migration does not touch the row that is there. What
-- it changes is the **default** — the answer an instance gives when nobody has
-- set a policy — and the default was `forbid`.
--
-- The reason is in the specification and it is about this archive and not about
-- safety in the abstract: the owner's corpus has 8 of 54 works carrying
-- `ai-generated text`, and **he writes them** (D-4). An archive whose default
-- refuses the work its own author writes is an archive that refuses its
-- author, and the fix the default invites is the operator who discovers it
-- had to go and change a setting to publish work that was always allowed.
--
-- The posture is still the operator's, and nothing raises it: a work, an
-- uploader, an importer, an extension or a federated peer cannot move it
-- (spec §6.9). An operator who wants nothing generated sets `forbid`, and the
-- refusal that follows names the policy — "this instance forbids generated
-- content" — so a reader is told the rule and not given a bare error.
--
-- WHY THE TABLE IS REBUILT
--
-- SQLite cannot ALTER a column's default, and the documented rebuild procedure
-- has to switch foreign keys off — which SQLite refuses inside a transaction,
-- and every migration runs in one. **This table is safe to rebuild** where
-- `works` was not: it is a singleton with no foreign keys pointing at it and no
-- index of its own, so the rebuild is a copy, a drop and a rename.
--
-- The `works.generated_content_posture` column keeps its own default of
-- 'forbid'. That column is a **record of the posture in force when the work was
-- written**, not a default to be honoured: `create_work` names it from the
-- policy, and a row inserted without it would be a work whose recorded posture
-- nobody chose — which is the defect migration 0109's note warns about. It
-- cannot be given a new default without rebuilding `works`, and that rebuild
-- is not worth a default nothing reaches.

-- The new table's columns are the OLD table's columns, exactly, with one
-- change: the default. A rebuild that changes a column's type alongside its
-- default is a rebuild that has to convert the data, and this one copies the
-- rows verbatim.
--
-- (`id` is TEXT here and `updated_by` is a UUID, because that is what
-- migration 0109 declared: the policy's key is the constant 'default' and the
-- actor is an account.)
CREATE TABLE generated_content_policy_new (
    id          TEXT    PRIMARY KEY,
    posture     TEXT    NOT NULL DEFAULT 'allow'
        CHECK (posture IN ('forbid', 'disclose', 'allow')),
    updated_by  UUID    REFERENCES accounts (id) ON DELETE SET NULL,
    created_at  TEXT    NOT NULL,
    updated_at  TEXT    NOT NULL,
    version     INTEGER NOT NULL DEFAULT 1
);

INSERT INTO generated_content_policy_new
    (id, posture, updated_by, created_at, updated_at, version)
SELECT id, posture, updated_by, created_at, updated_at, version
  FROM generated_content_policy;

DROP TABLE generated_content_policy;

ALTER TABLE generated_content_policy_new RENAME TO generated_content_policy;
