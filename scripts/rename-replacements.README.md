# The rename's rules — `lorehaven` to Darkhold

`scripts/rename-replacements.txt` holds the six rules, and **this file explains them**,
because the rules file itself cannot.

## Why the explanation is not in the rules file

**`git filter-repo --replace-text` has no comment syntax.** A line in that file is a rule,
and a line with no `==>` is a rule that removes the text it contains. So a line that is
exactly `#` is a rule that removes every `#` from every blob in the repository.

That is not a hypothetical. **It happened**, during the verification of this milestone: a
rules file with a `#` comment block was written, the rewrite was re-run against the
pre-rename bundle to check it was reproducible, and the resulting tree had every comment in
the repository turned into `***REMOVED***`.

```toml
# A target directory of this project's own.        # A target directory of this project's own.
```
became
```toml
***REMOVED*** A target directory of this project's own.
```

**The tree was caught before it was committed**, because the verification compares the
re-run tree against the committed tree rather than against an expectation. The rules file
is now bare, and this file carries the explanation.

## The six rules

```
literal:LOREHAVEN==>DARKHOLD
literal:Lorehaven==>Darkhold
literal:lorehaven==>darkhold
literal:lore-metadata==>metadata
literal:darkhold_lore_metadata==>darkhold_metadata
regex:\blore_metadata\b==>darkhold_metadata
```

`filter-repo` applies them **in order**, each over the output of the one before it. The
order is the mechanism, not a style.

### The first four: the project's name

| rule | what it catches |
|---|---|
| `literal:LOREHAVEN==>DARKHOLD` | the environment prefix — `LOREHAVEN_TEST_PG_URL` |
| `literal:Lorehaven==>Darkhold` | the name in prose and in brand strings |
| `literal:lorehaven==>darkhold` | the name in identifiers, paths and crate names |
| `literal:lore-metadata==>metadata` | the crate directory, which is a *different* word from the project name |

**The fourth rule is only correct because of where it sits.** `lorehaven-lore-metadata`
becomes `darkhold-lore-metadata` under the third rule, and then `darkhold-metadata` under
the fourth. Run the fourth first, and the result would be `lorehaven-metadata` — a name that
is neither the old one nor the new one.

### The last two: the crate name, which only exists after the first four have run

```
before   package lorehaven-lore-metadata   lib lorehaven_lore_metadata
after    package darkhold-metadata         lib darkhold_metadata
```

`lorehaven` → `darkhold` alone gives `darkhold-lore-metadata` and `darkhold_lore_metadata`.
Neither is the name wanted, and **no rule that could run before the first three can produce
them**, because their source text does not exist yet. That is why the rewrite ran in two
passes when it was performed, and why the file lists the second pass's rules *after* the
first pass's.

**The last rule is a regex, and the word boundary is load-bearing.** `lore_metadata` appears
in prose in the crate's own doc comment. A bare `literal:lore_metadata==>` would match
*inside* `darkhold_lore_metadata` as well, because `_` is a word character and so there is
no word boundary at that position — the rule would rewrite the crate name a second time,
into `darkhold_darkhold_metadata`. `\b` prevents it.

## The paths are not this file's job

The rules file changes **contents**. The paths are changed by the filename callback:

```bash
--filename-callback 'return filename.replace(b"lore-metadata", b"metadata").replace(b"lorehaven", b"darkhold")'
```

**The callback's argument is named `filename` and it is BYTES.** A `str.replace` on it does
not compile.

## The command, and the fact that it is reproducible

```bash
git filter-repo --force --replace-text scripts/rename-replacements.txt \
  --filename-callback 'return filename.replace(b"lore-metadata", b"metadata").replace(b"lorehaven", b"darkhold")' \
  --refs master
```

**This single command reproduces the committed history exactly** — not merely an equivalent
tree, but **the same commit SHAs**, `faa03c7f73aa730764aa8ddc44d6fe3c28a80959` at the tip
and all 1,048 below it.

That was checked by cloning the pre-rename bundle, running the command above against it, and
comparing the result with the committed tree: **1,265 files, identical paths, identical blob
hashes, identical tip SHA.**

**A rewrite that cannot be re-run is a rewrite that cannot be checked.** This one can.
