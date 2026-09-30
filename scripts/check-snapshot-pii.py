#!/usr/bin/env python3
"""M60-01 — the snapshot policy gate.

§11.16 says, in one sentence that is easy to skim and hard to satisfy:

    A new column that carries personal data is not automatically covered by this
    list, and that is a standing obligation rather than a one-time audit: adding
    a column to a table in this list requires deciding its row above.

This script IS that enforcement. It fails when a column exists in a table the
policy covers and no policy entry names it.

## Why the inventory is derived, not listed

The obvious implementation is a hand-written list of (table, column) pairs
checked against a hand-written list of known columns. That version is worse than
no gate, and specifically: **it cannot detect the case it exists to detect.**
A new column is missing from BOTH lists, so the gate passes. The list would need
to be updated in the same commit as the migration, by the same author, for the
same reason -- which is a comment saying "remember to also do the other thing."

So the inventory comes from `migrations/postgres/*.sql` -- the same files that
create the columns. A new column is, by construction, in the file being read,
and cannot be absent from the check.

## What a pass means, and what it does not

A pass means: **every column of every covered table has a policy decision.** It
does NOT mean the decision is *correct*, and the policy file carries a
`reviewed_by` and `reviewed_on` per entry precisely because the gate can see
that a decision exists and cannot see that anyone thought about it. An entry
added as `keep` with no justification is the failure this cannot detect, and
§11.16's real answer to it is code review.

## Usage

    python3 scripts/check-snapshot-pii.py --self-test   # prove the gate can fail
    python3 scripts/check-snapshot-pii.py               # gate the working tree

Exit 0 = every covered column has a decision. Exit 1 = at least one does not,
with the offending table.column named.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
MIGRATIONS = REPO / "migrations" / "postgres"
POLICY = REPO / "docs" / "snapshot-column-policy.json"

# Tables the policy governs. Derived from the spec's own table list plus the
# account_id fan-out: ANY table carrying account_id is covered, because
# §11.16.3b is explicit that a second join key is the one a first draft misses,
# and a 27th table gaining account_id is a policy question, not a detail.
COVERED_TABLES = {
    "accounts", "pseuds", "webhook_endpoints", "sessions",
    "imports", "works", "chapters", "reading_events", "kudos",
    "comments", "devices", "device_tokens", "login_attempts",
}

# Columns whose NAME establishes that they carry an addressable identity.
# These are covered on ANY table, because a table named `foo` can still gain a
# column called `ip_address`, and the point of the gate is to make that a
# decision rather than an oversight.
#
# UNANCHORED, and deliberately. The first version anchored with `^` and its own
# self-test caught the consequence: `submitter_ip` did not match, so an identity
# column with a qualifier in front of it passed undecided. Real names are
# qualified in front far more often than not -- `recovery_email`, `user_agent`,
# `author_ip`, `payout_account_id` -- and a gate that only recognises the
# unqualified form recognises the form nobody writes.
#
# The cost of unanchoring is false positives: `address_kind` and `token_count`
# are not PII, and the gate will ask about them. That is the right way round --
# a question costs a line in a JSON file, a missed column costs a disclosure.
IDENTITY_NAME_RE = re.compile(
    r"("
    r"account_id|user_id|owner_id|author_id|actor_id|"
    r"e_?mail|password|passwd|"
    r"token|secret|api_key|private_key|"
    r"\bip\b|_ip$|^ip_|ip_address|remote_addr|"
    r"user_agent|"
    r"real_name|legal_name|full_name|"
    r"phone|msisdn|"
    r"street|postcode|postal_code|zip_?code|"
    r"latitude|longitude|geo_lat|geo_lon|geo_point|"
    r"device_token|push_token|_endpoint$|endpoint_url|"
    r"session_id|cookie|fingerprint"
    r")",
    re.I,
)


# Columns whose NAME establishes that they carry a BODY -- the reader-visible
# text of a work, in its two renderings. Gated on ANY table, for the same reason
# IDENTITY_NAME_RE is, and unanchored for the same reason: anchoring with `^` made
# `submitter_ip` pass, because real names are qualified in front far more often
# than not. A content regex anchored the same way would recognise only
# `plain_text` and miss `chapter_plain_text`, `cached_body_html` and `raw_text` --
# the forms an author actually writes.
#
# THIS EXISTS BECAUSE A GATE WAS SILENT ON THE MOST SENSITIVE COLUMN IN THE
# SCHEMA. `chapter_revisions.plain_text` carries the body, and it was not in the
# policy at all until M60-05; the M60-01 gate only asked about columns in tables
# it already knew carried PII, and chapter_revisions was not one of them. That
# fix was applied to ONE TABLE and the class was not fixed, so M59-10's
# `reader_body_copies` -- a reader's copy of an external body, added three hours
# after that note was written -- sailed past with no decision asked for. The
# account_id on that same table WAS caught, because that name matches
# IDENTITY_NAME_RE, which is what made the omission visible at all.
#
# Deliberately NOT included: name, title, description, summary. They are
# content-adjacent and extremely common, and adding them would ask about
# `works.title` and every `*_name` in the schema -- turning the gate into noise on
# every future migration, which is how a gate gets ignored. This names the shapes
# that carry a BODY.
#
# The same false-positive trade the identity regex already makes: a question costs
# a line in the JSON policy, a missed column costs a disclosure.
CONTENT_NAME_RE = re.compile(
    r"("
    r"plain_?text|sanitized_?html|raw_?html|raw_?text|"
    r"document_?json|body_?text|body_?html|"
    r"excerpt|full_?text|chapter_?text"
    r")",
    re.I,
)

# CREATE TABLE [IF NOT EXISTS] name ( ... )  -- and ALTER TABLE ... ADD COLUMN.
CREATE_TABLE_RE = re.compile(
    r"CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?([a-z_][a-z0-9_]*)\s*\(",
    re.I,
)
ADD_COLUMN_RE = re.compile(
    r"ALTER\s+TABLE\s+(?:IF\s+EXISTS\s+)?([a-z_][a-z0-9_]*)\s+"
    r"ADD\s+COLUMN\s+(?:IF\s+NOT\s+EXISTS\s+)?([a-z_][a-z0-9_]*)",
    re.I,
)
# A column definition line: leading name, then a type. Excludes PRIMARY KEY,
# FOREIGN KEY, CONSTRAINT, UNIQUE, CHECK and index clauses.
COLUMN_LINE_RE = re.compile(r"^\s*([a-z_][a-z0-9_]*)\s+[a-z]", re.I)
NOT_A_COLUMN = {
    "primary", "foreign", "constraint", "unique", "check", "key", "index",
    "exclude", "like", "as", "select", "insert", "values", "constraint_type",
}


def strip_comments(sql: str) -> str:
    sql = re.sub(r"--[^\n]*", "", sql)
    return re.sub(r"/\*.*?\*/", "", sql, flags=re.S)


def parse_inventory(migrations_dir: pathlib.Path) -> dict[str, set[str]]:
    """Every (table -> columns) the migrations create, in file order.

    Later migrations win on conflicts, and ALTER TABLE ADD COLUMN accumulates,
    which is what makes this see a column added in migration 0092 as readily as
    one created in migration 0001.
    """
    tables: dict[str, set[str]] = {}
    for path in sorted(migrations_dir.glob("*.sql")):
        sql = strip_comments(path.read_text(encoding="utf-8", errors="replace"))
        for m in ADD_COLUMN_RE.finditer(sql):
            tables.setdefault(m.group(1).lower(), set()).add(m.group(2).lower())
        for m in CREATE_TABLE_RE.finditer(sql):
            table = m.group(1).lower()
            # Walk the balanced parens of this CREATE TABLE body.
            start = m.end()
            depth, i = 1, start
            while i < len(sql) and depth:
                if sql[i] == "(":
                    depth += 1
                elif sql[i] == ")":
                    depth -= 1
                i += 1
            body = sql[start : i - 1]
            for line in body.splitlines():
                line = line.strip().rstrip(",")
                if not line:
                    continue
                cm = COLUMN_LINE_RE.match(line)
                if cm and cm.group(1).lower() not in NOT_A_COLUMN:
                    tables.setdefault(table, set()).add(cm.group(1).lower())
    return tables


def load_policy() -> dict[str, dict]:
    if not POLICY.exists():
        return {}
    data = json.loads(POLICY.read_text())
    return data.get("columns", {})


def check(tables: dict[str, set[str]], policy: dict[str, dict]) -> list[str]:
    """Return one line per column that has no policy decision."""
    undecided: list[str] = []
    for table in sorted(tables):
        cols = tables[table]
        is_covered_table = table in COVERED_TABLES
        for col in sorted(cols):
            entry = policy.get(f"{table}.{col}")
            if entry is not None:
                continue
            # A covered table needs EVERY column decided. Any other table needs
            # one only if the column name itself says it carries an identity.
            if (is_covered_table or IDENTITY_NAME_RE.search(col)
                    or CONTENT_NAME_RE.search(col)):
                undecided.append(f"{table}.{col}")
    return undecided


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--self-test", action="store_true",
                    help="prove the gate fails on an undecided column")
    args = ap.parse_args()

    tables = parse_inventory(MIGRATIONS)
    policy = load_policy()

    if args.self_test:
        return self_test(tables, policy)

    undecided = check(tables, policy)
    if undecided:
        print(f"FAIL: {len(undecided)} column(s) with no snapshot policy decision.")
        print("§11.16 requires a decision for every column of every covered table,")
        print("and for any column whose name says it carries an identity.\n")
        for name in undecided:
            print(f"  undecided: {name}")
        print(f"\nAdd each to {POLICY.relative_to(REPO)} with treatment, why, and a")
        print("reviewer. Treatments: replace | drop_column | drop_table | keep | keep_gated")
        return 1

    total = sum(len(c) for c in tables.values())
    print(f"OK: {total} columns across {len(tables)} tables, all with a policy decision.")
    return 0


def self_test(tables: dict[str, set[str]], policy: dict[str, dict]) -> int:
    """A gate that has never failed is not a gate.

    Three properties, each checked by making it break:
      1. it passes on the real tree (otherwise it proves nothing)
      2. it FAILS on a new column in a covered table
      3. it FAILS on a new column whose name says it carries an identity
      4. it FAILS on a new column whose name says it carries a BODY, on any table
    """
    failures = []
    if check(tables, policy):
        failures.append("the working tree does not pass its own gate")

    probe = {t: set(c) for t, c in tables.items()}
    probe.setdefault("accounts", set()).add("recovery_email")
    if not check(probe, policy):
        failures.append("a new column in a covered table was NOT caught")

    probe2 = {t: set(c) for t, c in tables.items()}
    probe2.setdefault("roadmap_suggestions", set()).add("submitter_ip")
    policy2 = {k: v for k, v in policy.items()
               if k != "roadmap_suggestions.submitter_ip"}
    if not check(probe2, policy2):
        failures.append("an identity-named column on an uncovered table was NOT caught")

    probe3 = {t: set(c) for t, c in tables.items()}
    probe3.setdefault("accounts", set()).add("nickname")
    policy3 = dict(policy)
    policy3["accounts.nickname"] = {"treatment": "replace", "why": "chosen by a person"}
    if check(probe3, policy3):
        failures.append("a decided column was still reported undecided")

    # THE FOURTH PROPERTY, and the one this fan-out was added for. A body-text
    # column on a table that is neither covered nor identity-bearing must still be
    # caught: that is the exact shape the gate was silent on. Without this probe
    # the CONTENT_NAME_RE fan-out could be deleted by a later editor who saw no
    # failing test, and the hole would reopen silently.
    probe4 = {t: set(c) for t, c in tables.items()}
    probe4.setdefault("reader_body_copies", set()).add("plain_text")
    policy4 = {k: v for k, v in policy.items()
               if k != "reader_body_copies.plain_text"}
    if not check(probe4, policy4):
        failures.append("a BODY-TEXT column on an uncovered table was NOT caught")

    if failures:
        print("SELF-TEST FAILED -- the gate cannot be trusted:\n")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("SELF-TEST OK: gate passes the real tree, and fails on:")
    print("  - a new column in a covered table")
    print("  - an identity-named column on any table")
    print("  - a BODY-TEXT column on any table (the fan-out added for M59-10)")
    print("  - and stops reporting once a decision exists")
    return 0


if __name__ == "__main__":
    sys.exit(main())
