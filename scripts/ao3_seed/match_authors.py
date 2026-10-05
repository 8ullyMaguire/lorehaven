"""Match lorehaven works to AO3 authors by normalised title.

The lorehaven corpus on thinkcentre was bulk-loaded 2026-09-20..25 with no
`creators`, no `published_at` and no `source_url`, so the only recoverable
identity is the title. Measured coverage on the live corpus is 12.5%
(7,954 of 63,737) -- see the plan's risk section.

TWO RULES THAT ARE MEASURED, NOT ASSUMED. Both are load-bearing:

1. Never combine `title` with `word_count`. lorehaven's `word_count` is the SUM
   over `chapter_revisions`; AO3's is the work TOTAL. Pairing them collapses
   7,954 matches to 2 -- pairing them is not a stricter key, it is a wrong one.

2. `word_count` alone is not identifying (6.1%), so it must never be the sole
   key: it would attach authors to works that merely happen to be similar in
   length.

Only UNIQUE title matches are attributed. 2,506 normalised titles map to several
distinct AO3 works -- `This Fire Burns` appears 4+ times as separate works --
and guessing among them would invent an author.

Unmatched works are deliberately left alone rather than guessed. The reader's
instruction (2026-10-05) is to fix what can be fixed correctly and leave the
rest for human curators: an unattributed work is recoverable, a wrongly
attributed one is a lie that outlives the import.
"""

from __future__ import annotations

import re
import sqlite3
from dataclasses import dataclass
from urllib.parse import quote

__all__ = [
    "normalise_title",
    "AuthorIndex",
    "MatchResult",
    "match_works",
    "AO3_WORK_URL",
]

AO3_WORK_URL = "https://archiveofourown.org/works/{work_id}"
AO3_AUTHOR_URL = "https://archiveofourown.org/users/{user}/pseudos/{pseud}"

# AO3 titles carry chapter suffixes and parenthetical qualifiers that survive
# differently in each corpus, so collapse all punctuation and case to compare.
_NON_WORD = re.compile(r"\W+")


def normalise_title(title: str | None) -> str:
    """Case- and punctuation-insensitive key for title matching.

    Whitespace runs collapse to one space so `A  Work` and `A-Work` agree.
    """
    return _NON_WORD.sub(" ", (title or "").lower()).strip()


@dataclass(frozen=True)
class MatchResult:
    """One work's attribution outcome. `author` is None when unresolved."""

    work_id: str
    title: str | None
    author: str | None
    ao3_work_id: str | None
    ao3_work_url: str | None
    ao3_author_url: str | None
    reason: str

    @property
    def attributed(self) -> bool:
        return self.author is not None


class AuthorIndex:
    """Title -> (ao3_work_id, author) for every AO3 work with a known author.

    A title is indexed ONLY when exactly one AO3 work carries it. Ambiguous
    titles are held aside in `ambiguous` so a caller can report them instead of
    guessing.
    """

    def __init__(self, ambiguous: dict[str, int] | None = None) -> None:
        self._by_title: dict[str, tuple[str, str]] = {}
        self.ambiguous: dict[str, int] = ambiguous or {}

    def add(self, ao3_work_id: str, title: str | None, author: str | None) -> None:
        if not author or not title:
            return
        key = normalise_title(title)
        if not key:
            return
        if key in self._by_title:
            self.ambiguous[key] = self.ambiguous.get(key, 1) + 1
            del self._by_title[key]
            return
        if key in self.ambiguous:
            self.ambiguous[key] += 1
            return
        self._by_title[key] = (str(ao3_work_id), author)

    def lookup(self, title: str | None) -> tuple[str, str] | None:
        return self._by_title.get(normalise_title(title))

    def __len__(self) -> int:
        return len(self._by_title)

    @classmethod
    def from_db(cls, db_path: str) -> AuthorIndex:
        """Build the index from the AO3 recommender metadata DB.

        Read-only, so it is safe to point at the 1.7 GB file directly. Only the
        three columns needed are selected; the `authors` TEXT column holds one
        name or `Display (pseudonym)`.

        `ambiguous` is deliberately KEPT. Clearing it made `match_works` report
        every failure as 'no AO3 title match' -- a dry-run then claimed 0
        ambiguous titles when the corpus really holds 4,841 of them, which hides
        exactly the cases a curator needs to see.
        """
        con = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
        try:
            index = cls()
            rows = con.execute(
                "SELECT id, title, authors FROM works "
                "WHERE authors IS NOT NULL AND authors <> ''"
            )
            for ao3_work_id, title, author in rows:
                index.add(str(ao3_work_id), title, author)
            return index
        finally:
            con.close()


def match_works(
    works: list[tuple[str, str | None]], index: AuthorIndex
) -> list[MatchResult]:
    """Match `(work_id, title)` pairs against the index.

    Titles are `str | None`: a row with a NULL title is a real input, and it
    must be reported as unmatched rather than raising. Never raises on a bad
    row, because an unmatched work is counted and the backfill continues.
    """
    out: list[MatchResult] = []
    for work_id, title in works:
        hit = index.lookup(title)
        if hit is None:
            key = normalise_title(title)
            reason = (
                "ambiguous title" if key in index.ambiguous else "no AO3 title match"
            )
            out.append(MatchResult(work_id, title, None, None, None, None, reason))
            continue
        ao3_work_id, author = hit
        pseud = author.split("(")[-1].rstrip(")").strip() or author
        user = author.split("(")[0].strip() or author
        # AO3 pseuds carry spaces, dots and apostrophes. Unencoded, the URL is
        # broken -- so quote the path segments, not the whole URL.
        author_url = AO3_AUTHOR_URL.format(
            user=quote(user, safe=""), pseud=quote(pseud, safe="")
        )
        out.append(
            MatchResult(
                work_id,
                title,
                author,
                ao3_work_id,
                AO3_WORK_URL.format(work_id=ao3_work_id),
                author_url,
                "unique title match",
            )
        )
    return out