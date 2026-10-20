# 16. Extensions and the marketplace

**Status:** reserved
**Milestone:** M23
**Source:** the corresponding section of the Darkhold specification

## 16.1 The kinds

Reader widgets · dashboard widgets · themes and layout presets · recommendation engines ·
declarative recipes · search helpers · writing tools · challenge variants · positivity filter
rules · mood tag extensions · fandom landing templates · integrations · **source adapters**.

## 16.2 The power ladder

| tier | what it is | risk | the operator's cost |
|---|---|---|---|
| `config` | a set of options and weights | none | negligible |
| `recipe` | a JSON composition of existing parts | none | negligible |
| `service` | a URL implementing a protocol | medium — *their* server | a timeout, a circuit breaker, a cache |
| `wasm` | a small scoring function in a sandbox | medium-high | a runtime, a fuel meter, a memory meter |

**The most powerful tier is the cheapest for the operator.** Compute lives on the user's server, so
the operator's bill does not scale with the ecosystem. The safest tiers cover roughly 90 % of
demand, so the sandbox is not on the critical path.

**Themes are token values only, never raw CSS.** A theme cannot inject a stylesheet, so a theme is
not an injection surface.

**WASM is gated.** The sandbox must exist, the domain lockdown must be written and attacked, and a
second reviewer must have read it — enforced by a build script that fails the build if a WASM
runtime is adopted before then.

## 16.3 The marketplace mechanics

Publish (trust-gated) · a gallery filtered by kind · one-click **idempotent** install · 1–5 star
ratings with a running average · **remix** (copy any public extension as a private draft) · install
counts · a documented, versioned manifest format.

Every extension carries a **gate**: the trust level required to install it, and the permissions it
requests. Trust and subscription change the *ceiling* on resource use, never the *permissions*.

---
