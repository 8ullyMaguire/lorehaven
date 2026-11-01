# Decision needed: deploying this lineage

**Status: waiting on the operator. No deploy was performed for M63-02 because of this.**

## What was found

Measured this session, from `cachyos-B450` and from `thinkcentre`:

| fact | value |
|---|---|
| local `~/code-local/rust/darkhold` | branch `master`, `ahead 1155, behind 1046` of `origin/master` |
| deploy tree `/personal/documents/code/rust/darkhold` | HEAD `c177bc1b`, with uncommitted edits (`README.md`, `auth.rs`, `governance.rs`, `inventory.rs`, …) |
| does the deploy tree contain this line's recent commits? | `git cat-file -e 8b5f851f` → **missing**; `af162ea3` → **missing** |
| running binary stamp | `commit=ac0d6946`, `built_at=2026-10-28T13:54:04+01:00` |
| live DB (`darkhold`) | migrations at **0131**, 128 ledger rows, 141,317 `library_items`, 58,042 `works` |
| live `/api/v1/catalogue` | 200 with real rows (`items[0].source_key = "sb"`) |
| live `/api/v1/catalogue/sources` | `sb 35,927`, `archive 28,488`, … |

So the live instance **serves the catalogue feature and sits at migration 0131** while its
checkout is at `c177bc1b`, which does **not** contain this line's catalogue commits. Two histories
have converged on one database — the state `reconciling-a-foreign-lineage-deployment` and
`darkhold-thinkcentre-deploy` both warn about, and the reason this line's `git push` is not a
deploy.

`darkhold-thinkcentre-deploy` says the two lineages share 0001-0113 apart from comments and that
the deploy host's tree is an `git archive` export. That was true on 2026-10-27; it is not true now
(ahead/behind in the thousands, commits absent).

## Why this blocked the item

M63-02's code is in `crates/domain`, `crates/db` and `crates/app` on **this** line. Deploying it
means either rsyncing this tree over a checkout holding another session's uncommitted work, or
`git archive HEAD | tar -x` into the deploy root — both of which silently replace the other line's
files. `darkhold-thinkcentre-deploy` records exactly that failure ("whichever session builds and
restarts last wins, silently") and prescribes a bundle-merge instead of a copy. That is a
deployment-lineage decision, not an implementation detail, so it is written down rather than
guessed.

## Options

**A. Bundle-merge this line into the deploy tree (skill's documented path).**
`git bundle create` a range, `git merge --no-edit` on the host, resolve, rebuild, restart.
*For:* keeps both lines' history; the host stays the single deploy tree; no force-push.
*Against:* a merge of ~1000 commits each way in a tree with uncommitted edits; whoever owns the
host's working changes must be available. Expect conflicts in the files the deploy tree has
modified (`auth.rs`, `governance.rs`, `inventory.rs`, `README.md`).

**B. Deploy from a dedicated branch of this line.**
Fast-forward `deploy/master` from this line, `git archive` it into a *second* checkout
(`~/deploy/darkhold-master`), build there, and point the unit's `ExecStart` at that binary —
leaving the other checkout and its uncommitted work untouched.
*For:* no merge, no conflict, reversible by flipping `ExecStart` back; the skill's
`install-binary.sh` already keeps the replaced binary.
*Against:* two checkouts on the host (the skill warns the deploy path is shared, so `ExecStart`
must be unambiguous); migrations still run against the one live database, so a migration from
this line reaches data the other line reads.

**C. Do not deploy this line; port items instead.**
Treat the deploy tree as the trunk and port each finished item into it as a patch.
*For:* the live instance is never at risk from a bulk merge.
*Against:* every item costs a port, and the two lines drift further; the catalogue features that
already exist live prove drift is happening one item at a time.

## Recommendation

**B**, with the migration caveat handled first. It is the only option that ships without touching
another session's uncommitted work, and it is reversible in one `systemctl` command because the
replaced binary is kept. Before the first deploy: confirm migrations 0130–0133 on this line are
no-ops against the live 0131 database (they should be — 0131 is already applied — and a
`darkhold migrate --dry-run`-style diff on a copy is the check).

## What is needed to proceed

A yes on the option, and a moment when no other session is building on `thinkcentre`.
