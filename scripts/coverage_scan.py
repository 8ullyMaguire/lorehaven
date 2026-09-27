#!/usr/bin/env python3
"""Coverage scan for `pub async fn` in crates/db/src, as reached from the
integration suites in crates/app/tests.

Answers: which data-layer functions does no test touch? The number that
matters when deciding what to cover next, and the one to re-derive rather
than extrapolate from an earlier run.

Two import shapes have to be recognised, and missing the second one silently
under-counts:

    use lorehaven_db::notifications;              // module name
    use lorehaven_db::roles as vanguard;          // module alias
    use lorehaven_db::notifications::{list, mark_read};   // grouped

A grouped import brings *bare* function names into scope, so those names count
as references too. Omitting that case made this report 348/831 when the true
figure was 358 -- the module it missed had just been finished.

A third shape is a call from *within* crates/db/src, reaching a module through
another module's public surface:

    crates/db/src/lib.rs:  migrate::apply(self).await

A function reached only that way reads as uncovered here, which is wrong rather
than merely pessimistic: `migrate` is the one module that is never worth testing
directly, because every `TestDb::connect` in every suite runs it. Crediting
intra-crate calls keeps the number honest in the other direction too -- a module
that is genuinely dead is still genuinely dead, because nothing in the crate
calls it either.

    python3 scripts/coverage_scan.py

    python3 scripts/coverage_scan.py
"""

import glob
import os
import re
import sys

BASE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
TESTS = os.path.join(BASE, "crates/app/tests/*.rs")
DB_SRC = os.path.join(BASE, "crates/db/src/*.rs")

# A path prefix in a `use` line: `lorehaven_db::`, a nested module path, or the
# test-support crate.
BR = r"(?:lorehaven_db::|lorehaven_db::[a-z_]+::|test_support::)"


def module_aliases():
    """Map every local name a test file binds to the db module it came from."""
    aliases = {}
    for path in glob.glob(TESTS):
        src = open(path).read()
        for m in re.finditer(r"use " + BR + r"(\w+)(?:\s+as\s+(\w+))?\s*;", src):
            aliases[m.group(2) or m.group(1)] = m.group(1)
        for m in re.finditer(r"use " + BR + r"(\w+)::\{([^}]*)\}", src):
            for item in m.group(2).split(","):
                item = item.strip()
                if not item:
                    continue
                if " as " in item:
                    aliases[item.split(" as ")[1].strip()] = m.group(1)
                else:
                    aliases[item] = m.group(1)
    return aliases


def references(aliases):
    """Every (local_name, fn_name) pair a test can reach."""
    refs = set()
    for path in glob.glob(TESTS):
        src = open(path).read()
        for m in re.finditer(r"\b(\w+)::(\w+)\s*\(", src):
            refs.add((m.group(1), m.group(2)))
        for m in re.finditer(r"use " + BR + r"\w+::\{([^}]*)\}", src):
            for item in m.group(1).split(","):
                item = item.strip().split(" as ")[-1].strip()
                if item and re.fullmatch(r"\w+", item):
                    refs.add((item, item))
    # Calls from inside the crate itself. `migrate` is reached through
    # `Database::migrate` in lib.rs rather than from a test, and is run by every
    # harness that connects.
    for path in glob.glob(DB_SRC) + glob.glob(os.path.join(BASE, "crates/db/src/**/*.rs")):
        src = open(path).read()
        for m in re.finditer(r"\b(\w+)::(\w+)\s*\(", src):
            refs.add((m.group(1), m.group(2)))
    del aliases
    return refs


def main():
    aliases = module_aliases()
    refs = references(aliases)

    total = covered = 0
    zero = []
    for path in glob.glob(DB_SRC):
        name = os.path.basename(path)[:-3]
        if name == "lib":
            continue
        pub = set(re.findall(r"^pub async fn (\w+)", open(path).read(), re.M))
        if not pub:
            continue
        hit = {fn for alias, fn in refs if aliases.get(alias) == name or alias == name}
        hit &= pub
        total += len(pub)
        covered += len(hit)
        if not hit:
            zero.append((name, len(pub)))

    pct = 100 * covered // total if total else 0
    print(f"db pub async fns: {total} | covered: {covered} ({pct}%) | "
          f"uncovered: {total - covered}")
    print(f"modules with zero coverage: {len(zero)}")
    for name, count in sorted(zero, key=lambda x: -x[1]):
        print(f"   {name:26s} {count}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
