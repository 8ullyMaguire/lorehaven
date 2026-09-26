#!/usr/bin/env python3
"""Seed the roadmap consensus board (spec §44) from docs/requirements.csv.

Maps every requirements.csv row to a roadmap card:
  - area            -> category
  - requirement     -> title
  - status          -> stage:
      implemented-locally-tested | implemented-fixture-tested |
      implemented-but-not-executed -> shipped
      planned                         -> idea (CSV carries no stage detail)
      unsupported                     -> rejected
      external-integration-not-live   -> finished
      partially-implemented           -> in_progress

Upserts by normalized title (lowercase, whitespace collapsed, punctuation
stripped), so re-running is idempotent. Never downgrades: a card already in
`shipped` stays `shipped` whatever the CSV says; a `rejected` card is never
resurrected by the seed (the operator moves it back manually).

Usage:
  python3 scripts/seed_roadmap.py --dry-run                    # plan only
  python3 scripts/seed_roadmap.py --db-url sqlite:///data/lorehaven.sqlite
  python3 scripts/seed_roadmap.py --db-url postgres://...      # or DATABASE_URL

Idempotent: exits 0, prints a per-card action report
(inserted / updated / unchanged / protected).
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import re
import sqlite3
import sys
import uuid
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CSV_PATH = REPO_ROOT / "docs" / "requirements.csv"

STAGES = (
    "idea",
    "up_next",
    "in_progress",
    "finished",
    "shipped",
    "medium_term",
    "long_term",
    "rejected",
)

# CSV status -> stage. Unknown statuses are a hard error (fail loudly, never
# silently mislabel a card).
STATUS_TO_STAGE = {
    "implemented-locally-tested": "shipped",
    "implemented-verified-e2e": "shipped",
    # Added after the seeder was found hard-erroring on it: 39 rows carried this
    # status and the script refused to process the file at all. A status that
    # describes the *strongest* claim in the vocabulary being the one the tool
    # rejects is how a tracker stops being trustworthy -- the next person adds
    # another one, and the failure mode is a tool that has never run end to end
    # on the current data.
    "implemented-fully-tested": "shipped",
    "implemented-fixture-tested": "shipped",
    "implemented-but-not-executed": "shipped",
    "external-integration-not-live": "finished",
    "partially-implemented": "in_progress",
    "planned": "idea",
    "unsupported": "rejected",
}

# Stages the seed may never move a card OUT of (operator-only, §44.6).
PROTECTED = {"shipped", "rejected", "up_next", "in_progress", "finished"}


def normalize_title(title: str) -> str:
    """Normalized comparison key: lowercase, punctuation stripped, ws collapsed."""
    lowered = title.lower()
    stripped = re.sub(r"[^\w\s]", "", lowered, flags=re.UNICODE)
    return re.sub(r"\s+", " ", stripped).strip()


def now_iso() -> str:
    import datetime

    return datetime.datetime.now(datetime.timezone.utc).isoformat()


@dataclass
class CardRow:
    id: str
    title: str
    category: str
    stage: str


def load_csv() -> list[tuple[str, str, str, str]]:
    """(req_id, title, category, stage) per CSV row, skipping header."""
    out: list[tuple[str, str, str, str]] = []
    with open(CSV_PATH, newline="", encoding="utf-8") as f:
        for row in csv.DictReader(f):
            status = row["status"].strip()
            if status not in STATUS_TO_STAGE:
                sys.exit(f"error: unknown status {status!r} on row {row['id']!r}")
            out.append(
                (row["id"], row["requirement"].strip(), row["area"].strip(),
                 STATUS_TO_STAGE[status])
            )
    if not out:
        sys.exit("error: no rows parsed from requirements.csv")
    return out


# ── SQLite backend ──────────────────────────────────────────────────────────


def seed_sqlite(conn: sqlite3.Connection, rows, dry_run: bool) -> dict[str, int]:
    report = {"inserted": 0, "updated": 0, "unchanged": 0, "protected": 0}
    report_lines: list[str] = []

    conn.execute(
        "CREATE TABLE IF NOT EXISTS roadmap_cards ("
        " id TEXT PRIMARY KEY, title TEXT NOT NULL,"
        " category TEXT NOT NULL DEFAULT 'general',"
        " stage TEXT NOT NULL DEFAULT 'idea'"
        "  CHECK (stage IN ('idea','up_next','in_progress','finished',"
        "                   'shipped','medium_term','long_term','rejected')),"
        " elo_rating REAL NOT NULL DEFAULT 1500.0,"
        " matches_played INTEGER NOT NULL DEFAULT 0,"
        " times_best INTEGER NOT NULL DEFAULT 0,"
        " times_worst INTEGER NOT NULL DEFAULT 0,"
        " created_at TEXT NOT NULL, updated_at TEXT NOT NULL)"
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_roadmap_cards_stage ON roadmap_cards (stage)"
    )

    for req_id, title, category, stage in rows:
        existing = conn.execute(
            "SELECT id, stage FROM roadmap_cards WHERE title = ?", (title,)
        ).fetchone()
        if existing is None:
            action = "inserted"
            if not dry_run:
                conn.execute(
                    "INSERT INTO roadmap_cards"
                    " (id, title, category, stage, created_at, updated_at)"
                    " VALUES (?, ?, ?, ?, ?, ?)",
                    (str(uuid.uuid4()), title, category, stage, now_iso(), now_iso()),
                )
        else:
            card_id, current_stage = existing
            if current_stage in PROTECTED and current_stage != stage:
                action = "protected"
            elif current_stage == stage:
                action = "unchanged"
                if not dry_run:
                    conn.execute(
                        "UPDATE roadmap_cards SET category = ?, updated_at = ?"
                        " WHERE id = ?",
                        (category, now_iso(), card_id),
                    )
            else:
                action = "updated"
                if not dry_run:
                    conn.execute(
                        "UPDATE roadmap_cards SET category = ?, stage = ?,"
                        " updated_at = ? WHERE id = ?",
                        (category, stage, now_iso(), card_id),
                    )
        report[action] += 1
        report_lines.append(f"  {action:10s} {req_id:8s} [{stage:11s}] {title[:70]}")

    if not dry_run:
        conn.commit()
    for line in report_lines:
        print(line)
    return report


# ── Postgres backend ────────────────────────────────────────────────────────


def seed_postgres(dsn: str, rows, dry_run: bool) -> dict[str, int]:
    try:
        import psycopg  # psycopg3
    except ImportError:
        try:
            import psycopg2 as pg2
        except ImportError:
            sys.exit(
                "error: Postgres target needs psycopg (pip install psycopg"
                "[binary]) or psycopg2"
            )
        conn = pg2.connect(dsn)
        param = "%s"
        import psycopg2.extensions

        if hasattr(conn, "autocommit"):
            conn.autocommit = False
    else:
        conn = psycopg.connect(dsn)
        param = "%s"

    report = {"inserted": 0, "updated": 0, "unchanged": 0, "protected": 0}
    report_lines: list[str] = []
    cur = conn.cursor()
    try:
        # Self-sufficient: create the table if migration 0066 has not run yet,
        # so the script can seed a deployed instance ahead of the Rust milestone.
        cur.execute(
            """
            CREATE TABLE IF NOT EXISTS roadmap_cards (
                id              TEXT PRIMARY KEY,
                title           TEXT NOT NULL,
                category        TEXT NOT NULL DEFAULT 'general',
                stage           TEXT NOT NULL DEFAULT 'idea'
                                CHECK (stage IN ('idea','up_next','in_progress',
                                                 'finished','shipped','medium_term',
                                                 'long_term','rejected')),
                elo_rating      REAL NOT NULL DEFAULT 1500.0,
                matches_played  INTEGER NOT NULL DEFAULT 0,
                times_best      INTEGER NOT NULL DEFAULT 0,
                times_worst     INTEGER NOT NULL DEFAULT 0,
                created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
            )
            """
        )
        cur.execute(
            "CREATE INDEX IF NOT EXISTS idx_roadmap_cards_stage"
            " ON roadmap_cards (stage)"
        )
        conn.commit()
        for req_id, title, category, stage in rows:
            cur.execute(
                f"SELECT id, stage FROM roadmap_cards WHERE title = {param}",
                (title,),
            )
            existing = cur.fetchone()
            if existing is None:
                action = "inserted"
                if not dry_run:
                    cur.execute(
                        "INSERT INTO roadmap_cards"
                        " (id, title, category, stage, created_at, updated_at)"
                        f" VALUES ({param}, {param}, {param}, {param}, now(), now())",
                        (str(uuid.uuid4()), title, category, stage),
                    )
            else:
                card_id, current_stage = existing[0], existing[1]
                if current_stage in PROTECTED and current_stage != stage:
                    action = "protected"
                elif current_stage == stage:
                    action = "unchanged"
                    if not dry_run:
                        cur.execute(
                            f"UPDATE roadmap_cards SET category = {param},"
                            f" updated_at = now() WHERE id = {param}",
                            (category, card_id),
                        )
                else:
                    action = "updated"
                    if not dry_run:
                        cur.execute(
                            f"UPDATE roadmap_cards SET category = {param},"
                            f" stage = {param}, updated_at = now()"
                            f" WHERE id = {param}",
                            (category, stage, card_id),
                        )
            report[action] += 1
            report_lines.append(
                f"  {action:10s} {req_id:8s} [{stage:11s}] {title[:70]}"
            )
        if not dry_run:
            conn.commit()
    finally:
        cur.close()
        conn.close()
    for line in report_lines:
        print(line)
    return report


def main() -> int:
    global CSV_PATH
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument(
        "--db-url",
        default=os.environ.get("DATABASE_URL", "sqlite:///data/lorehaven.sqlite"),
        help="sqlite:///path or postgres:// DSN (default: DATABASE_URL or"
        " sqlite:///data/lorehaven.sqlite)",
    )
    ap.add_argument("--csv", default=str(CSV_PATH), help="requirements.csv path")
    ap.add_argument(
        "--dry-run", action="store_true", help="print the plan, change nothing"
    )
    args = ap.parse_args()

    CSV_PATH = Path(args.csv)
    rows = load_csv()
    print(f"seeding {len(rows)} rows from {CSV_PATH} -> {args.db_url}"
          f"{' (dry run)' if args.dry_run else ''}")

    if args.db_url.startswith("sqlite:///"):
        path = args.db_url.removeprefix("sqlite:///")
        conn = sqlite3.connect(path)
        try:
            report = seed_sqlite(conn, rows, args.dry_run)
        finally:
            conn.close()
    elif args.db_url.startswith(("postgres://", "postgresql://")):
        report = seed_postgres(args.db_url, rows, args.dry_run)
    else:
        sys.exit(f"error: unsupported db-url {args.db_url!r}")

    print(json.dumps(report, indent=2))
    if args.dry_run:
        print("dry run: nothing committed")
    return 0


def self_test() -> int:
    """Every status in the CSV must be a status this script understands.

    The vocabulary grew without the map growing with it: 39 rows carried
    `implemented-fully-tested` -- the *strongest* claim the file makes -- and
    `load_csv` hard-errored on the first one, so the seeder had never run
    against the current data. A tracker whose strongest claim its own tooling
    rejects is worse than one that admits it is incomplete, because the failure
    surfaces as an error on a tool nobody runs on every change.

    So the check is a check, not a comment: a new status without a mapping is a
    failing exit code.
    """
    statuses: dict[str, int] = {}
    for row in read_rows():
        statuses[row["status"]] = statuses.get(row["status"], 0) + 1

    unknown = {s: n for s, n in statuses.items() if s not in STATUS_TO_STAGE}
    if unknown:
        detail = ", ".join(f"{s!r} x{n}" for s, n in sorted(unknown.items()))
        print(f"error: statuses with no stage mapping: {detail}", file=sys.stderr)
        print("add each to STATUS_TO_STAGE, or correct the spelling in the CSV", file=sys.stderr)
        return 1

    print("status vocabulary ok: " + ", ".join(f"{s} ({n})" for s, n in sorted(statuses.items())))
    return 0


def read_rows() -> list[dict[str, str]]:
    """CSV rows as dicts, without the stage mapping this module also needs."""
    with open(CSV_PATH, newline="", encoding="utf-8") as handle:
        return list(csv.DictReader(handle))


if __name__ == "__main__":
    if "--self-test" in sys.argv:
        raise SystemExit(self_test())
    raise SystemExit(main())
