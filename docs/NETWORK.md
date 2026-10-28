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

## 4. One person, two sites: signing in by handle

`POST /api/v1/auth/login` takes an **identifier**, not an address: a value containing `@` is looked up as a
mailbox, anything else is looked up as a pseud handle (`identity::find_account_by_handle`, oldest pseud
first, case-insensitive). So `Critical_Nexus` signs in here the same way they sign in on Ascend, without
inventing an address for them.

**Not single sign-on.** The two sites hash passwords independently, so "the same account" today means the
same handle and a password set on each site. Real SSO is an OIDC provider and belongs to T5 in
`~/code-local/rust/ascend/docs/PLAN-network.md`; nothing here pretends otherwise.

*Status: **proven live, 2026-10-28** — deployed, and `Critical_Nexus` signs in here with the same credentials
as on Ascend (handle only, no address typed). Two tests in `crates/app/tests/login_by_handle.rs`: the first was
checked against the old behaviour (email-only routing makes it fail), the second pins the property that
matters more — a wrong password and an unknown handle are both a plain `401`, so the endpoint is not an
oracle for which handles exist.*

### The account on this side

`Critical_Nexus` (`25e8fe74-…`) holds everything this instance defines:

| privilege | where | value |
|---|---|---|
| operator | `darkhold.toml` `[administration] operator_account_id` | this account |
| trust | `accounts.trust_level` | `6` = `TRUST_MAX` (`TL_TRUSTEE`) |
| admin role | `operator_role` | `admin` |
| capabilities | session | `can_read`, `can_write`, `can_message`, `can_be_listed`, `max_rating: explicit` |

Its password had to be reset by writing an Argon2id PHC string into `password_credentials` (there is no
reset CLI): the account predates this work and was the operator placeholder. Worth adding a
`reset-password` subcommand — a person who forgets their password currently cannot be helped at all.
