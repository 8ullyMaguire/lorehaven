# Review handoff — 2026-09-19 (post-commit 98423fd)

A read-only review of the website after commit `98423fd` ("webhook delivery,
SSRF hardening, and export pipeline fixes"). No source files were modified.
This file lists the issues found, in priority order, with the evidence another
agent needs to fix them.

## Issue 1 (bug, high): webhook deliveries are signed with an empty secret

`crates/app/src/webhook_delivery.rs:65` reads the endpoint's HMAC secret from
the row returned by `lorehaven_db::marketplace::list_all_active_webhooks`, but
that query (`crates/db/src/marketplace.rs:~340`) does not SELECT the `secret`
column — it returns only `id, owner, url, events, active, created_at`
(`list_webhooks` at `marketplace.rs:373` shares the same column list).
`wh["secret"].as_str().unwrap_or("")` therefore always yields `""`, and every
outbound webhook is signed with HMAC-SHA256 under an empty key. A receiver
that stores the real secret will reject every delivery; a receiver that trusts
any signature accepts forgeries.

Fix: add `secret` to the SELECT in `list_all_active_webhooks` (only — leave
`list_webhooks`, the `/me/webhooks` listing, without it so the API keeps not
leaking secrets), and add a test asserting the recorded
`webhook_deliveries.signature` verifies against the stored secret.

## Issue 2 (lint gate, medium): `bulk_export.rs` fails `cargo clippy -D warnings`

Commit 98423fd's `crates/app/src/bulk_export.rs` carries two warnings that fail
the project gate (`just lint` = `cargo clippy --all-targets --all-features --
-D warnings`):

1. `bulk_export.rs:357` — `#[cfg(not(feature = "zip"))]` names a feature that
   does not exist (no `[features]` table in `crates/app/Cargo.toml`), so
   `unexpected_cfgs` fires. There is no alternate implementation behind the
   positive cfg, so the attribute should simply be removed (the hand-rolled
   stored-method ZIP is the only path).
2. `bulk_export.rs:345` — `Artifact.converter_version` is never read (the bulk
   path always writes `None`). Remove the field or prefix it `_`.

Also noted for the same file: `bulk_export.rs:~300` builds the ZIP artifact
without an `extension` field, while `crates/app/src/exports.rs` `Artifact`
(one path later, when the commit message says the media type is propagated)
carries `media_type` + `extension` + `converter`. Verify whichever struct the
bulk download route actually reads uses `ExportFormat::Zip.media_type()`
(`application/zip`) and a `.zip` extension — the HEAD commit message claims
this was fixed, but the two `Artifact` structs drifted.

## Issue 3 (style gate, low): `cargo fmt --check` fails in exports.rs

`cargo fmt --all -- --check` reports diffs at `crates/app/src/exports.rs:460`
and `:483` (the long `ExportError::ZipOnlyForBulk` match arms added by 98423fd
exceed line width). Run `cargo fmt --all`.

## Issue 4 (minor, correctness of record): webhook event envelope fields

The `publish.notify` payload written by `crates/db/src/content.rs:1311` is
`{"work_id", "actor_pseud_id"}` — it carries neither `event_type` nor
`created_at`. `webhook_delivery.rs:31` falls back to `derive_event_type` (fine:
`work.published`), but `webhook_delivery.rs:70` then produces an envelope with
`created_at: ""`, which is part of the signed canonical string. Either bind
`now_rfc3339()` at delivery time or add `created_at`/`event_type` to the
outbox payload at publish time (the latter also keeps signatures stable
across redelivery attempts).

## Issue 5 (test gap): no coverage for the new delivery bridge

`deliver_notification` (`webhook_delivery.rs`) and the
`publish.notify → webhook` wiring (`server.rs:150`) have no integration test:
`crates/app/tests/` references webhooks only in `milestone_16.rs` (creation,
signing primitives) and `route_inventory.rs` (route existence). A test that
enqueues a `publish.notify` outbox event, runs one worker pass against a stub
HTTP endpoint, and asserts one `webhook_deliveries` row with a verifying
signature would have caught Issue 1.

## Verification state when this file was written

- `cargo fmt --all -- --check`: FAILS (Issue 3) — reproduced directly.
- `cargo clippy --workspace --all-targets -- -D warnings`: the two
  `bulk_export.rs` warnings (Issue 2) were observed live; a final clean-build
  gate run was still in progress when this file was written. Earlier noisy
  E0463/E0786 "can't find crate" errors in this session were self-inflicted
  (concurrent gate runs corrupting `target/` rmeta artifacts) — delete the
  zero-byte `target/debug/deps/*.rmeta` files or `cargo clean` before
  re-running; they are not repo defects.
- `cargo test --workspace`: not yet completed clean this session.

## Environment notes for whoever picks this up

- Gates: `just check` runs fmt check, clippy (`--all-features`), tests, and
  frontend checks. Run them sequentially; parallel cargo invocations in the
  same `target/` corrupt artifacts on this machine.
- The frontend (`frontend/src`, Svelte 5 + TS, `scripts/fe.sh`) has no
  webhook or bulk-export UI surface yet; `/exports` exists as a route. Any
  webhook management UI would need the `/me/webhooks` API, which deliberately
  does not return `secret` after creation (the one-time `whsec_…` response in
  `routes/marketplace.rs:286` is the only disclosure — keep it that way).
