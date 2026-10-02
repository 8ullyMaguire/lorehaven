# Gap C — AI pre-read scoring (#18)

**Status:** design settled, not yet implemented. Written before any code, per the
standing workflow.

The audit calls this the highest-value of the six gaps ("the four things that matter
*more* than ranking tweaks"), because it is the cold-start answer: every ranking signal
in §20.10.4 needs readers, and a new work has none. Nothing here is a quality *judgement*
of prose — it scores an import against the operator's configured dimensions, which is
what §23.7's "optional metadata suggestions" already authorises.

## What the spec allows, and what it forbids

Three constraints shape the whole design, and two of them are prohibitions:

- **§23.7.** Disabled without configuration. Explicit private-text consent. Quoted
  costs. No automatic publication. Generated output distinguished from author text.
  Deterministic abstention when no provider is configured (the §35.5 thread-summary
  precedent).
- **§32.6.** "It does not display composite quality scores publicly by default."
  So a pre-read score is **never public** and never appears on a work page.
- **§0.3.** "No credit, payment or trust level can move any ranking signal." A paid
  author's work and a free author's work must be scored identically, or this is the
  mechanism §0.3 forbids wearing an AI's name.

## Design decisions

### D1. The score is per-work, per-dimension, and private

One score per (work, dimension), never a single composite. A work is scored against
each operator-configured dimension independently, so "too long" and "wrong tone" are
separately visible and separately actionable. §32.6 forbids the public composite; the
private per-dimension form is what makes it useful without leaking a judgement.

### D2. The AI returns a verdict against a schema, never prose

A provider returns structured JSON conforming to a validated schema. Unparseable or
out-of-range output is **discarded and recorded as an abstention**, not coerced. This is
the same discipline as the Blind Date ordering decision: never let a component that
cannot be trusted produce a value that looks trusted.

### D3. No provider means no score, and that is a normal answer

`Option<PreReadScore>` throughout. An operator with no provider configured gets `None`
and the UI omits the section. This mirrors §35.5's "deterministic abstention when no
provider" and is why the trait is an `Option`-returning boundary rather than a
`Result`-returning one that the caller must translate.

### D4. Consent is per (author, provider), checked at call time

Not at configuration time. §23.7 requires "users may opt out of specific providers" and
"explicit private-text consent" — both are properties of the author *at the moment of
the call*, because consent can be withdrawn. Stored consent that is not re-checked is
consent the author cannot take back.

### D5. Cost is quoted before the run, and the quote is the test

The provider trait takes a `CostQuote` and returns a `CostQuote` actually charged.
A caller that would exceed its budget does not call. This is §22.11's guardrail and
§23.7's "quoted costs" together, and it is why the trait is async and fallible rather
than a pure function.

### D6. The score can never move a ranking signal

Not a parameter, a promise: `PreReadScore` has no path into any ranking query. If it
were allowed to weight discovery, §0.3 is violated the moment an author pays. It is
advisory metadata on the author's own work page, and the cold-start problem it solves
is *the author's* problem — knowing before publication that a work is 40k words when
this fandom's median is 8k.

## Implementation order

1. `crates/domain/src/ai.rs` — the `AiProvider` trait, `AiTask` enum, `CostQuote`,
   `PreReadVerdict`, and the schema validation. Pure domain, no I/O, unit-testable.
2. `crates/domain/src/preread.rs` — dimension aggregation from per-dimension verdicts
   into a per-work report, with the abstain path.
3. `crates/app/src/ai/` — provider adapters (Ollama, OpenAI-compatible) and the
   consent check.
4. Tests at each step, mutation-gated like gaps B and F.

## Not doing, deliberately

- **No vector store.** Embeddings are §35.5's semantic search, a different feature that
  happens to share the provider trait. Building both at once means neither is verified.
- **No automatic publication of AI output.** §23.7 forbids it; `PreReadReport` is
  read-only.
- **No scoring of other authors' works.** Only the work's own author sees a score.