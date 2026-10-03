#!/usr/bin/env python3
"""§55.6's gate: no WASM runtime is adopted while the sandbox is unwritten.

Spec §55 specifies two adapter paths — a declarative manifest (Path A) and a WASM
module (Path B) — and ships only the first. `wasmi` is named in §21.4 as the
intended runtime and is deliberately absent from this workspace, because Path B's
security model is entirely the domain lockdown of §55.4.1 and that
implementation does not exist.

**This check exists because `crates/app/tests/wasm_gate.rs` was not enough.**
That test runs after `cargo build`, and the first attempt to prove it fired added
`wasmi = "0.4"`, which fails to compile on this workspace's toolchain (1.78:
`panic` is ambiguous between a glob import and `#[macro_use]` in wasmi 0.4.5).
So the build died before the test could report anything — a gate that only fires
after the thing it gates has already broken the build is a gate that reports the
wrong failure. A static check runs first and says what actually happened.

It is also worth knowing that wasmi 0.4 does not build here at all: adopting
Path B is not a one-line dependency change on this toolchain.

Run with `--self-test` to check the checker's own rules against known inputs,
matching the other gates in this directory.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

# A WASM runtime, by crate name. Matched as a dependency key rather than a
# substring anywhere in the file: a prose mention in a comment, or the word
# "wasm" in a target triple, is not an adoption.
RUNTIME_CRATES = ("wasmi", "wasmtime", "wasm-bindgen", "waseo-bindgen")

# The manifests a dependency would be added to. Listed explicitly rather than
# globbing: a glob reads target/ and the vendored registry, and a gate that is
# slow or noisy gets ignored.
MANIFESTS = (
    "Cargo.toml",
    "crates/app/Cargo.toml",
    "crates/domain/Cargo.toml",
    "crates/scrapers/Cargo.toml",
    "crates/db/Cargo.toml",
    "crates/decisions/Cargo.toml",
)

# A dependency line: `wasmi = "0.4"`, `wasmi = { workspace = true }`,
# `wasmi.workspace = true`. The crate name is followed by `=`, optionally with
# whitespace — and by an optional `.something` first, because the workspace
# inheritance form writes `wasmi.workspace = true` and a regex without the dot
# matched neither it nor `serde.workspace`. The self-test caught that: it
# asserted `wasmi.workspace` was found and it was not.
DEP_LINE = re.compile(r"^\s*([A-Za-z0-9_-]+)\s*(?:\.[A-Za-z0-9_-]+\s*)*=", re.MULTILINE)


def find_adoptions(text: str) -> list[tuple[int, str]]:
    """Dependency lines naming a WASM runtime, as (line number, crate)."""
    hits = []
    for match in DEP_LINE.finditer(text):
        name = match.group(1)
        if name in RUNTIME_CRATES:
            line = text.count("\n", 0, match.start()) + 1
            hits.append((line, name))
    return hits


def check(manifests: list[Path], root: Path) -> list[str]:
    problems = []
    for manifest in manifests:
        path = root / manifest
        if not path.exists():
            problems.append(
                f"{manifest}: not found. The gate cannot check a manifest it cannot "
                f"read, and a gate that reads nothing must not report success."
            )
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except OSError as exc:
            problems.append(f"{manifest}: cannot read: {exc}")
            continue

        for line, name in find_adoptions(text):
            problems.append(
                f"{manifest}:{line}: adopts `{name}`, but §55.6's gate is unmet — the "
                f"sandbox does not exist and §55.4.1's domain lockdown has not been "
                f"written and attacked. Build the sandbox and its private-address "
                f"refusal test first, or amend §55.6 and say why."
            )
    return problems


def self_test() -> int:
    """The checker's own rules, on statements whose correct answer is known."""
    cases: list[tuple[str, str, list[str]]] = [
        # (label, manifest text, expected crate names that must be found)
        ("plain dependency", 'wasmi = "0.4"', ["wasmi"]),
        ("workspace inheritance", "wasmi.workspace = true", ["wasmi"]),
        ("inline table", "wasmtime = { version = '30', features = ['cranelift'] }", ["wasmtime"]),
        ("two runtimes", 'wasmtime = "30"\nwasmi = "0.4"', ["wasmtime", "wasmi"]),
        ("indented", "[dependencies]\n    wasmi = \"0.4\"", ["wasmi"]),
    ]
    negatives = [
        ("prose mention", "# we should adopt wasmi eventually\nscraper = \"0.27\""),
        ("the word in a name", "wasm-bindgen-test = \"0.3\""),
        ("no wasm at all", 'serde = { version = "1", features = ["derive"] }'),
        ("a url mentioning it", 'note = "see https://crates.io/crates/wasmi"'),
    ]

    failures = []
    for label, text, expected in cases:
        found = [name for _, name in find_adoptions(text)]
        if sorted(found) != sorted(expected):
            failures.append(f"  {label}: expected {expected}, found {found}")

    for label, text in negatives:
        found = [name for _, name in find_adoptions(text)]
        if found:
            failures.append(f"  {label}: expected no adoption, found {found}")

    if failures:
        print("wasm-gate self-test FAILED:", file=sys.stderr)
        print("\n".join(failures), file=sys.stderr)
        return 1

    total = len(cases) + len(negatives)
    print(f"wasm-gate self-test passed ({total} cases, {len(negatives)} negative)")
    return 0


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()

    root = Path(__file__).resolve().parent.parent
    problems = check([Path(m) for m in MANIFESTS], root)
    if problems:
        print("§55.6 gate: a WASM runtime is adopted while the gate is unmet.", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        return 1

    print(f"§55.6 gate: no WASM runtime adopted ({len(MANIFESTS)} manifests checked)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
