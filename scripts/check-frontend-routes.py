#!/usr/bin/env python3
"""Find a frontend call to a door the API does not have.

## Why this gate exists

The rename of `chapters` to `units` (M4-02) changed the **model** and kept the
**URL**: the API's doors are still `/api/v1/works/{id}/chapters` and
`/api/v1/chapters/{id}`, because a URL is the most stable public surface an
instance has.

The frontend was renamed too — and the rename over-reached into the **API
paths**. `frontend/src/lib/api.ts` came to call `/works/{id}/units` and
`/units/{id}`, and the API had no such doors: **every one of them was a 404**.

Nothing caught it, and the reasons are the point:

* the frontend's unit tests **mock the API**, so they are green against a
  door that does not exist;
* the Rust tests call the API directly, so they never see the frontend's
  spelling;
* **the e2e suites are not part of the gate** — `gate.sh e2e` needs a suite
  name, so it is run by hand.

So the frontend and the API drifted apart, and the only thing that noticed was
a person running the e2e suite. This gate is that notice, made automatic.

## What it compares

The **route table** (`crates/app/src/routes/inventory.rs`) against the paths
the frontend builds.

The route table and not the registered routes, because the table is already
verified against the registrations by `route_inventory.rs` -- so the table is
the *declared* set, and this gate asks the question the table can answer: does
the frontend call anything the declaration does not have?

The paths are compared by **shape**: a parameter becomes a wildcard, so
`/chapters/{id}` and `/chapters/${id}` are the same door. A path with a query
string is compared without it.

## What it does not do

* It does not check that the frontend calls every door. A door nobody calls
  is a door nobody needs, and saying so is a different question.
* It does not check the HTTP method. The route table carries the method, but
  a frontend call's method is often a parameter of the calling function, and
  guessing it would produce findings nobody could act on.
* It does not read the route table's *handlers*. Only the paths matter here.

Exit 0 when clean, 1 with the findings otherwise, so it works as a CI gate.
"""

from __future__ import annotations

import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent

# The route table's entries: `RouteEntry { ... path: "/some/path", ... }`.
ROUTE_ENTRY = re.compile(
    r"RouteEntry\s*\{[^}]*?path:\s*\"([^\"]+)\"", re.DOTALL
)

# A frontend call: `apiFetch<...>(`...`)` or `fetch(`...`)`. The generic
# parameter and the call are separated so the path is the last thing before the
# opening bracket.
API_CALL = re.compile(
    r"(?:apiFetch|fetch)\s*(?:<[^>]*>)?\s*\(\s*[`'\"]([^`'\"]*)",
    re.DOTALL,
)
# The same, for a template literal, which may carry substitutions.
TEMPLATE_CALL = re.compile(
    r"(?:apiFetch|fetch)\s*(?:<[^>]*>)?\s*\(\s*`",
    re.DOTALL,
)

# The API is nested under `/api/v1`, and the route table's paths are relative
# to it. A few frontend calls spell the whole path out, so the prefix is
# stripped before the comparison.
API_PREFIX = "/api/v1"


def template_literal(text: str, start: int) -> tuple[str, int] | None:
    """The template literal that begins at `start`, and where it ends.

    **A template literal can contain another one**, and a nested one is
    written with backticks: `` `/admin/jobs${suffix ? `?page=${page}` : ''}` ``.
    A regex that stops at the first backtick reads the *prefix* of the path
    and reports a path that is not a path.
    """
    # The scan starts AFTER the opening backtick: the opening one is not the
    # closing one, and a scanner that starts on it reads an empty literal.
    index = start + 1
    depth = 0
    while index < len(text):
        ch = text[index]
        if text.startswith("${", index):
            depth += 1
            index += 2
            continue
        if ch == "}" and depth:
            depth -= 1
            index += 1
            continue
        if ch == "`" and depth:
            # A nested template: its own braces are counted, and it ends at
            # the next backtick that is not inside one of its own
            # substitutions.
            index += 1
            inner = 0
            while index < len(text):
                if text.startswith("${", index):
                    inner += 1
                    index += 2
                    continue
                if text[index] == "}" and inner:
                    inner -= 1
                    index += 1
                    continue
                if text[index] == "`" and not inner:
                    break
                index += 1
            # Past the nested template's closing backtick: the outer template
            # continues after it, and scanning from the backtick itself would
            # read the nested template as a second nested one.
            index += 1
            continue
        if ch == "`" and not depth:
            return text[start + 1 : index], index + 1
        index += 1
    return None


def substitutions(path: str) -> list[tuple[int, int, str]]:
    """Every `${...}` in a template literal, as `(start, end, body)`.

    **The braces are counted, not matched with a regular expression.** A
    substitution's body can contain a nested template -- `${suffix ? `a` :
    `b`}` -- and a `[^}]*` pattern stops at the *inner* brace and reports a
    path that is not the path.
    """
    out: list[tuple[int, int, str]] = []
    index = 0
    while index < len(path):
        if path.startswith("${", index):
            depth = 1
            cursor = index + 2
            while cursor < len(path) and depth:
                if path[cursor] == "{":
                    depth += 1
                elif path[cursor] == "}":
                    depth -= 1
                cursor += 1
            if depth == 0:
                out.append((index, cursor, path[index + 2 : cursor - 1]))
                index = cursor
                continue
        index += 1
    return out


def normalise(path: str) -> str:
    """A path with every parameter written the same way.

    The API spells a parameter `{id}` and the frontend spells it `${id}`, and
    both are the same door. The `${...}` has to go **before** the `{...}`,
    because the second pattern would otherwise match the braces inside the
    first and leave a `$` behind.
    """
    path = re.sub(r"\$\{[^}]*\}", "{}", path)
    path = re.sub(r"\{[^}]*\}", "{}", path)
    return path.rstrip("/") or "/"


def shapes(path: str) -> list[str]:
    """The shapes a frontend path can stand for.

    Usually one. **Two** when the last segment is a choice between two
    literals -- `${accept ? 'accept' : 'decline'}` builds one of two doors,
    and the API has both, so the path is not a door that does not exist.

    `{id}` (the API's spelling) and `${id}` (the frontend's) are the same
    door, so both become `{}`.

    **A trailing substitution that does not follow a `/` is a query string.**
    `/discovery${query}` is the `/discovery` door with a query; `/chapters/${id}`
    is the `/chapters/{}` door. Getting this wrong reports a door that
    exists, and a gate that reports doors that exist is a gate nobody reads.
    """
    # **The prefix is stripped FIRST**, because stripping it shifts every index
    # and the spans are found by index.
    if path.startswith(API_PREFIX):
        path = path[len(API_PREFIX) :]
        # The prefix ends in a `/` and the remainder starts with a word, so the
        # separator has to be put back.
        if not path.startswith("/"):
            path = "/" + path
        path = path or "/"

    spans = substitutions(path)

    # **The query string is split AFTER the substitutions are found**, because
    # a `?` inside a substitution is a ternary and not a query: the invitation
    # path builds `${accept ? 'accept' : 'decline'}` and splitting on its `?`
    # cuts the path in half.
    query_at = None
    for start, end, _body in spans:
        pass
    outside = []
    cursor = 0
    for start, end, _body in spans:
        outside.append(path[cursor:start])
        cursor = end
    outside.append(path[cursor:])
    head = "".join(outside)
    if "?" in head:
        query_at = head.index("?")
        path = path[: path.index("?")]
        spans = [
            (start, end, body)
            for start, end, body in spans
            if start < query_at
        ]

    # A trailing substitution that does not follow a `/` is a query string.
    if spans:
        start, end, body = spans[-1]
        if start > 0 and path[start - 1] != "/":
            path = path[:start]
            spans = spans[:-1]

    # A trailing choice between two literals is two doors.
    alternatives: list[str] = []
    if spans:
        start, end, body = spans[-1]
        choice = re.findall(r"'([^']*)'", body)
        if len(choice) == 2 and "?" in body:
            for literal in choice:
                alternative = path[:start] + literal
                alternatives.append(normalise(alternative))
    if alternatives:
        return alternatives

    for start, end, _body in reversed(spans):
        path = path[:start] + "{}" + path[end:]
    return [normalise(path)]


def route_table_shapes() -> set[str]:
    """The shapes of every path the route table declares."""
    source = (REPO / "crates/app/src/routes/inventory.rs").read_text(
        encoding="utf-8", errors="replace"
    )
    out: set[str] = set()
    for path in ROUTE_ENTRY.findall(source):
        out.update(shapes(path))
    return out


def frontend_paths() -> list[tuple[pathlib.Path, int, str]]:
    """Every path the frontend builds, with where it was built."""
    out: list[tuple[pathlib.Path, int, str]] = []
    root = REPO / "frontend/src"
    for path in sorted(list(root.rglob("*.ts")) + list(root.rglob("*.svelte"))):
        # A test mocks the API, so a test's paths are not the frontend's paths.
        if path.name.endswith(".test.ts") or path.name.endswith(".test.ts"):
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        # The template literals first, and the quoted pattern is asked not to
        # match inside one -- otherwise a template's *prefix* is read as a path
        # of its own, and `/chapters/${id}` is reported as `/chapters/`.
        template_spans: list[tuple[int, int]] = []
        for match in TEMPLATE_CALL.finditer(text):
            found = template_literal(text, match.end() - 1)
            if found is None:
                continue
            literal, end = found
            template_spans.append((match.start(), end))
            if not literal.startswith("/"):
                continue
            line = text[: match.start()].count("\n") + 1
            out.append((path, line, literal))
        for match in API_CALL.finditer(text):
            if any(start <= match.start() < end for start, end in template_spans):
                continue
            literal = match.group(1)
            if not literal.startswith("/"):
                continue
            line = text[: match.start()].count("\n") + 1
            out.append((path, line, literal))
    return out


def main() -> int:
    declared = route_table_shapes()
    if not declared:
        print("no routes found in the route table -- the table moved?", file=sys.stderr)
        return 2

    findings: list[tuple[pathlib.Path, int, str]] = []
    seen: set[tuple[str, str]] = set()
    for path, line, literal in frontend_paths():
        if any(candidate in declared for candidate in shapes(literal)):
            continue
        key = (str(path), literal)
        if key in seen:
            continue
        seen.add(key)
        findings.append((path, line, literal))

    if findings:
        print("frontend calls a door the route table does not have:")
        for path, line, literal in findings:
            try:
                shown = path.relative_to(REPO)
            except ValueError:
                shown = path
            print(f"  {shown}:{line}: {literal}")
        print(
            "\nA path here is a 404 waiting to happen. The route table is the\n"
            "declared set of doors; the frontend calls them."
        )
        return 1
    print(
        "OK: every door the frontend calls is in the route table "
        f"({len(declared)} doors)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
