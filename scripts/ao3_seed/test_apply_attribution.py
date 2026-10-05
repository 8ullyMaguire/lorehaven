"""Tests for the attribution SQL generator.

The generated SQL is what actually writes to a live database, so these assert on
its TEXT: the idempotency clauses and the escaping are the parts that either
work or quietly corrupt a production table.
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from apply_attribution import build_statements, load_works, write_sql  # noqa: E402
from match_authors import AuthorIndex, match_works  # noqa: E402


def _results():
    index = AuthorIndex()
    index.add("5282288", "Absolute Divinity", "JustBukharin")
    index.add("36333757", "A Different Way Forward", "Slytherin7Piece")
    return match_works(
        [("w1", "Absolute Divinity"), ("w2", "A Different Way Forward"),
         ("w3", "Nothing Matches This")],
        index,
    )


def test_only_matched_works_produce_statements() -> None:
    creators, edges = build_statements(_results())

    assert len(creators) == 2          # JustBukharin, Slytherin7Piece
    assert len(edges) == 2             # w3 is unmatched and must be absent


def test_one_creator_is_reused_across_its_works() -> None:
    index = AuthorIndex()
    index.add("1", "Same Author Work A", "Blackmarch")
    index.add("2", "Same Author Work B", "Blackmarch")
    results = match_works(
        [("w1", "Same Author Work A"), ("w2", "Same Author Work B")], index
    )

    creators, edges = build_statements(results)

    assert len(creators) == 1, "one creator row, reused"
    assert len(edges) == 2, "but one edge per work"
    assert {e[2] for e in edges} == {creators[0]["id"]}


def test_creator_ids_are_deterministic_across_runs() -> None:
    """A random id cannot match its own conflict target, so it would duplicate."""
    first, _ = build_statements(_results())
    second, _ = build_statements(_results())

    assert [c["id"] for c in first] == [c["id"] for c in second]


def test_edge_ids_are_deterministic_per_work() -> None:
    _, first = build_statements(_results())
    _, second = build_statements(_results())

    assert first == second


def test_generated_sql_is_idempotent_and_transactional(tmp_path: Path) -> None:
    creators, edges = build_statements(_results())
    out = tmp_path / "attribution.sql"
    write_sql(creators, edges, str(out))
    sql = out.read_text()

    assert sql.startswith("--")
    assert "BEGIN;" in sql and sql.rstrip().endswith("COMMIT;")
    # Both statements need a conflict target or a rerun duplicates rows.
    assert sql.count("ON CONFLICT (source_key, source_creator_id)") == len(creators)
    assert sql.count("ON CONFLICT (work_id, creator_id, role)") == len(edges)
    assert "now()::text" in sql


def test_quotes_in_a_display_name_are_escaped(tmp_path: Path) -> None:
    """An apostrophe in a pseud must not break out of the SQL string."""
    creators = [{
        "id": "11111111-1111-1111-1111-111111111111",
        "display_name": "O'Brien (Ao3_User)",
        "source_creator_id": "O'Brien (Ao3_User)",
        "canonical_url": "https://archiveofourown.org/users/x/pseuds/y",
    }]
    out = tmp_path / "q.sql"
    write_sql(creators, [], str(out))
    sql = out.read_text()

    assert "O''Brien" in sql
    assert "'O'Brien" not in sql


def test_null_canonical_url_is_written_as_sql_null(tmp_path: Path) -> None:
    creators = [{
        "id": "22222222-2222-2222-2222-222222222222",
        "display_name": "Nameless",
        "source_creator_id": "Nameless",
        "canonical_url": None,
    }]
    out = tmp_path / "n.sql"
    write_sql(creators, [], str(out))

    assert "NULL" in out.read_text()


def test_load_works_skips_short_and_blank_rows(tmp_path: Path) -> None:
    """A row with no title column is unusable, so it is skipped, not faked.

    `csv.reader` yields ['uuid-2'] (one field) for a truncated row and [] for a
    blank line; neither can become a (work_id, title) pair, and inventing a
    title for them would attribute a work nobody identified.
    """
    src = tmp_path / "works.csv"
    src.write_text("uuid-1,Title One\nuuid-2\n\nuuid-3,Title Three\n")

    works = load_works(str(src))

    assert works == [("uuid-1", "Title One"), ("uuid-3", "Title Three")]


def test_load_works_keeps_an_explicit_null_title(tmp_path: Path) -> None:
    """A present-but-empty title is a real row: it must survive as None so it
    is counted as unmatched rather than silently dropped from the totals."""
    src = tmp_path / "works.csv"
    src.write_text("uuid-1,Title One\nuuid-2,\n")

    works = load_works(str(src))

    assert works == [("uuid-1", "Title One"), ("uuid-2", None)]