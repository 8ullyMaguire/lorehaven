# Publishing a snapshot

A monthly procedure. It is written down because a monthly task done from memory
is a monthly opportunity to get it wrong, and the mistakes it prevents are not
recoverable: a dump published in October cannot be un-published in December.

Two requirements, independent, and **both** must hold. There is no path where one
substitutes for the other (§11.17.1):

| | Requirement | Fails how |
|---|---|---|
| **File** | no identifying value in the bytes (§11.16) | a reader of the dump learns who |
| **Channel** | no IP, hostname, or location revealed (§11.17) | an observer of the network learns where |

Anonymised data with a timing side channel is still an operator identification.
An encrypted dump on clearnet is still a location disclosure to whoever observes
the connection.

## STOP AND DO NOT PUBLISH

Stop. Nothing below this line is worth the risk. Each item is a case where the
publication is already a disclosure, not a mistake that can be corrected.

1. **The target is not a `.onion` address or an I2P destination.** Any clearnet
   host — object storage, a public git repository, a paste site, a torrent
   tracker, email as a file channel, a "temporary" direct link. Every one records
   your address, and several retain it after the file is removed. *A monthly
   cadence makes this worse than a one-off*: a recurring upload from a stable
   address is a subscription to being located.
2. **You are considering a clearnet torrent behind a VPN.** The VPN hides your
   address from the tracker. It does not hide it from the swarm. The peer set
   still learns it.
3. **The passphrase has been used before.** A reused passphrase means one
   intercepted file is a standing key to every other. The preflight refuses this
   by comparing fingerprints, so you cannot argue with it.
4. **The I2P destination is last month's.** A reused destination lets a recipient
   correlate every month's dump to one identity, which is the entire threat.
5. **The file is not encrypted at rest** — or is compressed but not encrypted. A
   `.sql.zst` handed to someone is readable by them.
6. **You have run `doctor` against anything other than a restore of this exact
   file.** A snapshot that does not restore is worse than no snapshot, because a
   recipient is the one who discovers it.
7. **The run is on the host that serves Lorehaven.** The sharing service belongs
   in a dedicated VM or container. Compromising the share host must not be a
   path to the instance.
8. **You are tempted to put the passphrase in the same request as the link.**
   §11.17.2 separates the channels. Automation that undoes the separation by
   being convenient has removed the only part that was protecting you.

If you have already published: say so in the operator channel with the date and
the channel used, and treat the dump as disclosed. It cannot be recalled.

## Procedure

### 1. Build the snapshot

```bash
scripts/take-snapshot.sh --out ~/.local/share/lorehaven/snapshots
```

The order inside is a security property, not a preference:

1. `pg_dump` **first**, masked at dump time. Never dump raw and scrub
   afterwards — a raw dump on disk is a raw dump whatever happens next, and that
   file is what an attacker wants.
2. `--exclude-table` for every `drop_table` entry in the policy.
3. `zstd` compress.
4. `age`-encrypt with a **fresh** passphrase.
5. Delete the intermediate and confirm it is gone.
6. Write the manifest: date, rule version, row counts. **No operator, no host.**

### 2. Verify it restores

```bash
createdb -p 5433 lorehaven_restore_check
zstd -d -c <file>.age | age -d > /tmp/restore.sql   # or: age -d -i key
psql -h 127.0.0.1 -p 5433 -d lorehaven_restore_check -v ON_ERROR_STOP=1 -f /tmp/restore.sql
LOREHAVEN_DATABASE_URL=postgresql:///lorehaven_restore_check \
    cargo run -q -p lorehaven-app -- doctor --strict
dropdb -p 5433 lorehaven_restore_check
shred -u /tmp/restore.sql
```

`doctor` takes the database from `LOREHAVEN_DATABASE_URL`, not a flag, and
`--strict` turns warnings into failures — which is what "passes doctor" has to
mean for this check to be worth anything.

If `doctor` fails, **do not publish.** The script refuses on failure; that
refusal is the requirement, not a convenience.

### 3. Check the channel

```bash
read -rs PASSPHRASE && echo "$PASSPHRASE" | scripts/check-snapshot-channel.py \
    --target "$ONION" \
    --file ~/.local/share/lorehaven/snapshots/lorehaven-YYYY-MM.sql.zst.age \
    --state-dir ~/.local/share/lorehaven/snapshot-state \
    --timestamp-offset "$OFFSET" \
    --passphrase-fpr-stdin --record
```

`$OFFSET` is the value `take-snapshot.sh` printed at the start of the run. It is
deliberately in neither the dump nor the manifest: an offset recorded beside the
data is a **published** offset, and two dumps shifted by the same amount join
row-for-row on every timestamp (§11.16.5). The generated SQL shows only that a
shift happened, which §11.16.3 needs for the construction to be checkable.

The passphrase is read on **stdin**, never as an argument — an argument is
visible in `ps` to every process on the machine.

`--self-test` proves the check can fail. Run it before trusting a pass:

```bash
scripts/check-snapshot-channel.py --self-test
```

### 4. Distribute the link and the passphrase on different channels

This is the part that is easy to undo by accident. The `.onion` goes by one
channel, the passphrase by another, and they never travel together — not in the
same message, not in the same repository, not "just this once".

### 5. Record the publication

The `--record` flag writes the passphrase **fingerprint** (salted, never the
passphrase), the destination, the **offset value**, the date, the rule version and
the file name. The offset is stored in the clear on purpose: it is not a secret,
it is a fact about which snapshots must not be aligned, and the operator needs to
read it when a refusal is unexplained. It is absent from the *dump*; the state
directory is not published. This file is what makes next month's rotation
check possible; if it is lost, the next snapshot cannot be proven to have
rotated.

## What this does not do

Stated so it is not oversold:

- **Not a backup.** A de-identified snapshot cannot restore an instance and must
  never be presented as one.
- **Not synchronisation.** One-way, periodic, no update path, no conflict
  resolution.
- **Not a way to make you anonymous.** It stops a dump carrying identities and a
  transfer carrying an address. An operator who participates in a public forum
  under a real identity is findable by other means.
- **Not a protection for readers from you.** A published snapshot is a
  disclosure you make on your readers' behalf. §3.4 is unaffected.

The per-snapshot `pseud_id` re-key is **deliberately stable** (§11.16.3) so the
graph is usable and checkable. That is also exactly what makes two snapshots
joinable by anyone holding both. The per-snapshot variation lives in the
unpublished timestamp offset, never in the salt.
