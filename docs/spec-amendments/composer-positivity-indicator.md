# Amendment: a live positivity indicator in the composer — measured, and NOT as a score

Status: **evaluated and rejected as specified** (2026-09-29). Supersedes nothing;
records why a plausible feature is not to be built, and what *is* worth building
in its place.

## 1. The proposal

A live indicator in the post composer, refreshing every *x* ms (configurable)
after the writer pauses, showing the probability that the draft would be flagged
as passive-aggressive or hostile. Nothing shown if the model is unavailable.
Motivated by §12.1 positivity being the site's first priority, and by Laya
already being integrated for the §11.14 calibrated-import path.

## 2. What was measured

Against the real model on thinkcentre (Unsloth Decision API, `laya`, 678 MB
multilingual, CPU-only: i5-8500T / 6 cores / 5.5 GB available, 9.9 GB swap
nearly full). 28 labelled texts — 10 hostile, 18 clean, and the 8 "hard
negatives" that matter most: sharp or negative but legitimate criticism.

A `noul` question returns a calibrated probability of "yes". Scores:

| framing | hostile mean | clean mean | separation |
|---|---|---|---|
| "passive-aggressive, hostile, demeaning, or hateful toward a person or group" | 0.264 | 0.013 | +0.251 |
| "Is this text toxic?" | 0.287 | 0.014 | +0.273 |
| "Is this rude or abusive to someone?" | 0.534 | 0.048 | +0.486 |

### The reliability table is the finding

Fraction of the 10 hostile texts scoring ≥ threshold, and how many of the 18
clean texts were false-flagged:

| threshold | hostile caught | clean false-flagged |
|---|---|---|
| 0.3 | 4/10 | 0/18 |
| 0.5 | 3/10 | 0/18 |
| 0.7 | 1/10 | 0/18 |
| 0.8 | 0/10 | 0/18 |
| 0.9 | 0/10 | 0/18 |

**There is no threshold that works.** At 0.5 it catches 3 of 10 genuinely
hostile posts while flagging none of the 18 legitimate ones — which sounds like a
result until you notice the *misses are the interesting cases*:

| score | text |
|---|---|
| 0.0097 | "Honestly your reading comprehension is genuinely shocking. Did you even finish primary school?" |
| 0.0752 | "I bet your mother would be ashamed of you too." |
| 0.0859 | "You're a waste of space and everyone is tired of you." |
| 0.1234 | "This is the dumbest take I've seen all year, and I've seen a lot." |

Scores below the *worst* clean text (0.0425). A model that calls "I bet your
mother would be ashamed of you" 7.5% toxic is not detecting passive aggression;
it is detecting profanity and second person.

### It is not a prompt-wording problem

Five further framings — insult, belittling, passive-aggressive, dismissive,
attacking-a-person — were run on the six missed texts alone:

| framing | mean on the missed texts | ≥ 0.5 |
|---|---|---|
| toxic | 0.142 | 0/6 |
| insult | 0.078 | 0/6 |
| belittling | 0.226 | 1/6 |
| passive-aggressive | 0.254 | 1/6 |
| dismissive | 0.151 | 0/6 |

**Zero of 30 framings × text combinations reached 0.5 on these texts.** The
capability gap is real, not a phrasing artefact.

### Latency, the part that would have worked

p50 **2.6–2.8 s**, p95 3.3 s, cold start 10–20 s. The vendor's claim of "well
under a second" does not hold for this host. At a 2.7 s median the "every 500 ms"
idea is off by 5×; every keystroke-triggered request would be queued and
stale-answered, so a debounce of 2–3 s *after a pause* is the floor, and the
indicator would be visibly behind the writer.

## 3. Why this is rejected rather than merely tuned

Three independent reasons, any one sufficient:

1. **The score is wrong in the direction that matters.** A 78% indicator on a
   genuinely abusive draft, and 1% on "I bet your mother would be ashamed of you",
   teaches writers that the number is arbitrary. Once the number is known to be
   arbitrary, the advisory is worse than nothing: a writer who trusts it will
   rewrite a clean post and *keep* an abusive one.
2. **Latency makes it structurally impossible** at the proposed cadence.
3. **A false positive on a hard negative is a policy decision, not a tuning
   problem.** The clean texts scoring highest (0.0425: "You're being
   deliberately obtuse and everyone here can see it") are exactly the honest
   criticism a forum exists for. Any threshold low enough to catch the misses
   starts touching them, and that is the operator's call, not a default.

The general lesson, and the reason this is written down rather than discarded:
**a "calibrated" probability is calibrated to the question it was asked.** Laya's
`noul` is well-calibrated *for "contains profanity or an overt insult"* — which
is what it was trained to detect. Asking it about passive aggression and reading
the number as an answer about passive aggression is the error, and the number's
familiar format is what makes the error easy to commit.

## 4. What is worth building instead

The measurement says the model is good at **overt insult and profanity** and bad
at **indirect, demeaning, sarcastic hostility**. So:

- **A concrete-match nudge, not a percentage.** On submit (not on every
  keystroke), scan for the markers §12.1 already defines, and if any are present
  offer a specific rewrite suggestion naming the phrase. Deterministic, instant,
  no model, and it cannot be wrong in the way the score was.
- **Post-publication human review** for reported content, which is the mechanism
  §12.1 already specifies and the one that does not need a model to be right.
- **Only if the model is retrained for the construct** — and then re-run exactly
  this evaluation, with a corpus drawn from *this* forum's moderation history
  rather than from a generic one, because the generic corpus is what made the
  gap look smaller than it is.

The composer indicator stays out of the roadmap. The measurement is the artefact;
re-running it against a different model is one command.

## 5. Reproducing

Harness and results on this host's scratch: `positivity_eval.py` (28-text
labelled corpus, reliability table, latency percentiles),
`positivity_prompt_variants.py` (5 framings on 6 probes),
`positivity_sanity.py` (5 framings on the missed texts). All against
`http://127.0.0.1:8888/v1/systemone` on thinkcentre, model `laya`. No secrets in
the harness; the key is read from `~/.unsloth/systemone.key` at runtime.
