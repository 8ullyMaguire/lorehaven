#!/bin/bash
# Mutation harness for work coordinates (spec §49.3, §49.7, §49.8).
#
# §49.8 asks for "byte for byte" reproducibility and for four works of identical
# tags to produce different coordinates. Both are claims a test can pass while
# the property is false, so each mutation breaks exactly one of them.
#
# Toolchain pinned to the ARCH cargo: a rustup shim in ~/.cargo/bin shadows it
# and rejects this crate's lints, which would make every mutation "fail to build"
# for an unrelated reason. Per-run backup + trap so a timeout cannot leave a
# mutation applied.
#
# NEVER COMMIT WHILE THIS IS RUNNING. The working tree holds a deliberately
# broken source for the duration; the EXIT trap restores it afterwards, but
# anything that reads the tree mid-run -- git add, git commit, an editor, an
# agent -- sees the mutation. That happened: commit 641f479 captured mutation 12
# (inter-word spaces treated as sentence boundaries) and needed 957512e to undo.
# It passed all ten tests anyway, which is the dangerous part: an equivalent
# mutant on the current fixtures is exactly what you would commit by accident.
#
# mutate() accepts EITHER an inline python body or a path to a .py file. The file
# form exists because one inline mutation was a shell single-quoted string wrapping
# a Python triple-quoted string wrapping Rust double quotes; the shell ate the
# quoting, the mutation silently never ran, and the harness printed ANCHOR
# MISSING -- which reads like a code problem and is not one.
set -u
cd /home/alvaro/code-local/rust/lorehaven || exit 1
export PATH=/usr/bin:/bin
export RUSTFLAGS="--cap-lints=warn"
CARGO=/usr/bin/cargo
SRC=crates/domain/src/coordinates.rs
BACKUP=/tmp/mutcoord-$$
mkdir -p "$BACKUP"
# Restore from GIT, not from a copy taken at startup. The copy approach failed
# twice: once a commit captured a live mutation, and once a killed run restored a
# backup that had itself been taken mid-run. `git checkout --` restores whatever
# was last committed, which is the only definition of "correct" that matters here.
# The harness therefore requires a clean tree for its target before it starts.
if ! git diff --quiet -- "$SRC"; then
  echo "REFUSING TO RUN: $SRC has uncommitted changes."
  echo "  A mutation harness restores from git; with a dirty tree it would discard"
  echo "  your work or compound it. Commit or stash first."
  exit 2
fi
cp "$SRC" "$BACKUP/coordinates.rs"
restore() { git checkout -- "$SRC"; }
trap 'restore' EXIT INT TERM

classify() {
  local label="$1" out="$2"
  if echo "$out" | grep -qE '^error(\[[A-Z0-9]+\])?: (could not compile|aborting)'; then
    echo "NOT A KILL (build failed): $label"
    echo "$out" | grep -E '^error' | head -2
  elif echo "$out" | grep -q 'test result: FAILED'; then
    echo "KILLED: $label"
    echo "  failing: $(echo "$out" | grep -E '^    [a-z_]+$' | tr -d ' ' | tr '\n' ' ')"
  elif echo "$out" | grep -q 'test result: ok'; then
    echo "SURVIVED (suite still green): $label"
  else
    echo "INDETERMINATE: $label"; echo "$out" | tail -4
  fi
}

mutate() { # $1 label, $2 python body OR a path to a .py file
  echo "=================== MUTATION: $1"
  restore
  # The argument may be either an inline body (the original form) or a path to a
  # .py file. The file form exists because one inline mutation contained a shell
  # single-quoted string wrapping a Python triple-quoted string wrapping Rust
  # double quotes; the shell ate the quoting, the mutation silently never ran,
  # and the harness reported ANCHOR MISSING -- which reads like a code problem
  # and is not one. Anything ending in .py is treated as a file, and a missing
  # file is reported as its own failure so it can never again look like a verdict
  # on the code.
  local arg="$2"
  local applied=1
  if [ -f "$arg" ]; then
    SRC="$SRC" python3 "$arg" || applied=0
  else
    SRC="$SRC" python3 -c "$arg" || applied=0
  fi
  if [ "$applied" -ne 1 ]; then
    echo "MUTATION DID NOT APPLY (never tested): $1"
    return
  fi
  # Hard timeout per mutation. Without it a mutation that makes the suite deadlock
  # hangs the whole run: mutation 11 removed the loop's only index increment and the
  # test binary sat in futex_wait for 2h03m of CPU time. That is a real finding
  # about the mutation, and the harness must be able to report it as HUNG rather
  # than stall indefinitely.
  local out
  out="$(timeout --signal=KILL 300 $CARGO test -p lorehaven-domain --test coordinates 2>&1)"
  local rc=$?
  if [ $rc -eq 137 ] || [ $rc -eq 124 ]; then
    echo "HUNG (timeout 300s, likely an infinite loop or deadlock): $1"
    pkill -KILL -f 'deps/coordinates-' 2>/dev/null
    return
  fi
  classify "$1" "$out"
}

# 1. THE determinism mutation, and the one that matters most. A HashSet instead of
#    a BTreeSet for the vocabulary count: iteration order becomes seed-dependent,
#    which is invisible in a single process and wrong across two.
mutate "vocabulary counting uses a randomly-seeded HashSet" '
import os
p=os.environ["SRC"]; s=open(p).read()
old="    let mut distinct: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();"
new="    let mut distinct: std::collections::HashSet<String> = std::collections::HashSet::new();"
assert old in s, "anchor missing"
open(p,"w").write(s.replace(old,new,1))
'

# 2. The absent-is-not-a-zero rule inverted: a short work measured as zeros.
#    This is the single most important property in §49.3.
mutate "a work under the minimum is measured as zero instead" '
import os
p=os.environ["SRC"]; s=open(p).read()
old="""    if word_count < MIN_MEASURABLE_WORDS {
        return Coordinates::Unmeasurable(Unmeasurable::TooShort);
    }"""
new="""    if false {
        return Coordinates::Unmeasurable(Unmeasurable::TooShort);
    }"""
assert old in s, "anchor missing"
open(p,"w").write(s.replace(old,new,1))
'

# 3. Variance normalised by the MEAN rather than the longest sentence, which
#    compresses the range and blunts the measure that separates varied from flat.
mutate "sentence variance is normalised by the mean, not the longest" '
import os
p=os.environ["SRC"]; s=open(p).read()
old="    ((variance.sqrt() / longest).clamp(0.0, 1.0), variance)"
new="    ((variance.sqrt() / mean).clamp(0.0, 1.0), variance)"
assert old in s, "anchor missing"
open(p,"w").write(s.replace(old,new,1))
'

# 4. Case folding dropped, so "The" and "the" inflate the type count.
mutate "vocabulary richness does not case-fold" '
import os
p=os.environ["SRC"]; s=open(p).read()
old="            .flat_map(|c| c.to_lowercase())"
new=""
assert old in s, "anchor missing"
open(p,"w").write(s.replace(old,new,1))
'

# 5. A single-chapter work reports spread 0.0 instead of None -- the
#    absent-is-not-a-zero mistake one level down.
mutate "a single-chapter work reports zero spread instead of none" '
import os
p=os.environ["SRC"]; s=open(p).read()
old="""    if chapters.len() < 2 {
        return None;
    }"""
new="""    if chapters.len() < 2 {
        return Some(0.0);
    }"""
assert old in s, "anchor missing"
open(p,"w").write(s.replace(old,new,1))
'

# 6. The dialogue clamp removed, so a bad count yields a coordinate above 1.0 --
#    outside the scale a reader weight lives on.
mutate "the dialogue ratio is not clamped to the weight scale" '
import os
p=os.environ["SRC"]; s=open(p).read()
old="        (corpus.dialogue_words as f64 / word_count as f64).clamp(0.0, 1.0)"
new="        corpus.dialogue_words as f64 / word_count as f64"
assert old in s, "anchor missing"
open(p,"w").write(s.replace(old,new,1))
'

# 7. Chapters joined with a single newline, so the last sentence of one chapter
#    and the first of the next are counted as one sentence. Deterministic, and
#    still wrong -- which is the point: §49.3 wants reproducibility AND a measure
#    that means something.
mutate "chapters are joined with one newline, merging boundary sentences" '
import os
p=os.environ["SRC"]; s=open(p).read()
old="                out.push_str(\"\\n\\n\");"
new="                out.push_str(\"\\n\");"
assert old in s, "anchor missing"
open(p,"w").write(s.replace(old,new,1))
'

# 8. Threshold off by one, so a work of exactly MIN_MEASURABLE_WORDS is
#    unmeasurable. §49.3 does not name the number, so the boundary is a
#    decision -- and a decision nobody pins is a decision nobody made.
mutate "the minimum-length boundary is off by one" '
import os
p=os.environ["SRC"]; s=open(p).read()
old="    if word_count < MIN_MEASURABLE_WORDS {"
new="    if word_count <= MIN_MEASURABLE_WORDS {"
assert old in s, "anchor missing"
open(p,"w").write(s.replace(old,new,1))
'

# 9. An unmeasurable work is silently treated as measured, by mapping the enum
#    to a zeroed coordinate at the boundary the type is supposed to protect.
mutate "unmeasurable is reported as measured" '
import os
p=os.environ["SRC"]; s=open(p).read()
old="""    pub fn is_measured(&self) -> bool {
        matches!(self, Coordinates::Measured(_))
    }"""
new="""    pub fn is_measured(&self) -> bool {
        true
    }"""
assert old in s, "anchor missing"
open(p,"w").write(s.replace(old,new,1))
'

# 10. Single newline as the chapter join, so a boundary falls inside a sentence
#     rather than between two. Deterministic, and the sentence count gives it away.
mutate "chapters joined with a single newline" '
import os
p=os.environ["SRC"]; s=open(p).read()
old="                out.push_str(\"\\n\\n\");"
new="                out.push_str(\"\\n\");"
assert old in s, "anchor missing"
open(p,"w").write(s.replace(old,new,1))
'

# 11. The blank-line rule inverted to "any newline is a break", which is the first
#     bug this suite caught: a "\n\n" join becomes two breaks and the empty one
#     invents a one-character sentence.
mutate "every newline is a break, so a blank line emits a phantom sentence" '
import os
p=os.environ["SRC"]; s=open(p).read()
old="            if newlines >= 2 && current > 0 {"
new="            if newlines >= 1 && current > 0 {"
assert old in s, "anchor missing"
open(p,"w").write(s.replace(old,new,1))
'

# 12. Spaces treated as sentence breaks -- the second bug. Splits ordinary prose
#     into one-word fragments and INVERTS the measure.
#
#     Written to a file rather than inlined: this one used to be a single-quoted
#     shell string containing a Python triple-quoted string containing Rust double
#     quotes, and the shell ate the quoting. The mutation silently never ran, and
#     the harness printed ANCHOR MISSING -- which reads like a code problem and is
#     not one. mutate() now takes a script file, so this class of failure cannot
#     recur quietly.
cat > "$BACKUP/m12.py" <<'PY'
import os
p = os.environ["SRC"]; s = open(p).read()
old = """        current += 1;
        if matches!(ch, '.' | '!' | '?') {"""
new = """        if ch == ' ' {
            if current > 0 {
                lengths.push(std::mem::take(&mut current));
            }
            continue;
        }
        current += 1;
        if matches!(ch, '.' | '!' | '?') {"""
assert old in s, "anchor missing"
open(p, "w").write(s.replace(old, new, 1))
PY
mutate "spaces are treated as sentence boundaries" "$BACKUP/m12.py"

# 13. The saturation regression: sd/mean instead of sd/max, which only lands in
#     range by being clamped and so loses all resolution at the top.
mutate "variance normalised by the mean, saturating the top of the range" '
import os
p=os.environ["SRC"]; s=open(p).read()
old="    ((variance.sqrt() / longest).clamp(0.0, 1.0), variance)"
new="    ((variance.sqrt() / mean).clamp(0.0, 1.0), variance)"
assert old in s, "anchor missing"
open(p,"w").write(s.replace(old,new,1))
'

restore
echo "=================== done; tree restored"
echo "btree: $(grep -c 'BTreeSet' "$SRC")  threshold: $(grep -c 'word_count < MIN_MEASURABLE_WORDS' "$SRC")"
