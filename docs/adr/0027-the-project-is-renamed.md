# ADR 0027: The project is renamed — Darkhold, and the history is kept

- Status: accepted
- Date: 2026-10-19

## Context

The project was named **lorehaven** and is now **Darkhold** — *the dark side of fanfiction*.
The name is not a cosmetic change: the instance's whole identity is its editorial position
(D-8 in the Darkhold specification), and "lorehaven" described a *place you shelter in*, which
is the opposite of what the instance is. The register the owner actually reads is power,
dominance, corruption and progression, and the name is the first thing a stranger meets.

The specification was written against a conversion, not a rewrite. **lorehaven is the base** and
is converted in place: the git history, the 703-row requirement ledger, 112 migrations per
dialect, 75 route modules and 4,181 tests are all kept.

## Decision

1. **The rename is in place, and the history is kept.** Every commit is kept — the same
   commits, in the same order, with the same messages, the same authors and the same dates.
   **The commit SHAs necessarily change**, because a commit's hash covers its tree and the tree
   changed; "the history is kept" is about the commits, not the hashes. The verification is the
   commit metadata, compared line for line before and after, and it is recorded in
   `docs/verification.md`.
2. **The rewrite is `git filter-repo`, and nothing else.** One command reproduces the committed
   history exactly — the same commit SHAs, all 1,048 of them:
   ```bash
   git filter-repo --force --replace-text scripts/rename-replacements.txt \
     --filename-callback 'return filename.replace(b"lore-metadata", b"metadata").replace(b"lorehaven", b"darkhold")' \
     --refs master
   ```
   The six rules are in `scripts/rename-replacements.txt` and they are explained in
   `scripts/rename-replacements.README.md`, which is where the explanation has to live:
   **`--replace-text` has no comment syntax**, and a line in that file that is exactly `#` is a
   rule that removes every `#` from every blob in the repository. It is not a hypothetical — a
   rules file with a comment block was written during the verification of this milestone, and
   the re-run against the pre-rename bundle produced a tree in which every comment in the
   repository had become `***REMOVED***`. **The tree was caught before it was committed**,
   because the verification compares the re-run tree against the committed tree rather than
   against an expectation.
   - The first four rules replace the project's name, and the fourth is only correct because
     of where it sits: `lorehaven-lore-metadata` becomes `darkhold-lore-metadata` and *then*
     `darkhold-metadata`.
   - The last two fix the crate name, and they **only exist after the first four have run**,
     because their source text does not exist before it. That is why the rewrite was performed
     in two passes and why the rules file lists the second pass's rules after the first's.
   - The filename callback's argument is named `filename` and it is **bytes**. A `str.replace`
     on it does not compile.
   - **The rewrite was re-run against the pre-rename bundle and produced an identical tree:
     1,265 files, identical paths, identical blob hashes, identical tip SHA.** A rewrite that
     cannot be re-run is a rewrite that cannot be checked.
3. **Only `master` is rewritten.** The 45 tags and the remote-tracking branches keep the old
   history. Two reasons: **release tags are the owner's decision alone**, and the remotes still
   hold the old history until the owner chooses to force-push. The local `master` is one commit
   ahead of `github/master` and `origin/master`, and 74 ahead / 4 behind `thinkcentre/master`;
   the divergence predates this change and is not reconciled here.
4. **The old name survives only as a record of the rename, and never as a live name.** It is
   in five places in the tracked tree, and all five are the rename described in the past
   tense: this ADR, `docs/adr/0028-the-ledger-is-re-triaged-not-restarted.md`, the
   `## 2026-10-19 — M0` entry in `docs/verification.md`, and the three M0 ledger rows
   (`DM0-02`, `DM0-04`, `DM0-05`) that name the *old* bundle, the *old* crate path and the
   *old* environment prefix, because the thing being recorded **is** the change from those
   names.

   **The rule is not "the old name appears nowhere."** It is **"the old name appears nowhere
   as a live name"** — and a name in a past-tense sentence about a rename is the opposite of
   live. A gate that grepped for the string without that distinction would fail the record of
   its own rename.
5. **The token namespace is not renamed.** The CSS tokens are `--color-*`, `--reader-*`,
   `--font-*`, `--radius-*`, `--leading-*` and `--duration-*` — 114 declarations, and **not one
   of them carries the product name.** The specification names `--reader-*` itself, so a
   `--dh-*` namespace would contradict the spec and churn every `var(--…)` use for no gain.
6. **The requirement ledger is re-triaged in place, not restarted.** The 703 rows are mapped
   into the Darkhold vocabulary; the rows that are decisions rather than requirements are
   carried across as `unsupported`, which is how the non-goals are *enforced* rather than
   merely stated — a row cannot be silently reclassified as `planned` without the diff being
   visible.

## Consequences

- The binary is `darkhold`, the configuration file is `darkhold.toml`, the environment prefix
  is `DARKHOLD_*`, the local database is `data/darkhold.sqlite` and the scratch directories are
  `darkhold-*`.
- **The ledger is still CRLF**, and `scripts/ledger.py` is now the only writer of it. The
  rewrite preserved the line endings, and the writer keeps them.
- **Force-pushing the rewritten `master` is the owner's decision**, not this change's. Until
  it happens, the remotes and the local `master` disagree, and the disagreement is a *rename*
  disagreement rather than a content one.
- The repository directory is renamed to match. The `thinkcentre` remote still points at the
  old path on that host, because renaming a bare repository on another machine is a separate,
  deliberate operation.

## The trap this change exposed

`crates/app/tests/m53_source_credentials.rs` — 36 KB of tests — **had been untracked since
2026-10-04**, because the *global* gitignore at `~/.config/git/gitignore` contains
`*credentials*` and the repository's own `.gitignore` negated it only for
`migrations/**/*credentials*.sql`. The file describes the table it tests and holds no secret.
The consequence was that **every checkout except this machine was missing those tests**, and
`git add -A` reported success the whole time; the rename then failed to compile because
nothing had rewritten the crate references in a file git had never heard of.

This is the second instance of a trap the repository's own `.gitignore` documents in detail,
and it is now closed the way that file says it should be closed: **an explicit negation per
file** (`!crates/**/*credentials*.rs`), never a loosened global rule — the global rule exists
to do its job for actual secrets.

**The generalisable lesson: a content rewrite only reaches committed blobs.** Any working-tree
file that is untracked is invisible to `git filter-repo`, and it will fail to compile with the
old names still in it. The check that finds them is `git ls-files --others --ignored
--exclude-standard`, and the check that must be run after a rename is **the build**.
