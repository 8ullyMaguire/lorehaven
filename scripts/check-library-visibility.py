#!/usr/bin/env python3
"""A route that reads a reader's library must take its account from the session.

## Why this exists

M45-53 ("import visibility default: public sees metadata and link, cached body
private until author claims") was the requirement flagged in the gaps review as
the biggest reputational and copyright risk on the project. Measuring it against
the tree found the privacy property is ALREADY enforced -- and enforced at the
query, not at the route:

    db::reader_body_copies::body_for(db, work_id, account_id)
      WHERE work_id = ?1 AND account_id = ?2 AND state = 'ready'

and `a_reader_cannot_read_another_readers_copy` in
`crates/app/tests/reader_body_copies.rs` settles one reader's copy with the text
`ALICE'S EXCLUSIVE TEXT`, has a second reader fetch the same work, and asserts
404 with the bytes absent from the body. That test predates the requirement.

So the property holds -- but it holds by CONVENTION. Every route that reads
`library_items` happens to pass `user.account_id.to_string()`. Nothing fails if
the next one takes an id out of a path parameter instead. That is the same
failure mode as the bug this project has shipped four times: `is_public` dropped
from a query, invisible on one engine; `profile_empty` unable to survive an empty
result; a nav link present but unreachable; a `slot_id` written by the server and
read by nobody. In every case the invariant was true until one edit.

This gate is what makes it true until an edit fails a build.

## What it checks

For every handler in `crates/app/src/routes/*.rs` that reaches `library_items`
or `reader_body_copies`, the account argument must come from the session:
`user.account_id`, `session.0.account_id`, or `RequireSession(...)` in scope.

## What it CANNOT check, and says so

A regex cannot decide where a value came from two frames up. If a handler takes
`account_id: String` as an extractor and the router binds it from the path, this
gate sees a bare identifier and reports it -- which is the safe direction, since
the report demands a human read rather than granting a pass.

Where it is genuinely unsure it says UNSURE rather than PASS. A gate that
silently passes what it cannot prove is worse than no gate, because it is
believed. Run with `--explain` to see every candidate and why it was allowed.

## Self-test

`python3 scripts/check-library-visibility.py --self-test` -- six cases: three
that must be caught (account from a path param, from a query param, from a
header) and three that must not (session-derived, operator-scoped with an
explicit note, and a store function taking no account at all).

The self-test is not decoration. A gate that has never gone red is a gate that
does not work, and this repo has shipped one of those: a hand-rolled
query-arm-symmetry checker that exited 0 in every state including the bug it was
written to find, and was deleted.
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path

ROUTES = Path("crates/app/src/routes")

# The tables whose rows are a reader's private data. Anything that reads these
# must be told whose reader it is acting for.
PRIVATE_TABLES = ("library_items", "reader_body_copies", "import_chapters")

# Where an account may legitimately come from.
SESSION_SOURCES = re.compile(
    r"(user|session|me|sess|actor)\s*(?:\.\s*0\s*)?\.\s*account_id"
    r"|user\.account_id"
    r"|session\.0\.account_id"
)

# How an account may ILLEGITIMATELY arrive: the caller, not the session, decides
# whose private rows to read.
#
# The extractor type is GENERIC. `path: Path<String>` says nothing about which
# segment it binds -- it could be a work id, and in the failing self-test case it
# was. Matching on the extractor therefore cannot work: two of the three "must be
# caught" cases were named `path` and `q`, with no word like "account" anywhere
# in the signature. (That was the first version's bug, and the self-test is what
# found it -- which is the entire reason it exists.)
#
# The signal that DOES work is the ARGUMENT handed to the private-table call:
# `list_library_items(db, &user.account_id...)` is fine, `list_library_items(db,
# &path)` is not. So the gate reads the call, not the signature.
# The function names whose SECOND argument is the account. Word boundaries
# matter: without them `list_library_items` matches the `library_items` alternative
# and the parse goes wrong from there. (That was the second bug the self-test
# found — it reported every "must be caught" case as merely `unsure`.)
# Which argument is the ACCOUNT, per function, as a 1-based position among the
# arguments AFTER the leading `db`. `None` means the function takes no account and
# cannot leak one this way.
#
# These positions were read out of the store's signatures rather than assumed. The
# first version hardcoded "argument 2" for everything and reported FOUR violations
# on correct code -- `source_for_work(db, work_id)` and `count_copies(db, work_id)`
# take a work id, and `request_copy(db, work_id, account_id, ...)` puts the account
# third. A gate that cries wolf is worse than no gate, because people learn to
# skip it.
ACCOUNT_ARG_POSITION = {
    "list_library_items": 1,   # (db, account_id, limit, after)
    "body_for": 2,              # (db, work_id, account_id)
    "request_copy": 2,          # (db, work_id, account_id, source_key, chapter_key, trust)
    "status_for": 2,            # (db, work_id, account_id)
    "copy_id_for": 2,           # (db, work_id, account_id)
    # No account: these are keyed by work or by copy id.
    "source_for_work": None,
    "chapter_key_for_work": None,
    "count_copies": None,
    "status_by_id": None,
    "settle_ready": None,       # (db, copy_id, plain_text, sanitized_html) — a worker
    "settle_failed": None,
    "settle_refused": None,
}
_CALL_RE = re.compile(
    r"\b(" + "|".join(re.escape(f) for f in ACCOUNT_ARG_POSITION) + r")\s*(?:<[^>]*>)?\s*\("
)


def call_arguments(body: str) -> list[tuple[str, list[str]]]:
    """(function_name, top-level arguments) for every private-table call in `body`.

    Scans for the name, then walks forward balancing parentheses rather than
    trusting a regex for the argument list — an unbalanced regex stops at the
    first `(` inside the arguments, which is how `s.db()` was being mistaken for
    the whole call.
    """
    out: list[tuple[str, list[str]]] = []
    for m in _CALL_RE.finditer(body):
        i = m.end()  # just past the opening paren
        depth = 1
        in_str = False
        while i < len(body) and depth > 0:
            ch = body[i]
            if in_str:
                if ch == "\\":
                    i += 2
                    continue
                if ch == '"':
                    in_str = False
            elif ch == '"':
                in_str = True
            elif ch == "(":
                depth += 1
            elif ch == ")":
                depth -= 1
            i += 1
        out.append((m.group(1), _split_args(body[m.end() : i - 1])))
    return out


# An extractor that could carry a caller-chosen id. Kept as a SECONDARY signal,
# because a route with one of these AND a private-table read deserves a look even
# if the argument happens to look session-derived.
CALLER_CHOSEN_EXTRACTOR = re.compile(
    r"\b(?:Path|Query)\s*<", re.IGNORECASE
)

# An operator-scoped read is allowed, but only when the handler says why in a
# comment -- so the exception is visible at the call site rather than buried in
# this file's allowlist.
OPERATOR_NOTE = re.compile(r"//\s*(operator|audit|admin)[^\n]*", re.IGNORECASE)


# An extractor or response type marks a function as an HTTP handler in this
# codebase. Anything else is a helper or a worker, and the gate does not judge it:
# a helper's account parameter is a design choice its callers own, and a worker
# has no session to read.
HANDLER_MARKERS = (
    "State<",
    "RequireSession",
    "MaybeSession",
    "Path<",
    "Path(",
    "Query<",
    "Query(",
    "Json<",
    "Json(",
    "HeaderMap",
    "Extension<",
    "Form<",
    "Multipart",
    "-> Response",
    "Response {",
    "IntoResponse",
)


def looks_like_handler(seg: str) -> tuple[bool, str]:
    """Is `seg` an HTTP handler? Returns (verdict, the marker that decided it)."""
    head = seg[: seg.find("{") if "{" in seg else len(seg)]
    for marker in HANDLER_MARKERS:
        if marker in head:
            return True, marker
    return False, ""


@dataclass
class Finding:
    path: Path
    line: int
    kind: str  # "unsafe" | "unsure"
    detail: str


def _top_level_fn_spans(text: str) -> list[tuple[int, int, int]]:
    """(line_no, fn_start_offset, end_offset) for every fn at brace depth zero.

    Splitting on `^fn ` alone was the third bug the self-test found: a handler
    nested inside another fn -- which is exactly what the self-test's fixture is,
    since it wraps the handler in a `router()` to make it parse -- lands wholly
    inside its parent's "handler", and then the parent's own name is what gets
    reported. Real route modules have no nesting, so the gate was quietly
    reporting the wrong function on the one input designed to catch it.

    Depth is tracked over braces, strings and comments, because a `}` inside a
    string literal would otherwise close a function early.
    """
    spans: list[tuple[int, int, int]] = []
    i = 0
    depth = 0
    n = len(text)
    fn_re = re.compile(r"(?:pub\s+)?(?:async\s+)?fn\s+\w+")
    pending: tuple[int, int] | None = None  # (line_no, offset of `fn`)
    while i < n:
        ch = text[i]
        if ch == "/" and i + 1 < n and text[i + 1] == "/":
            j = text.find("\n", i)
            i = n if j < 0 else j
            continue
        if ch == "/" and i + 1 < n and text[i + 1] == "*":
            j = text.find("*/", i + 2)
            i = n if j < 0 else j + 2
            continue
        if ch == '"':
            i += 1
            while i < n and text[i] != '"':
                i += 2 if text[i] == "\\" else 1
            i += 1
            continue
        if ch == "'" and i + 2 < n and text[i + 1] == "\\":
            i += 3  # a char literal like '\n'
            continue
        if depth == 0 and ch in "f":
            m = fn_re.match(text, i)
            if m:
                line_no = text.count("\n", 0, i) + 1
                pending = (line_no, i)  # i is the offset of `fn`
                i = m.end()
                continue
        if ch == "{":
            if depth == 0 and pending is not None:
                depth = 1
                i += 1
                continue
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0 and pending is not None:
                spans.append((pending[0], pending[1], i))
                pending = None
        i += 1
    if pending is not None:
        spans.append((pending[0], pending[1], len(text)))
    return spans


def handlers_reaching_private_tables(text: str) -> list[tuple[int, str]]:
    """(line_no, 'fn name') for every top-level fn that touches a private table."""
    out: list[tuple[int, str]] = []
    for line_no, fn_start, end in _top_level_fn_spans(text):
        # The span's own start — not a search anchored at its end, which would only
        # ever see the closing line.
        seg = text[fn_start:end]
        if not any(t in seg for t in PRIVATE_TABLES):
            continue
        name = re.match(r"(?:async\s+)?fn (\w+)", seg)
        out.append((line_no, f"fn {name.group(1) if name else '<unknown>'}"))
    return out


def check_file(path: Path) -> list[Finding]:
    text = path.read_text(encoding="utf-8")
    findings: list[Finding] = []

    for line_no, label in handlers_reaching_private_tables(text):
        body = _body_from(text, label)
        if body is None:
            continue

        # An operator-scoped read is allowed when the handler says why in a
        # comment -- so the exception is visible at the call site rather than
        # buried in this file's allowlist.
        is_handler, marker = looks_like_handler(body)
        if not is_handler:
            # Not an HTTP handler: a private helper or a job worker. Its account
            # parameter is legitimate -- `find_duplicate(state, account_id, work)`
            # is called with `user.account_id` -- and a worker has no session to
            # read. Judging these is how the first version produced four false
            # violations on correct code.
            continue

        has_note = bool(OPERATOR_NOTE.search(body))

        session_derived = bool(SESSION_SOURCES.search(body))
        bad_arg = None
        calls = call_arguments(body)
        for name, parts in calls:
            # `call_arguments` has already split at top-level commas. Splitting a
            # second time tore any argument containing a comma into two, so the
            # account slot was a fragment rather than the account -- and every
            # "must be caught" case came back as merely `unsure`. That was the
            # fifth bug the self-test caught.
            position = ACCOUNT_ARG_POSITION.get(name)
            if position is None:
                # This function takes no account. It cannot leak a reader's rows by
                # being handed the wrong one -- but it also is not a read a
                # session gates, so it is not evidence of anything either way.
                continue
            if len(parts) <= position:
                continue
            candidate = parts[position]
            if not candidate:
                continue
            if SESSION_SOURCES.search(candidate):
                continue
            # A literal, a constant, or a session-derived expression is fine.
            if re.fullmatch(r"(None|Some\(\w+\)|&?\w+\.to_string\(\))", candidate):
                continue
            if has_note:
                # Downgraded, not allowed. A comment claiming operator authority is
                # a CLAIM, and the gate's job is to make a human confirm it — so
                # this reports `unsure` and never a silent pass.
                findings.append(
                    Finding(
                        path,
                        line_no,
                        "unsure",
                        f"{label}: reads another reader's rows via `{candidate}` and carries an "
                        f"operator/audit note — confirm the read is authorised and audited",
                    )
                )
                break
            bad_arg = candidate
            break

        if bad_arg is None:
            if not session_derived and calls and has_note:
                continue
            if not session_derived and calls:
                findings.append(
                    Finding(
                        path,
                        line_no,
                        "unsure",
                        f"{label}: reads a private table and no session-derived account was "
                        f"recognised. Confirm by reading it.",
                    )
                )
            continue

        findings.append(
            Finding(
                path,
                line_no,
                "unsafe",
                f"{label}: passes `{bad_arg}` as the account for a private-table read, and it "
                f"does not come from the session. A reader's rows must be selected by their own "
                f"session, never by a value the caller supplied.",
            )
        )

    return findings


def _split_args(text: str) -> list[str]:
    """Split an argument list on commas that are not inside brackets or strings."""
    out, depth, current, in_str = [], 0, [], False
    i = 0
    while i < len(text):
        ch = text[i]
        if in_str:
            if ch == "\\":
                current.append(text[i : i + 2])
                i += 2
                continue
            if ch == '"':
                in_str = False
            current.append(ch)
        elif ch == '"':
            in_str = True
            current.append(ch)
        elif ch in "(<[":
            depth += 1
            current.append(ch)
        elif ch in ")>]":
            depth -= 1
            current.append(ch)
        elif ch == "," and depth == 0:
            out.append("".join(current))
            current = []
        else:
            current.append(ch)
        i += 1
    if current:
        out.append("".join(current))
    return out


def _body_from(text: str, label: str) -> str | None:
    """The text of the handler named `label`, or None if it cannot be located.

    Boundaries come from `_top_level_fn_spans`, so a handler is never truncated
    and never swallows its neighbour.
    """
    want = label.removeprefix("fn ")
    for _line_no, fn_start, end in _top_level_fn_spans(text):
        m = re.match(r"(?:async\s+)?fn (\w+)", text[fn_start:end])
        if not (m and m.group(1) == want):
            continue
        # Include the doc comment ABOVE the handler. A justification for an
        # operator-scoped read belongs in the doc comment, which is where every
        # other such note in this codebase lives -- so a body that starts at `fn`
        # cannot see it. That was the sixth bug the self-test caught.
        head = _leading_doc_comment(text, fn_start)
        return head + text[fn_start:end]
    return None


def _leading_doc_comment(text: str, fn_start: int) -> str:
    """The `///` block immediately above `fn_start`, or ''."""
    lines: list[str] = []
    pos = text.rfind("\n", 0, fn_start) + 1
    while pos > 0:
        prev_end = pos - 1
        prev_start = text.rfind("\n", 0, prev_end) + 1
        line = text[prev_start:prev_end].strip()
        if line.startswith("///") or line.startswith("//!"):
            lines.append(line)
            pos = prev_start
            continue
        break
    return ("\n".join(reversed(lines)) + "\n") if lines else ""


# ─────────────────────────────────────────────────────────────────────────────
# Self-test
# ─────────────────────────────────────────────────────────────────────────────

CASES: list[tuple[str, str, str | None]] = [
    (
        "caught: account from a path parameter",
        """
        async fn leak(path: Path<String>, State(s): State<AppState>) -> ApiResult<Json<Value>> {
            let items = library::list_library_items(s.db(), &path, 50, None).await?;
            Ok(Json(json!(items)))
        }
        """,
        "unsafe",
    ),
    (
        "caught: account from a query parameter",
        """
        async fn leak(Query(q): Query<Q>, State(s): State<AppState>) -> ApiResult<Json<Value>> {
            let items = library::list_library_items(s.db(), &q.account_id, 50, None).await?;
            Ok(Json(json!(items)))
        }
        """,
        "unsafe",
    ),
    (
        "caught: account from a header",
        """
        async fn leak(headers: HeaderMap, State(s): State<AppState>) -> ApiResult<Json<Value>> {
            let acct = headers.get("x-account").unwrap().to_str().unwrap().to_string();
            let rows = library::list_library_items(s.db(), &acct, 50, None).await?;
            Ok(Json(json!(rows)))
        }
        """,
        "unsafe",
    ),
    (
        "allowed: account from the session",
        """
        async fn mine(RequireSession(user): RequireSession, State(s): State<AppState>) -> ApiResult<Json<Value>> {
            let rows = library::list_library_items(s.db(), &user.account_id.to_string(), 50, None).await?;
            Ok(Json(json!(rows)))
        }
        """,
        None,
    ),
    (
        "allowed: operator-scoped, with an explicit note",
        """
        /// AUDIT: operator reads, recorded in audit_log by the caller.
        async fn audit(RequireSession(user): RequireSession, State(s): State<AppState>) -> ApiResult<Json<Value>> {
            let rows = library::list_library_items(s.db(), &subject, 50, None).await?;
            Ok(Json(json!(rows)))
        }
        """,
        "unsure",
    ),
    (
        "allowed: a store function taking no account at all",
        """
        async fn count(State(s): State<AppState>) -> ApiResult<Json<Value>> {
            let n = library::total_library_items(s.db()).await?;
            Ok(Json(json!({ "n": n })))
        }
        """,
        None,
    ),
]


def self_test() -> int:
    import tempfile

    failures = 0
    for name, body, expect in CASES:
        with tempfile.NamedTemporaryFile("w", suffix=".rs", delete=False) as fh:
            # A handler at the top level, as in every real route module. The first
            # version wrapped each case in `fn router() { ... }`, which nested the
            # handler one brace deep -- unlike anything in the tree, and it made the
            # parser report the WRONG function (the wrapper) on the one input
            # designed to catch exactly that. A fixture that does not resemble the
            # code under test tests nothing.
            fh.write("use axum::routing::get;\nuse axum::extract::Path;\n")
            fh.write(body)
            path = Path(fh.name)
        try:
            findings = check_file(path)
            kinds = {f.kind for f in findings}
            if expect is None:
                ok = "unsafe" not in kinds
                detail = f"expected no unsafe finding, got {sorted(kinds) or 'none'}"
            else:
                ok = expect in kinds
                detail = f"expected a {expect!r} finding, got {sorted(kinds) or 'none'}"
            status = "pass" if ok else "FAIL"
            if not ok:
                failures += 1
            print(f"  [{status}] {name} — {detail}")
        finally:
            path.unlink(missing_ok=True)

    print(f"\n{len(CASES) - failures}/{len(CASES)} self-test cases passed")
    return 1 if failures else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--self-test", action="store_true", help="run the six-case self-test")
    ap.add_argument("--explain", action="store_true", help="print every candidate, not just failures")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    if not ROUTES.is_dir():
        print(f"error: {ROUTES} not found — run from the repository root", file=sys.stderr)
        return 2

    all_findings: list[Finding] = []
    scanned = 0
    for path in sorted(ROUTES.glob("*.rs")):
        scanned += 1
        all_findings.extend(check_file(path))

    unsafe = [f for f in all_findings if f.kind == "unsafe"]
    unsure = [f for f in all_findings if f.kind == "unsure"]

    if args.explain:
        for f in all_findings:
            print(f"{f.path}:{f.line}  [{f.kind}]  {f.detail}")

    for f in unsafe:
        print(f"VIOLATION {f.path}:{f.line}  {f.detail}", file=sys.stderr)

    print(f"\nscanned {scanned} route modules")
    print(f"  unsafe: {len(unsafe)}")
    print(f"  unsure: {len(unsure)}  (each needs a human read; a regex cannot trace an identifier)")

    if unsafe:
        print("\nFAILED: a reader's private rows can be reached without the session.", file=sys.stderr)
        return 1

    print("\nOK: every route reaching a private table takes its account from the session.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
