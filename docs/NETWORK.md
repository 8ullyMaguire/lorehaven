# The Ascend handshake

How a Darkhold work and an Ascend build point at each other, without either service calling the other.

**Why a file format and not an API.** The two products do not share a database, a session, or a clock yet
(see `~/code-local/rust/ascend/docs/PLAN-network.md`, T5). Until they do, the only honest contract between
them is *text a person carries*: a seed out of Ascend, a URL back. Anything that requires one service to
call the other today would be a coupling neither can test alone.

## 1. Ascend → a writer: the seed

`GET https://ascend.polarisocial.xyz/api/builds/{id}/export?format=seed` returns markdown. It is a public
read of a public build; no account is needed to fetch or to keep the file. It ends with the build's URL and
one instruction: if you write from this, attach your story to the build.

The seed is deterministic: the same build produces the same text. That is deliberate — a writer quoting the
seed in a bug report or a Discord thread produces something reproducible.

## 2. A writer → Ascend: the claim

`POST https://ascend.polarisocial.xyz/api/builds/{id}/story` with `{"url": "https://…"}`. Rules, all
verified in `crates/ascend-api/tests/growth.rs`:

| situation | result |
|---|---|
| build has no story, any session claims | `200`, link set, `story` entry in the build log |
| the same claimant repeats the same URL | `200`, idempotent |
| a different claimant, different URL | `409 conflict`, `details.story_url` names the story that holds it |
| the build's owner, any URL | `200`, replace allowed — it is their build |
| not an `http(s)` URL, whitespace, over 2048 chars | `400 bad_request` |

The claim is rate-limited per claimant (5/hour). Attribution goes to the claimant, not to whoever drew the
build: the seed is written for someone who does not own the build, so the endpoint cannot require ownership.

## 3. Darkhold's side (not built yet)

When a work is published and the author supplies a source URL that is an Ascend build, Darkhold keeps it on
the work row and renders "built from a build on Ascend" with the link. No fetching, no verification of the
URL's contents beyond the same `http(s)` check Ascend applies. That is the whole obligation — a link in the
direction a reader travels.

The loop closes when both directions exist: Ascend shows *who wrote it*, Darkhold shows *what it was built
from*. Neither is complete alone, and neither needs the other's database.
