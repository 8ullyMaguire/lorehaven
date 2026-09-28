# Plan — M60: Periodic De-identified Dataset Sharing

**Spec:** `docs/spec-amendments/periodic-deidentified-dataset.md` (§11.16, §11.17)
**Status:** Ready to implement, in phases. **Nothing here is a single PR.**

This plan is written so an LLM can implement each phase from this file alone.
Every step names the exact command and the exact expected output.

---

## What is already known, and was verified rather than assumed

These were checked before writing the plan, because the plan's shape depends on
them. **Re-verify before implementing** — the last one is version-dependent.

1. **The instance holds no client IP addresses.** No migration declares an
   `inet`, `ip_address`, `client_ip` or `remote_addr` column.
   `crates/app/src/limiter.rs` builds a rate-limit key in memory and discards it
   with the request. **The IP exposure in this feature is entirely a property of
   the transfer**, which is why §11.17 is a separate section and no amount of
   dump work addresses it.

2. **`pgcrypto` is available; `postgresql_anonymizer` is not.**
   ```bash
   sudo docker exec lh-m32e-pg psql -U lorehaven -d postgres -tAc \
     "SELECT name FROM pg_available_extensions WHERE name IN ('postgresql_anonymizer','pgcrypto')"
   # observed: pgcrypto
   ```

3. **This is a problem for the owner's recommended tool.**
   `postgresql_anonymizer` is the right *idea* and is the right answer for a
   deployment that can install extensions — but as of 3.2 it carries **three
   critical CVEs** (CVE-2026-19633, CVE-2026-19634, CVE-2026-83534,
   privilege escalation / SQL injection in the rule-import path), it now
   **refuses to run masking as a superuser** unless `anon.nosuperuser` is set
   back to `FALSE`, and its own documentation says **`anon.random_id()` cannot
   be used in backup masking** because `pg_dump` connects read-only and the
   function needs to advance a sequence. That last point is fatal for this
   feature as specified: the whole dataset depends on consistent re-keying, and
   the extension's primary-key advice is to use a *secret* shift, which §11.16.3
   requires to be **published** so a third party can verify the construction.

   **So the plan does not use `postgresql_anonymizer`.** It uses `pgcrypto`,
   which is in the stock image, has no CVE history, and produces a derivation a
   third party can recompute. The extension remains the better answer if the
   operator later moves to a managed instance that ships it — and the rules
   below port to `anon` masking almost unchanged. That is a note in the handoff,
   not a dependency.

4. **The deterministic re-key is verified to work** on this engine:
   ```bash
   sudo docker exec lh-m32e-pg psql -U lorehaven -d postgres -tA -c "
     SELECT v,
       (substr(encode(digest(v || 'lorehaven-snapshot-v1','sha256'),'hex'),1,8) || '-' ||
        substr(encode(digest(v || 'lorehaven-snapshot-v1','sha256'),'hex'),9,4) || '-4' ||
        substr(encode(digest(v || 'lorehaven-snapshot-v1','sha256'),'hex'),14,3) || '-a' ||
        substr(encode(digest(v || 'lorehaven-snapshot-v1','sha256'),'hex'),18,3) || '-' ||
        substr(encode(digest(v || 'lorehaven-snapshot-v1','sha256'),'hex'),21,12))::uuid
     FROM (VALUES ('11111111-1111-1111-1111-111111111111'::text),
                  ('22222222-2222-2222-2222-222222222222'::text),
                  ('11111111-1111-1111-1111-111111111111'::text)) s(v);"
   ```
   Expected: three rows, **the first and third identical**, the second
   different, all shaped as UUIDs. Same input → same output is the entire
   dataset; the `'4'` and `'a'` nibbles are forced so the result is a
   **version-4 UUID** and not merely a string that parses as one.

---

## Phase D1 — The masking rules as code, with a test that proves them

**This phase is the whole safety property. Do not go further until it is green.**

### D1.1 The inventory gate — `scripts/check-snapshot-pii.py`

A table in the spec (§11.16.2) is only a policy if something reads the schema
and fails on a column the policy has never been told about. This is the same
argument as `check-uncast-pg-placeholders.py` and the same failure it already
had: **a list that is not enforced is a list that is a comment.**

```python
#!/usr/bin/env python3
"""Fail when a column in a PII-bearing table is absent from the snapshot policy.

The policy is a table, not a list: a column that appears in a PII-bearing table
after the policy was written is, by construction, unclassified. Failing on it is
the only thing that keeps §11.16.2's "a new column requires deciding its row"
true rather than aspirational.

    --self-test   run the built-in cases (must pass)
    <path>        check a migrations directory (default: migrations/postgres)
"""
```

Rules it must implement:

* Parse every `CREATE TABLE` in the migrations directory, recording
  `(table, column) -> type`.
* `PII_TABLES` is the policy: a dict of table -> {column: treatment}, where
  treatment is one of `replace`, `drop_column`, `drop_table`, `keep`.
* **Any column of a `PII_TABLES` entry that is not itself in the policy is a
  failure.** This is the rule that does the work.
* Any table whose name matches a `DROP_TABLE` glob is exempt.
* A column matching a `KEEP` glob (e.g. `created_at`, `updated_at`) is exempt
  and is listed in the output as *checked and kept*, so a reader can see it was
  a decision rather than an oversight.

**Verification:**

```bash
python3 scripts/check-snapshot-pii.py --self-test
# expected: self-test: N cases passed

python3 scripts/check-snapshot-pii.py
# expected: OK: every column in a PII-bearing table is classified (N tables, M columns)
```

**Required self-test cases** — one per rule, plus these two that are the point:

* adding a `real_name TEXT` column to `accounts` is **rejected** (this is the
  case the script exists for);
* a migration that adds a column to a `DROP_TABLE`ped table is **allowed**
  (proving the exemption is not a blanket "any new column fails").

**Mutation to run before committing** (the repo's standing rule — a gate that
has never been seen red is a hypothesis):

```bash
# add an unclassified column to accounts, confirm the gate goes red, revert
```

### D1.2 The masking SQL, generated not written by hand

`scripts/build-snapshot-sql.py` reads the policy and emits one `.sql` file.
**Generated, because 60 migrations' worth of `pseud_id` columns will drift from
a hand-maintained list, and a drift here is a silent leak.**

For each `replace` column: a `CASE`/view projection. For each `pseud_id`-
shaped column: the §11.16.3 derivation. For each `drop_table`: the table is
excluded from the dump with `--exclude-table`.

**The re-key expression, as implemented:**

```sql
-- v1 namespace. PUBLISHED, not secret: §11.16.3 requires a third party to be
-- able to recompute the mapping and check it, and a secret here would be
-- reversible by anyone holding the real pseud_id list.
-- See docs/spec-amendments/periodic-deidentified-dataset.md §11.16.3
CREATE OR REPLACE FUNCTION snapshot_pseud(raw uuid) RETURNS uuid
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
  SELECT (
    substr(hex, 1, 8) || '-' || substr(hex, 9, 4) || '-4' ||
    substr(hex, 14, 3) || '-a' || substr(hex, 18, 3) || '-' || substr(hex, 21, 12)
  )::uuid
  FROM (SELECT encode(digest(raw::text || 'lorehaven-snapshot-v1', 'sha256'), 'hex') AS hex) s;
$$;
```

`IMMUTABLE` is required (a `pg_dump`-time function that claims to be stable
would defeat the planner's ability to fold it), and `PARALLEL SAFE` because the
dump may use parallel workers.

**Verification:**

```bash
python3 scripts/build-snapshot-sql.py --out /tmp/snap.sql
psql "$LOREHAVEN_TEST_PG_URL" -f /tmp/snap.sql   # must not error
```

```bash
# the property, asserted directly
psql "$LOREHAVEN_TEST_PG_URL" -tAc "
  SELECT snapshot_pseud('11111111-1111-1111-1111-111111111111'::uuid)
       = snapshot_pseud('11111111-1111-1111-1111-111111111111'::uuid)
     AS same_input_same_output,
         snapshot_pseud('11111111-1111-1111-1111-111111111111'::uuid)
       <> snapshot_pseud('22222222-2222-2222-2222-222222222222'::uuid)
     AS different_input_different_output;"
# expected: t | t
```

**The `4` and `a` nibbles must be asserted, not just the equality.** A derivation
that produces a valid-looking UUID of the wrong *version* is still a leak
vector if any tool keys on version. Add a third assertion:

```bash
psql "$LOREHAVEN_TEST_PG_URL" -tAc "
  SELECT substring(snapshot_pseud('11111111-1111-1111-1111-111111111111'::uuid)::text, 15, 1) = '4' AS version_4;"
# expected: t
```

### D1.3 The end-to-end leak test

`crates/app/tests/snapshot_anonymisation.rs` — the §11.16.7 test, and the one
that must be red before the feature exists.

1. Build a fixture `TestDb`, then insert **one row per identifying shape** with
   a *known plaintext* PII string: `accounts.email = 'leak-canary@example.invalid'`,
   `accounts.handle = 'leak_canary_handle'`, a `display_name`, a
   `password_hash` sentinel, a session token sentinel.
2. Run the snapshot pipeline into a temp dir.
3. **Read the output as bytes and assert no canary appears anywhere in it** —
   not in a table, not in a comment, not in a tool version string:
   ```rust
   let bytes = std::fs::read(&dump_path).expect("read the dump");
   let haystack = String::from_utf8_lossy(&bytes);
   for canary in CANARIES {
       assert!(!haystack.contains(canary), "the snapshot leaked {canary}");
   }
   ```
4. Assert the `pseud_id` join still resolves: two tables that share a `pseud_id`
   must share its replacement.
5. Restore into a fresh database and run the schema check.

**Step 3 reads raw bytes on purpose.** A test that queries the restored database
cannot see a canary in a `pg_dump` comment, and that is exactly where a real
leak survives a review.

**Verification:** red before the masking exists, green after. Record both.

### D1.4 Wire the gate into CI

Same shape as `check-sqlite-migration-syntax.py`, which is already wired:

```yaml
      - name: Snapshot PII policy covers every column
        run: |
          python3 scripts/check-snapshot-pii.py --self-test
          python3 scripts/check-snapshot-pii.py
```

---

## Phase D2 — Producing a snapshot

### D2.1 The pipeline script

`scripts/take-snapshot.sh`, or a Rust binary under `crates/app/src/bin/` if the
repo prefers the binary — **decide by looking at how `doctor` is invoked, and
match it.**

```bash
scripts/take-snapshot.sh \
  --out ~/.local/share/lorehaven/snapshots \
  --namespace lorehaven-snapshot-v1
```

Order of operations, and the order is a security property:

1. **`pg_dump` first, masked at dump time.** Anonymize at the source, never dump
   raw and scrub afterwards — a raw dump on disk is a raw dump, whatever happens
   next, and that file is the thing an attacker wants.
2. `--exclude-table` for every `drop_table` entry.
3. `--no-security-labels` if any masking metadata could ride along.
4. Compress with `zstd` (better ratio than gzip at this size).
5. `age`-encrypt with a **fresh** passphrase (§11.17.3).
6. **Delete the intermediate** and confirm it is gone. `shred` on spinning
   disks, plain `rm` on SSD/flash — and the comment must say why it is not
   `shred`.
7. Write the manifest (§11.17.4) — date, rule version, row counts, **no
   operator, no host**.

### D2.2 The restore check is part of producing, not a follow-up

A snapshot that does not restore is worse than no snapshot, because it is
discovered by a recipient. So the pipeline itself restores into a throwaway
database and refuses to publish on failure:

```bash
scripts/take-snapshot.sh ... || echo "REFUSING TO PUBLISH"
```

**Verification:** run it against a copy of the instance, restore, and
`cargo run -p lorehaven-app -- doctor` against the restore.

---

## Phase D3 — Distribution

**This phase is mostly operational, and the spec is already the specification.**
What needs building is the *check*, not the transfer.

### D3.1 A preflight script that refuses to publish over clearnet

`scripts/check-snapshot-channel.sh` — because "did I remember to use Tor" is
exactly the kind of thing that is remembered once.

It must refuse when: the target is a hostname that resolves publicly; no
`.onion` is present; a clearnet torrent client is detected; the file is not
encrypted; the passphrase equals the last snapshot's; the I2P destination is
the last one's.

The last three need state — a small manifest directory recording the previous
snapshot's I2P destination, timestamp offset, and passphrase **fingerprint**
(never the passphrase).

**Verification:** each refusal must be demonstrated by running it, not by
reading the script. Six refusals, six runs.

### D3.2 The runbook

`docs/runbooks/publishing-a-snapshot.md` — a person-facing procedure, because a
monthly task done from memory is a monthly opportunity to get it wrong. It must
include the **"stop and do not publish"** list, which is the part that matters
when something has already gone wrong.

---

## Phase D4 — Cadence

A systemd timer or cron entry, monthly, with **preconditions**: refuse to run if
the last snapshot is less than 25 days old. Publishing on the 1st and again
"because it was quick" is the correlation risk the spec is built against, and a
timer is the only thing that enforces a floor.

---

## What must NOT be built, and why

* **No clearnet distribution path, not even a documented one.** A documented
  exception becomes a used exception.
* **No "just the works and chapters" mode.** That is the dataset, and the
  audience rules in §7.7 are not expressible in a dump.
* **No per-snapshot `pseud_id` salt.** §11.16.3 requires the namespace to be
  published *and stable*, or the snapshot's central property stops being
  checkable. Per-snapshot randomness is in §11.17.4's *timestamp* offset, not
  here.
* **No upload automation that includes the passphrase in the same request.**
  §11.17.2 separates the channels; the automation must not undo that by being
  convenient.

---

## Open question for the owner, which does not block D1

**Is the snapshot for other Lorehaven instances, for research, or both?** The
answer changes what "useful" means and therefore how far the `pseud_id` graph
must survive:

* *other instances* — the graph must be **internally** consistent, and the
  cross-snapshot correlation in §11.17.4 is a real cost paid for that;
* *research* — you additionally need the `works`/`chapters` structure and
  timestamps intact, and probably **not** the social graph at all, which would
  let D1.2 be much simpler.

D1 is worth doing either way. Answer it before D2, not before D1.
