# 22. Operations, deployment and budgets

**Status:** reserved
**Milestone:** M32
**Source:** the corresponding section of the Darkhold specification

## 22.1 The artefact

**One binary** (plus the optional taste service), the frontend embedded, migrations bundled. One
artefact to deploy, back up, version and roll back.

Deployment: build on a compile host, copy the binary to **local disk** (never an NFS mount — a
wedged mount must not be able to SIGBUS the process), restart the service, health-check, and roll
back automatically if the health check fails.

## 22.2 Configuration and secrets

Secrets come from the environment or a file with restrictive permissions; never from the database,
never from the config file in production. Secrets at rest are encrypted with ChaCha20-Poly1305 and
keys are rotatable. Passwords are Argon2.

## 22.3 The budgets

**A budget with no measured baseline is a wish, not a budget.** The rule is therefore a
**provenance rule**, not a number:

> **Every cap in this section names its provenance.** A cap is either (a) **measured** — a
> figure produced by running the real binary against the real data on the target hardware — or
> (b) **derived** — a number computed from a measured baseline plus a stated margin. **A cap
> with neither is not a cap**, and the budget gate treats it as a failure.

**M1 MUST produce the archive's measured RSS baseline** before any cap here is set, and §22.3's
archive figures are written *against* that measurement rather than in advance of it. The gate
reports **three** numbers — the baseline, the current figure, and the delta — so a regression
is a delta and not a reading of an absolute.

**The gate reads content, not status.** This is not a stylistic preference; it is a recorded
failure. kindred's earlier gate accepted a `503` and reported **PASS** against a server with no
index loaded, and a `200` whose body said `"returned": 0` was also a pass. The archive's gate:

- **refuses to run** if the corpus is unreadable — *"measuring nothing is not a pass"*;
- **waits for `/health/ready`** rather than sleeping a guessed interval;
- **asserts on the body** — a `200` with an empty result set is a **failure**, and a `503` is a
  **failure**;
- **picks a real seed** — recommending from a non-existent id exercises the 404 path, which
  allocates nothing, and so measures nothing;
- **prints the delta per step**, so a failure **names the request**.

**The design frame is a Raspberry Pi 5B with 4 GB of RAM** running the archive, the taste service
and Postgres, alongside a general-purpose box. The develop-and-deploy host is `thinkcentre`.

| budget | cap | measured where |
|---|---|---|
| archive RSS | **≤ 1 GiB** | `darkhold doctor` and the budget gate |
| taste RSS, full mode (T2) | **220 MiB** | `make budget`, `VmHWM` after each step; `/healthz` 503 when over |
| taste RSS, lite mode (T1) | **60 MiB** | same |
| catalogue ingest | **≤ 180 MiB peak** | the ingest phase's own cap |
| catalogue embed (dim 32) | **≤ 205 MiB peak** | the embed phase's own cap |
| binary size | **≤ 60 MB** | the release build |
| cold start | **≤ 3 s** for the archive; the taste service's start is its index load | the deploy gate |
| recommendation latency | **≤ 150 ms** at n=5, ≤ 300 ms at n=50 | the latency gate |
| **Pi 5B total footprint** | **≤ 2.5 GiB** including Postgres | the Pi deployment gate — the number that decides whether the tier model is right |

**The budget gate boots the real binary against the real catalogue, walks the routes, and reads
the kernel's high-water mark after each step.** It refuses to run if the catalogue is unreadable
("measuring nothing is not a pass") and it prints the delta per step so a failure **names the
request**.

## 22.4 Health and doctor

`/health/live` (process) · `/health/ready` (database, schema, storage) · `/api/v1/meta` (instance
metadata, including the active content policy, the **published instance theme**, and the **tier
thresholds**) · `/healthz` on the taste service (including the RSS headroom) · `darkhold doctor`.

## 22.5 Backups and recovery

The database and the storage are backed up together, and the backup is **verified by restore**, not
by the existence of a file. The catalogue is a derived artefact and is regenerated, not backed up.

---
