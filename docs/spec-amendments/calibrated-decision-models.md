# Amendment — calibrated decision models (Laya), 2026-09-29

**Status:** accepted. **Applies to:** §11.14, §12.1, §30 (Decision Service), §0.4
(operator disclosures), §33 (positivity).

## 1. What this is

Laya is a small local decision model served by Unsloth at
`POST /v1/systemone`. It maps text to *calibrated probabilities* over a
declared question set: `noul` (yes/no), `choice` (one of N), `score` (a
position on a scale). It is 678 MB, runs on CPU, and needs 4 GB RAM.

The specification already asks for exactly this shape of decision in two
places, and in both places the instance currently answers with a keyword
list. This amendment wires the model in as a **second opinion beside the
deterministic classifier, never in place of it.**

## 2. The problem, stated in the spec's own terms

### 2.1 §11.14 grades the result, and a keyword list cannot grade

§11.14 requires three outcomes — `accepted`, `rejected`, `held` — and says a
rejection must be *certain*:

> **rejected** — certainly not a work: an empty or placeholder title or
> author, or canonical filler text in a metadata field.

`ImportWork::classify_quality` implements this with `is_placeholder_title`,
a `matches!` over six literal strings plus two `starts_with` prefixes. It is a
correct implementation of a *weaker* spec than §11.14 actually states: a title
of `Work 17 (anonymous)` is a placeholder to a reader and a work to the
function, and the function accepts it.

The three-way split is a probability problem, not a pattern problem. "Certainly
not a work" is a claim about a posterior, and §11.14's own default — *reject the
obvious junk, hold the rest* — is a statement about where to put the threshold.

### 2.2 §12.1 positivity is the same shape

`positivity::classify` matches against `HOSTILE_MARKERS` and
`CONSTRUCTIVE_MARKERS`. It returns a `confidence_bp`, so it already has the
shape of a calibrated answer, but the number is a constant per branch
(`8500`, `6000 + n * 1000`, `4500`) rather than a measurement. The spec's own
commitment at §12.1 — that the filter protects authors from readers while
letting real criticism through — is a judgement about text, and a keyword list
can only encode the judgement's author.

## 3. The amendment

### 3.1 A decision provider, not a decision

A new crate module `lorehaven_decisions` provides:

```text
DecisionProvider
├── Deterministic   — the existing classifiers, unchanged
└── Calibrated      — Laya via /v1/systemone, with the deterministic
                      classifier as its fallback
```

`Calibrated` **calls the deterministic classifier first**. It never overrides
it on the axis where the deterministic answer is unambiguous. Concretely:

| Deterministic says | Calibrated may do |
|---|---|
| `Rejected` with a *structural* reason (empty title, empty author) | nothing — agree |
| `Held` (no word count) | nothing — §11.14 says a zero word count is held, not rejected, and that is a rule, not a probability |
| `Accepted` | **narrow it to `held`** on a low confidence |

The model can only make the instance *more* cautious. It can never accept a
work the deterministic path rejected, and it can never reject one. This is the
asymmetry that makes the model safe to enable on an instance holding other
people's work: **the model's ceiling is a hold.**

### 3.2 Where it runs

The calibrated path runs in the **import worker**, not in a request handler.
§11.14's classification happens during an import; a request handler must not
block on a model load, and the first request against a cold model takes 10–20
seconds. The Decision Service endpoint (`POST /decisions/evaluate`) keeps
answering from the deterministic path, and gains an optional `provider` field
in the response so a caller can tell which answered.

### 3.3 Configuration

```toml
[decisions]
provider = "deterministic"   # or "calibrated"
base_url = "http://127.0.0.1:8888"
model     = "laya"
api_key   = "..."            # or LOREHAVEN_DECISIONS_API_KEY
timeout_ms = 5000
# The posterior at or above which a candidate may be ACCEPTED. Below it, the
# candidate is held. §11.14's "reject the obvious junk, hold the rest" means
# this starts HIGH and is lowered by an operator who has read the audit.
accept_threshold = 0.90
# Below this the model is not consulted at all, because a near-zero posterior
# on "is this a work?" is the deterministic path's own answer.
consult_floor = 0.10
```

`provider = "deterministic"` is the default and the deployed state of any
instance that has not set it. The model is opt-in.

### 3.4 Operator disclosure

`GET /api/v1/meta` gains the provider name and the active thresholds, beside
§0.4.3's existing operator-policy disclosures. An instance that classifies with
a model says so where §0.4 requires it to say what it is; a reader whose
comments are filtered by a 678 MB local model is owed the knowledge that a
model did it.

### 3.5 The audit trail that `/decisions/audit` promised

`GET /decisions/audit` currently returns `{ "items": [] }` — always. With the
calibrated provider it records, per decision: the task, the deterministic
answer, the model's posterior, the threshold, and which won. The endpoint
already exists and already claims to be operator-only; this gives it content.

## 4. What this amendment does NOT do

- **It does not remove the deterministic classifiers.** They are the floor, the
  offline path, and the reason a model can be switched off at any moment with
  no behaviour change beyond the model's own.
- **It does not classify beta-reader comments.** §18.8's exemption is
  explicit and stays explicit: blunt criticism is the point of a beta channel.
- **It does not make the model authoritative.** No output of
  `POST /v1/systemone` ever appears in the spec as a fact about a work, a
  comment, or an author. It appears as a hold decision, which a person can
  review.
- **It does not require a network.** `base_url` points at loopback. An instance
  with no model running gets the deterministic path and a recorded reason.
