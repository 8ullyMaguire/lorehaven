# Handoff — 2026-09-25: metadata exchange specified, docs synchronised, nothing built

Date: 2026-09-25 13:41 CEST. Base commit: `62aaab5`. Branch `master`, **12
commits ahead of `origin/master` before this one.**

## What this session did

Adopted the reviewed parts of an external AI's metadata-exchange proposal into
`docs/spec.md`, then brought every status document into line with that landing.
**No code was written and no test was run.** Every new requirement row is
`planned`; the endpoint, the crate, the entity flow and the trust bars do not
exist yet.

This is a docs-only commit on purpose. The user reviewed the triage and said to
land it; landing a *specification* is not the same as shipping the feature, and
the verification log says exactly that rather than borrowing an unrelated green
suite's numbers.

## The spec sections that landed

| Section | What it specifies |
|---|---|
| §0.3 | A metadata signal is a fact about a work, never a fact about a reader — enforced by the request schema, not by trust in the sender |
| §2.3.1 | `lore-metadata` — the shared serde-only wire crate, order-independent by design |
| §11.17 | The exchange endpoint: version negotiation, signal submission, canonical retrieval |
| §15.17 | Auto-created canonical entities, usable while unverified; `signal_count` as review priority only |
| §16.16.1 | Latent demand — a signal for an unheld work is demand evidence |
| §19.14 | TL1 to submit, TL3 to curate, neither configurable |

## Numbering — read this before citing any section

The triage's correction table was itself one number stale. It recommended §15.16
for the canonical-entity section; **§15.16 was already Content notes.** The
applied landing is **§15.17**. The triage's header now records this, so the
correction is visible rather than a silent divergence between two documents.

Free numbers used: `11.17` (after the adapter backlog), `15.17` (after Content
notes), `19.14` (after community feedback on moderation). `16.16` already
existed as a `###` subsection under §16.13, so latent demand is an amendment to
its contribution-term table rather than a new section.

## Rejected, and why — do not re-propose without new information

- **Credits for signal submission.** Contradicts §19.2 and the §0.3
  credits/trust separation: a reward attached to a contribution is how a
  contribution becomes influence. Also a spam vector against the rate limit.
- **`signal_count` as a demand weight.** The instance cannot count users — one
  account submitting twice is indistinguishable from two submitting once. Kept
  as review priority only.
- **Calling a confidence threshold "auto-quorum."** Quorum in this spec means
  human reviewer quorum (§19.4). The term was not repurposed.

## Verification performed

Section counts, fence balance, and a cross-reference set diff against the
pre-edit spec. Result: **10 unresolved `§x.y` references before, 10 after, 0
newly broken** — the 10 pre-existing ones (`§0.4`, `§10.4.1`, `§14.5`, `§14.9`,
`§16.17`, `§16.18`, `§20.3.1`, `§24.15`, `§34.3`, `§34.5`) are not this
change's damage and are **not fixed**. They are listed in `docs/verification.md`
so a later pass can find them.

Nothing was compiled, because nothing outside `docs/` changed.

## State of the tree

- `docs/requirements.csv` — 253 rows: 192 implemented (172 locally-tested, 20
  fully-tested), 57 planned, 4 unsupported. CRLF terminator preserved; 5-line
  diff, not a whole-file rewrite.
- **172 of 253 rows are `implemented-locally-tested`.** This is the largest soft
  spot in an otherwise green board and it is why M57 is not simply next. A sweep
  that promotes or downgrades each row on evidence is **unowned and
  unestimated** — it is not a task someone has already decided to do.
- `master` was 12 commits ahead of `origin/master` at session start. **E2E has
  not been re-measured on this tree** — the last recorded figure is 73/73 at
  `d04a54a`, and the `921a59d` reset makes older claims untrustworthy.
- Production on thinkcentre is `active` and answering 200, on the **old**
  commit. Deploy means SSH to thinkcentre and building there, never through
  SSHFS.
- `cargo clippy --workspace --all-targets`: 0 warnings, 0 errors. **A previous
  version of this handoff claimed this too, and it was false** — that check ran
  clippy with `2>/dev/null`, and rustc writes warnings to stderr, so the grep
  counted an empty stdout. The 30 warnings were there all along. They are fixed
  in `81bb104`, three of them real defects, and the figure above was measured
  with stderr captured. See the "Correction" section below.

## Next, in order

1. Re-run E2E on this tree, then push, then deploy.
2. **M52-09** — per-user rec engine preference (§16.1b), from a green tree.
3. M53 batch 1 — adapter porting.
4. M54/M55 — bot, then OpenAPI publication. M57 follows M55 deliberately: an
   API surface shaped before any external consumer exists is shaped by
   imagination, and the bot is the first real client.
5. M56 — M45 category governance voting is still unreachable (no proposal list
   route, no UI). ~1 day, and §45 is unusable without it.

## Correction — a false clean claim, and what it hid

This handoff originally stated `cargo clippy --workspace --all-targets: 0
warnings, 0 errors`. **That was wrong.** The command was run as
`cargo clippy ... 2>/dev/null | grep -c`, and rustc emits warnings on **stderr**
— so the pipe read an empty stdout and the count came back zero. Nothing was
suppressed by accident later; the check itself was measuring nothing.

Re-measured with `2>&1`, the tree had **30 warnings and 0 errors**, including
three real defects:

- `media_resilience::find_matching_standing_bounties` took a
  `media_reference_id` it never used, while its doc comment promised a
  per-reference match. The table has no such column.
- `longevity::half_life_map` generated its SQL placeholders with
  `if i == 0 { "" } else { "" }` — both arms empty. It worked by accident.
- `media_resilience::find_by_perceptual_hash` accepted a `max_distance` and
  ignored it, so a caller passing a large threshold got exact matches only
  while the signature implied otherwise.

Plus a test that had never run: `spoilers::test_display` had lost its `#[test]`
attribute, so its assertions were dead code. Clippy was right to flag it and my
first reaction — assuming it already had the attribute — was wrong.

All fixed in `81bb104`; clippy now reports 0/0 with stderr captured, and the
workspace suite is 1775 passed with one known rate-limit flake that passes alone.

**The lesson worth inheriting:** a linter count is only evidence if you know
which stream it came from. If a gate has never failed, suspect the gate before
trusting it.

## Things not to trust

- Any E2E number not measured on the current tree.
- The 172 locally-tested rows read as "working." They are untested against the
  full-stack bar, which is a different claim.
- `auto_quorum` or a signal-count-as-demand idea. Both were rejected on
  conflict with §0.3/§19.2, and the reasoning is in
  `docs/plans/metadata-exchange-triage.md`.
- The patch tool's standalone Rust syntax check on this repo — it reports
  spurious 2015-edition errors. `cargo` is the authority.

## Correction rule

A statement in any of these documents found to be wrong is fixed in place, in the
same commit as whatever proved it wrong. Section numbers in
`metadata-exchange-triage.md` will rot as the spec is edited; re-verify with
`grep -nE '^#+ *[0-9]+\.[0-9]+' docs/spec.md` before citing any of them. An
ambiguous decision becomes an ADR, not a comment.
