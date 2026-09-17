# docs/books — the implementation tutorial

This directory holds *Building Lorehaven*, the tutorial for building the site
from an empty directory to a running instance.

## Layout

| Path | What it is |
|---|---|
| `parts/` | the book, one file per part, in reading order |
| `lorehaven-tutorial.md` | all parts merged into one document (generated) |
| `lorehaven-tutorial.epub` | the same book as an EPUB (generated) |

`parts/` is the source of truth. The merged markdown and the EPUB are built from
it; never edit them by hand.

## Regenerating

```bash
cd docs/books

# merge the parts in filename order
python3 - <<'PY'
import glob
parts = sorted(glob.glob("parts/*.md"))
merged = "\n\n".join(open(p, encoding="utf-8").read().rstrip("\n") for p in parts) + "\n"
open("lorehaven-tutorial.md", "w", encoding="utf-8").write(merged)
print(f"merged {len(parts)} parts, {len(merged)} bytes")
PY

# build the EPUB (needs pandoc)
pandoc lorehaven-tutorial.md -o lorehaven-tutorial.epub \
  --toc --toc-depth=2 --split-level=1 --standalone \
  --metadata title="Building Lorehaven" \
  --metadata author="Lorehaven contributors" \
  --metadata lang=en
```

The EPUB must keep `mimetype` as its first entry, stored uncompressed — pandoc
does this correctly; a hand-rolled zip usually does not, and readers reject the
file. To check a build:

```bash
python3 -c "import zipfile; z=zipfile.ZipFile('lorehaven-tutorial.epub'); \
print(z.namelist()[0], z.infolist()[0].compress_type, z.read('mimetype').decode())"
# mimetype 0 application/epub+zip
```

## Rules for the parts

- One part per vertical slice; the filename order is the reading order, and the
  numeric prefix is the part's position.
- Every part ends with a checkpoint tag and an honest statement of what is still
  owed.
- File paths, migrations, modules and commands named in a part must exist in the
  repository. When something is not built, the part says so.
- A part teaches the complete design of its area; where the reference
  implementation is incomplete, that is stated in the part rather than smoothed
  over.
