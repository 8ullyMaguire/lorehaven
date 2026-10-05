"""Tests for the AO3 author matcher.

Each test here guards a MEASURED trap from the 2026-10-05 attribution run, not
a hypothetical. If one fails, the corresponding claim in the plan's risk section
is no longer true and the backfill would corrupt attribution.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from match_authors import AuthorIndex, match_works, normalise_title  # noqa: E402


def test_normalise_collapses_case_and_punctuation() -> None:
    assert normalise_title("Bench Warmer (A Naruto SI)") == normalise_title(
        "bench warmer a naruto si"
    )
    assert normalise_title("A  Work") == normalise_title("A-Work")
    assert normalise_title(None) == ""
    assert normalise_title("") == ""


def test_a_unique_title_resolves_to_its_author() -> None:
    index = AuthorIndex()
    index.add("5282288", "Absolute Divinity", "JustBukharin")
    results = match_works([("w1", "Absolute Divinity")], index)

    assert len(results) == 1
    result = results[0]
    assert result.attributed
    assert result.author == "JustBukharin"
    assert result.ao3_work_id == "5282288"
    assert result.ao3_work_url == "https://archiveofourown.org/works/5282288"
    # The author link must be the PROFILE, never the work URL.
    assert result.ao3_author_url == (
        "https://archiveofourown.org/users/JustBukharin/pseudos/JustBukharin"
    )
    assert result.reason == "unique title match"


def test_an_ambiguous_title_is_never_guessed() -> None:
    """`This Fire Burns` is 4+ distinct AO3 works. Guessing invents an author."""
    index = AuthorIndex()
    index.add("1", "This Fire Burns", "Killware")
    index.add("2", "This Fire Burns", "SomeoneElse")

    results = match_works([("w1", "This Fire Burns")], index)

    assert not results[0].attributed
    assert results[0].author is None
    assert results[0].reason == "ambiguous title"


def test_an_unmatched_title_is_left_unattributed() -> None:
    index = AuthorIndex()
    index.add("1", "Something Else Entirely", "Author")

    result = match_works([("w9", "Omakes")], index)[0]

    assert not result.attributed
    assert result.reason == "no AO3 title match"
    # Leaving it null is the point: curators can fix it, a wrong author cannot.
    assert result.ao3_author_url is None


def test_a_row_with_no_author_never_enters_the_index() -> None:
    index = AuthorIndex()
    index.add("1", "Anonymous Work", "Anonymous")
    index.add("2", "Anonymous Work", None)

    # 'Anonymous' is a real AO3 byline in 1,658 of 40k sampled rows, so a work
    # with it must not be treated as an identified author.
    assert len(index) == 1


def test_pseudonym_shape_splits_into_user_and_pseud() -> None:
    """AO3 stores `Display (pseudonym)`; the profile URL needs both parts."""
    index = AuthorIndex()
    index.add("7", "Some Fic", "Display Name (pseudonym)")

    result = match_works([("w1", "Some Fic")], index)[0]

    assert result.author == "Display Name (pseudonym)"
    # The space must be percent-encoded or the profile URL is broken.
    assert result.ao3_author_url == (
        "https://archiveofourown.org/users/Display%20Name/pseudos/pseudonym"
    )


@pytest.mark.parametrize("bad", [None, "", "   "])
def test_blank_titles_are_skipped(bad: str | None) -> None:
    index = AuthorIndex()
    index.add("1", bad, "Author")
    assert len(index) == 0


def test_matching_never_raises_on_a_malformed_row() -> None:
    index = AuthorIndex()
    index.add("1", "Real Title", "Author")

    results = match_works([("w1", None), ("w2", "Real Title")], index)

    assert [r.attributed for r in results] == [False, True]