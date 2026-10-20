#!/usr/bin/env python3
"""Assert that the split specification is sound.

Three things are checked, and each of them is a way the split can be wrong
without anybody noticing:

  1. **Every file's heading number matches its filename.** A file called
     `12-taste-themes-moods-channels.md` whose heading says `# 11.` is a
     citation trap: everything that points at §12 lands on §11's text, and
     nothing fails.
  2. **The split is in step with the master.** The split is a copy, and a copy
     that is done by hand is a copy that is done once. `scripts/split-spec.py`
     re-derives it; this asserts the result is current.
  3. **The number space is exactly what it claims to be** — the slots the plan
     names, and nothing else.

Exits 0 when the split is sound, and 1 with the offending files named when it
is not. **It is a gate, not a report.**
"""

from __future__ import annotations

import pathlib
import re
import subprocess
import sys

TARGET = pathlib.Path("docs/spec")

#: The slots the plan names: 00 is the index, 01-27 hold the specification's
#: sections, and 28-40 are reserved and unallocated.
EXPECTED = {0} | set(range(1, 41))


def main() -> int:
    failures: list[str] = []

    if not TARGET.is_dir():
        print(f"{TARGET} does not exist", file=sys.stderr)
        return 1

    files = sorted(path for path in TARGET.iterdir() if path.is_file())
    numbers: dict[int, list[pathlib.Path]] = {}

    for path in files:
        match = re.match(r"^(\d{2})-", path.name)
        if not match:
            failures.append(f"{path.name} does not start with a two-digit slot number")
            continue
        slot = int(match.group(1))
        numbers.setdefault(slot, []).append(path)

        text = path.read_text(encoding="utf-8")
        heading = re.search(r"^# (\d+)\.", text, re.M)
        if not heading:
            failures.append(f"{path.name} has no top-level heading")
            continue
        if int(heading.group(1)) != slot:
            failures.append(
                f"{path.name} says '## {heading.group(1)}.' — the filename and the "
                f"heading must carry the same number"
            )

    for slot, paths in sorted(numbers.items()):
        if len(paths) > 1:
            names = ", ".join(path.name for path in paths)
            failures.append(f"slot {slot:02d} has more than one file: {names}")

    missing = sorted(EXPECTED - set(numbers))
    if missing:
        failures.append(
            "the reserved number space is incomplete; missing: "
            + ", ".join(f"{slot:02d}" for slot in missing)
        )
    extra = sorted(set(numbers) - EXPECTED)
    if extra:
        failures.append(
            "the split holds slots the number space does not reserve: "
            + ", ".join(f"{slot:02d}" for slot in extra)
        )

    # The split is a copy of the master, and a stale copy is a divergence.
    check = subprocess.run(
        [sys.executable, "scripts/split-spec.py", "--check"],
        capture_output=True,
        text=True,
    )
    if check.returncode != 0:
        failures.append("the split is not in step with the master:")
        for line in check.stderr.strip().splitlines():
            failures.append(f"  {line}")

    if failures:
        print("the split specification is not sound:", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1

    print(f"the split specification is sound: {len(files)} files, slots 00-40")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
