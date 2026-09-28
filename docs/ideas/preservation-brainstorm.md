# Preservation, Distribution, and Archive Ecosystem — Idea Dump

## Robots.txt and Fetching Ethics

- ✓ **Granular robots.txt honor levels** — Instead of respect/ignore binary, offer levels: `strict` (never fetch anything disallowed), `standard` (respect disallow for automated crawls but allow user-initiated fetches), `metadata-liberal` (fetch metadata regardless, respect for bodies), `preservation-emergency` (ignore all denials for dying sources), `full-ignore` (adapter fetches whatever it can). Operator picks per source.

- ✓ **User-initiated vs. automated fetching distinction** — A robots.txt disallow generally targets crawlers, not individual users copy-pasting URLs. When a logged-in user pastes a URL, that's arguably a user-agent action, not a crawler. The fetch could carry a `User-Initiated: true` header and be treated differently from batch imports. Documented, honest, aligns with the spirit of robots.txt (which is anti-crawler, not anti-user).

- ✓ **Rate-limited politeness even when ignoring robots.txt** — If Lorehaven does ignore robots.txt for metadata, at minimum respect crawl-delay directives, use single-connection sequential fetching, and back off aggressively on 429s. Bad citizenship is different from ethical disagreement about robots.txt scope.

- **Robots.txt change alerting** — When a source's robots.txt changes materially (starts denying, starts allowing, adds crawl-delay), flag this to instance operators. They can decide whether to adjust their instance policy or notify users of coming import limitations.

- **Adapter-declared ethical stance** — Each adapter ships with a documented ethical position: "This adapter treats AO3 as permissive-metadata / restricted-body based on their public statements." Operators see and acknowledge this at install time. Reduces "wait, we were doing what?" moments.

- **Source-declared preferences beyond robots.txt** — Some sources publish machine-readable preservation preferences (e.g., a `/.well-known/preservation-policy.json` file). Adapters check for these; where present, they override robots.txt inferences with explicit source intent.

- **Meta-robots and X-Robots-Tag respect** — Adapters check page-level meta robots tags and HTTP X-Robots-Tag headers, which are more granular than site-level robots.txt. A source might allow crawling generally but mark specific works as `noarchive`.

- **Human-readable ToS parser** — Ambitious: adapters include a summary of the source's Terms of Service regarding scraping/mirroring, updated when ToS changes materially. Users see this before importing. Not legal advice, but transparency.

- **Consent inference from license** — If a work carries a CC license, that's affirmative consent for redistribution under the license's terms. Adapters detect Creative Commons metadata and treat CC-licensed works as consent-clear regardless of source robots.txt.

- **Ethical mode presets** — Instance-level shortcut: `ethics = "strict" | "balanced" | "preservationist"`. Strict respects everything, balanced follows the metadata-vs-content split, preservationist prioritizes work survival with disclosed tradeoffs. Individual settings still overridable.

---

## Cross-posting and Distributed Preservation

- ✓ **Preservation coverage score, publicly visible per work** — A small icon showing how many independent archives host the work: 🔴 (1, fragile), 🟡 (2, basic), 🟢 (3+, preserved). Author-toggleable. Readers who care about long-term availability can factor this in.

- ✓ **Preservation coverage as a search filter** — Advanced search option: "only show works preserved on ≥3 archives" or "prioritize well-preserved works." Serves readers who've been burned by fic loss and want durability signals.

- ✓ **Diminishing rewards past threshold** — First cross-post: 10 credits. Second: 15 (hits preservation threshold). Third: 8. Fourth: 5. Fifth: 3. Sixth+: 1. Rewards preservation, discourages spamming registrations at 47 tiny archives.

- ✓ **Verification could be done by curators through quorum, rather than automated, per instance configuration**

- ✓ **Users above x trust level could submit cross-posts not just authors** verified through quorum

- **Verification via reciprocal ping** — When author claims a cross-post to Archive B, Lorehaven attempts a lightweight verification fetch to confirm the work exists there. If Archive B is a Lorehaven instance, they exchange federation signatures. If it's AO3/FFN, adapter probes for the work. No verification = no credit.

- **Verification codes embedded in cross-posts** — When cross-posting, include a Lorehaven verification token in the work's end notes (invisible or explicitly labeled). Lorehaven periodically checks target archives for the token, confirming ongoing presence.

- ✓ **Cross-post health monitoring, link rot detection** — Cached preservation coverage is periodically re-verified. If a mirror disappears (link 404s, work deleted from source), the coverage score drops and the author gets a gentle notification: "Your work is no longer preserved on Archive B. Would you like to re-mirror?"

- **One-click multi-archive publish** — Authoring interface has "publish to Lorehaven + N other archives" as a single action. Each configured archive has an adapter that handles format conversion, metadata mapping, and posting. Reduces cross-posting friction from N×(steps) to 1×(steps).

- **Draft synchronization across archives** — Authors can edit a work in Lorehaven and push updates to all cross-post targets. Handles edits, new chapters, deletions. Turns Lorehaven into the canonical source of truth even when the reader-facing archive is elsewhere.

- ✓ **Preservation Champion badge** — Authors whose entire published body of work is above the preservation threshold earn a badge. Purely honorific. Visible on profile if opted in.

- **Preservation streak** — Author has cross-posted every work published in the last 12 months. Recognized quietly on profile. Not a leaderboard.

- **"This work exists on N archives" author notification** — Positive reinforcement: "Congrats, your work is now preserved on 3 independent archives. Great chance of long-term survival." Small dopamine hit for good preservation practice.

- ✓ **Cross-post adapter marketplace** — Third parties can write adapters for niche archives (fandom-specific wikis, small language communities, personal blogs). Same review process as extensions. Grows the cross-post target list beyond the platform's default set.

- ✓ **Auto preservation between Lorehaven instances**, mirror public works on every federated instance that allows preservation

- **Preservation partnership between two Lorehaven instances** — Instance A and Instance B mutually agree to mirror each other's public works (with author opt-in). If A goes down, B has copies with clear provenance. Federation-native preservation.

- **Multi-instance failover fetch** — When a reader tries to read a work whose primary source is unavailable, Lorehaven automatically tries mirror sites in order of preference. Reader sees the work; source badge updates to reflect actual source served.

- **Author's mirror declaration** — Author page can declare "canonical mirrors of my works" with URLs. Lorehaven links to these on every work page. Author-owned distribution announcement.

- **Cross-post preview** — Before publishing to cross-post targets, show a preview of how the work will appear on each target (formatting differences, metadata mapping, image handling). Prevents surprise.

- **Cross-post rollback** — If a cross-post goes wrong (formatting broken, wrong tags applied), author can trigger rollback from Lorehaven, and the adapter attempts to fix or remove the target-side copy.

- ✓ **Rescue emergency batch import** when a source is dying all it's works should be preserved. Flagging requires TL≥N and quorum. All works for a succesul flagged are imported automatically.

- **"Rescue import" quick action** — When a reader knows a source is dying, they can flag works in that source for rescue import. Requires TL≥N and quorum. Batch import proceeds under preservation-emergency clause.

- **Dying archive early-warning system** — Community-maintained list of at-risk archives (announced shutdowns, declining activity, ownership changes). Instance operators subscribe to alerts. Users can subscribe too. Prompts preservation action before crisis.

- **Preservation batch coordination between instances** — When Archive X announces shutdown, multiple Lorehaven instances coordinate to preserve different fandoms/genres to avoid duplicated effort and gaps. Distributed preservation with light coordination.

- **Author bibliography snapshotting** — For each followed author, periodically snapshot their full public work list and metadata (not necessarily bodies). If they later delete works, the snapshot preserves at least the record that the work existed. Basis for offering to rescue-cache if the author later requests.

- **Reader-initiated preservation request** — A reader who loves a work can send a request to the author: "I love this work — would you consider cross-posting to preserve it?" Framed positively, not naggingly. Rate-limited (one per work per reader).

- **Preservation as gift** — A reader (TL≥N) can offer to handle the technical cross-posting on behalf of an author who's willing but doesn't want to deal with the process. Author reviews and approves each target. Reader gets small credit; author gets preservation credit.

---

## Metadata vs. Body Storage Modes

- ✓ **Instance content policy declaration** — Publicly declared and machine-readable at `/api/v1/meta`: this instance is metadata-only, hybrid, or full-mirror. Users know before signing up what to expect.

- **Per-source policy overrides** — Instance-wide default plus per-source rules: "we're metadata-only in general but full-mirror for dyingarchive.example." Fine-grained control matching the mixed reality of source policies.

- **Storage badge per work** — 📖 Read here / 🔗 Read at source / ⏳ On-demand fetch. Reader knows before clicking whether they're staying on Lorehaven or leaving.

- **Metadata-only reading UX** — Clicking "read" on a metadata-only work opens the source in a new tab, or provides an embedded reader frame, or fetches-and-displays-without-storing. Reader choice, defaulting to whatever the instance prefers.

- ✓ **On-demand fetch with ephemeral cache** — For hybrid instances: when a reader opens a metadata-only work, body is fetched, displayed, and cached for session, hours or days, then discarded. Not persistent storage; just enough to serve the reader smoothly.

- ✓ **TL≥N User-triggered permanent cache (configuration, default off, always cache)** — Reader with sufficient trust bookmarks a metadata-only work "for offline access" — this triggers a persistent cache decision (subject to instance policy). Their bookmark request is what makes it cache.

- ✓ **Reader-side ephemeral cache in PWA** — Even if the instance doesn't cache the body, the reader's browser (via PWA / service worker) can. The instance never stores it, but the reader has offline access. Distributed reader-side preservation.

- **Instance identity: catalog vs. archive** — Some instances proudly identify as "catalog" (metadata-only, curatorial focus, low resource use) vs "archive" (full mirroring, preservation focus, high resource use). Design should support both proudly without treating one as lesser.

- **Storage costs shown transparently** — Instance admin panel shows storage used by cached bodies vs. metadata vs. media. Helps operators understand cost implications of storage mode choices.

- **Author-requested full mirror even on catalog instance** — Even a metadata-only instance can honor an individual author's request: "please mirror my work here, I'd like to be preserved." Author-directed exception to instance default.

- ✓ **Cache tiers** — Bodies accessed more often live in fast cache. After N days without reads, demoted to cold storage. After M days, dropped entirely (depending on instance configuration, off by default). Storage cost management.

- ✓ **Source-death auto-mirror** — If source health monitoring shows a source is failing (repeated errors, ownership change, shutdown announcement), instance policy can auto-escalate metadata-only works from that source to full-mirror status before it's too late.

- ✓ **Metadata-only can still be beautiful** — Design consideration: the work page for a metadata-only work should feel complete, not like an empty stub. Rich metadata display, author info, related-works, external link should feel intentional, not apologetic.

- ✓ **Content shape declaration** — Beyond storage mode, declare *what kind* of content this instance holds: fanfic only, all fiction, meta and essays, multimedia, etc. Sets reader and author expectations.

---

## Trust-Gated Community Caching

- ✓ **Cache requested works only ()** Users with vanguard role, high resonance with instance taste profile and TL≥N propose and approve body caches, all other works are metadata-only.

- **Cache proposal workflow** — TL≥N user encounters metadata-only work, proposes "cache body on this instance." Enters a lightweight review queue. Quorum of TL≥N users approves or declines. Approved works get fetched and cached.

- ✓ **Cache access tiers** — Cached body accessible only to users at trust level ≥ M (configurable, may differ from proposal threshold). Lower-trust users still see metadata + external link. Two-tier access reflects two-tier responsibility for the cache decision.

- ✓ **Cache reader opt-in role** — Even at sufficient trust, users opt in to be "cache readers" — they acknowledge they're seeing content that may not have explicit author consent, and take individual responsibility for that. Others at same trust level see metadata unless they opt in.

- **Cache decision rationale required** — When proposing a cache, user provides reason: "preservation (source dying)," "author unreachable but work canonically important," "translation reference," "personal study." Rationale is audit-visible.

- ✓ **Cache voting weight by tenure** — Vote weight increases with time-at-trust-level. A user who just reached TL≥N doesn't have the same weight as a 3-year TL≥N contributor. Prevents newly-trusted users from swinging cache decisions.

- ✓ **Cache expiration and re-vote** — Cached bodies expire after configurable period (default 180 days) and require re-approval to persist. Ensures ongoing consideration rather than accumulated cruft.

- ✓ **Cache decision transparency to author (configurable, disabled by default)** — When a work is proposed for caching, if the author is identifiable and reachable (e.g., they're a Lorehaven user, or their source profile is contactable), notify them and give them a chance to consent, decline, or provide instructions.

- ✓ **Automatic cache-purge on author objection (allow disabling in configuration)** — If an author (verified as the author) objects to their work being cached, immediate purge. No committee discussion. Author sovereignty is absolute.

- ✓ **Cache decision review audit** — Every approved cache logged with proposer, voters, rationale, date, target work, source. Publicly auditable in aggregate; individually auditable by admin and/or TL≥N users.

- ✓ **Cache decision reversal** — TL≥N users can propose "un-cache this work" for reasons like: author later objected, source revived and is healthy, poor quality of cached copy, ethical concerns emerged. Symmetric to the approval process.

- **"This is cached" indicator** — Users who can see cached bodies see a clear indicator: "This body is cached on this instance under a community preservation decision made on [date]." Transparency to the reader about what they're seeing.

- **Cache access log for legal defensibility** — Access to cached bodies is logged (privately, per §3.7). Establishes that access is limited and controlled if legal challenge arises. Not used to police users.

- ✓ **Emergency-cache fast path** — If a source is confirmed dead or shutdown-announced, cache proposals from that source bypass quorum and are auto-approved by single TL≥N user. Preservation urgency overrides deliberation.

- **Cache proposals ordered by demand** — When multiple works are proposed for caching, order the queue by reader interest (bookmarks, unique fetchers, search hits). Ensures scarce cache decisions serve the most-wanted preservation.

- **Cache impact preview** — Before voting, show voters: estimated storage impact, source policy on caching, author identifiability, quality of the metadata (does this even seem worth caching?). Informed voting.

- ✓ **Peer-instance cache request** — If Instance A has cached a body and Instance B wants it (also under preservation grounds), federation protocol allows requesting the cached body between trusted partner instances. Reduces duplicated fetching from the source.

---

## Author Rights and Consent

- ✓ **Author claim mechanism** — External work imported without author being present. Later, that author joins Lorehaven and claims the work: "I'm the author of this." Verified through source-side signal (posting a claim code on their source profile). Once claimed, author has full control.

- **Author-issued preservation license** — Authors can attach a preservation license to their works: "any instance may mirror this work under these terms." Simplifies cross-posting and caching decisions. Human-readable and machine-readable.

- **Explicit "please preserve me" opt-in** — Authors can mark works as "preservation preferred" — signals to instances to prioritize these for full mirroring. Combined with cross-post rewards, incentivizes durable-first authoring.

- **"Deletion is permanent" affirmation** — When author deletes a work, prompt: "This work exists on N archives. Deletion here does not remove it from other archives. Cross-posted copies survive." Prevents false sense of erasure.

- **Author-controlled cross-post exclusivity** — Author can mark a work as "Lorehaven-exclusive for N months" — Lorehaven adapters don't offer cross-posting during that window. Respects launch strategies while preserving eventual portability.

- **Consent revocation cascade** — Author revokes consent for their work to be on this instance. Lorehaven removes it, notifies preservation partners, updates federation records. Partners may keep their copies (their own consent framework applies) but the removal chain is traceable.

- **Orphaned work protocol** — Author is gone (deleted account, dead, unreachable). Work exists. Community decides preservation posture with elevated care: default is preserve-as-is, no edits, no monetization, but no active promotion either. Special "orphaned but preserved" status.

- **Estate/heir designation** — Authors can designate a Lorehaven user as their literary heir: on account inactivity confirmation or verified death, this user gains rights to manage the author's works (continue publication, grant permissions, transfer to another archive). Fandom-native literary estate.

- **Author gets analytics on cross-posted mirrors** — Even on external mirrors, if the target archive exposes APIs, Lorehaven fetches reader counts, comment counts, and shows the author unified analytics across all mirrors. "Your work has been read 12k times on Lorehaven, 47k on AO3, 3k on FFN."

- **Author-visible preservation coverage map** — Author dashboard shows a coverage map of their bibliography: which works are on how many archives, which are fragile. Nudges cross-posting without shaming.

- **Author's cross-post consent as first-class metadata** — Every work carries author's declared cross-post preferences: which archives are approved, which forbidden, which require case-by-case permission. Adapters and reader-initiated cross-post requests honor these.

---

## Rewards, Gamification, and Anti-Gaming

- **Preservation credits are separate from content credits** — Different bucket, different cap, different display. Preservation is preservation; reading and writing are their own rewards. Prevents "I imported 500 works from a dead archive" from dominating an author's credit balance.

- **Preservation credits decay if coverage decays** — If an author earns preservation credits for having a work on 3 archives, and then one of those archives dies bringing them to 2, they don't lose the earned credits, but future credit earnings from that work are recalibrated. Ongoing coverage matters, not one-time acts.

- **Preservation quality multiplier** — Cross-posting to well-established, high-reputation archives earns more than cross-posting to a random tiny site. Reputation of the target archive factors in. Prevents credit farming via junk archives.

- **Verified archive registry** — Lorehaven maintains (or federates) a list of recognized preservation-worthy archives with reputation scores. Community-maintained, admin-adjudicated for disputes. Only cross-posts to verified archives earn full preservation credit.

- **Weekly and lifetime preservation caps** — Even preservation credits are capped weekly and monthly. A prolific back-catalog author cross-posting their 200 works can't dump all credits in one week; earnings spread over time.

- **Anti-mutual-cross-post-farming** — Two authors can't earn credit by cross-posting each other's works. Automated detection of tight mutual patterns; system flags and requires manual review.

- **Anti-self-mirror** — Cross-posting to an archive you own doesn't count. Detected via account linkage where possible; declared by author where automatic detection fails. Self-mirroring is preservation but not *distributed* preservation.

- **Preservation-first badge as opt-in** — Authors can display "Preservation-first author" badge on profile. Signals they prioritize work survival. Visible community signal; not enforced.

- **Cross-post effort recognition, not just count** — If cross-posting to Archive X is technically easy (Lorehaven has a one-click adapter), reward is smaller. If it's hard (author had to manually format), reward is larger. Rewards actual preservation effort, not adapter maturity.

- **Preservation contribution as trust signal input** — Long-term preservation contribution (measured by cross-post history, cache proposals, dying-archive rescue participation) is one input into trust level review. Not the only input; not automatic. But sustained preservation work is a form of vetted-conduct signal.

- **Preservation leaderboards, per-window only** — Weekly and monthly "most works preserved" boards. Never all-time (aligns with §9.7 principle). Rotates focus, prevents single dominant preservationist.

- **Fandom-scoped preservation recognition** — "Top preservationist for Harry Potter this month" is more meaningful than instance-wide totals. Distributes recognition, lets small-fandom preservationists shine.

---

## Federation, Instance Coordination, and Discovery

- **Instance directory with preservation posture** — Public directory of Lorehaven instances, filterable by preservation stance: "show me full-mirror preservation-focused instances," "show me catalog-only curated instances." Helps operators find partners; helps users find their kind of instance.

- ✓ Instance discovery with filterable configuration to choose exactly the instance you would like most. Should include extra filterable aggregates like weekly/monthly active users, cached works number, metadata works number...

- **Cross-instance search for preserved works** — When a user searches on Instance A and no results, optionally federate the search to trusted partner instances. If Instance B has it, present as "available at partner instance B."

- ✓ **Distributed catalog** — Every instance publishes its work catalog (metadata) as a federated document. Other instances can index catalogs for cross-instance discovery. Zero-cost distributed discovery layer over any storage mode.

- ✓ Configuration to auto preserve works on other instances with low preservation (off by default).

- **Preservation gap analysis across the federation** — When federated: "these works only exist on Instance X, which has low uptime this quarter. Consider preservation partnership." Community-visible gaps drive coordinated action.

- **Instance-level preservation manifesto** — Each instance publishes a preservation manifesto: what we preserve, how, under what conditions, how to request removal, how to request preservation. Public-facing document, honest, versioned.

- ✓ Instance constitution, allow users with TL≥N to vote on creation/edit or admin to create/edit, instance rules, preservation rules, and more.

- **Federation-wide preservation index** — Aggregate across all federated instances: how many works exist total, how many are preserved on ≥3 instances, where are the gaps. Fandom-wide preservation health dashboard.

- **Preservation partnership treaties** — Formal machine-readable agreement between instances: "we mutually mirror public works from these fandoms," "we rescue-import from each other on shutdown," "we honor each other's author-consent decisions." Reduces ad-hoc negotiation.

- **Dead-instance detection and rescue coordination** — Federation protocol includes health signals. If an instance goes offline for N days, partner instances coordinate: who preserves what? Who notifies users? Standard failover playbook.

- **Cross-instance author identity** — Author on Instance A can prove identity on Instance B (via cryptographic signature or federation protocol). Enables consent decisions to flow across instances: revoke on A, revoke everywhere the author holds identity.

- **Federated preservation batch coordination** — Multiple instances participating in a preservation batch (dying archive) coordinate scope: A takes fandoms X,Y; B takes fandoms Z,W; C takes translations. Avoids duplicated effort during time-critical rescue.

- **"Home instance" designation for a work** — Federated concept: each work has a home instance where it originates. Mirrors elsewhere acknowledge this. Home instance is authoritative for edits, deletions, author communications. Simplifies conflict resolution.

---

## Reader Experience Around Preservation

- **Preservation reading list** — Reader can maintain a "preservation priority" list: works they want to ensure they can still access if sources die. Automatically triggers appropriate caching or offline export based on their trust level and instance policy.

- **Offline export of preservation priorities** — Users can export their preservation priority list as an EPUB bundle or archive package. If the world ends, they still have their favorite fics. Personal preservation empowerment.

- **"This work might disappear" reader warning** — Works hosted only on unhealthy sources get a subtle warning: "This work exists only on [source], which has been unreliable. Consider bookmarking a backup or exporting." Empowers readers to preserve.

- **Reader-side preservation via PWA** — PWA caches works the reader has read or bookmarked. If the source dies and the instance's cached copy is inaccessible, the reader still has their personal copy. Not shareable, but preserves their reading.

- **"Report source down" reader action** — Reader encounters a link-out that 404s. One-tap report. Signals to instance that this source is failing. Triggers preservation escalation.

- **Reader preservation contributions visible on profile** — "This reader has contributed to preserving 47 works by supporting rescue imports." Opt-in visibility. Recognition for community preservation participation without competitive gamification.

- **"Where else can I read this" widget on work page** — Every work page lists all known mirrors with health indicators. Reader chooses where to read. Lorehaven doesn't hoard traffic; reader gets best availability.

- **Reader-facing quality signals for mirrors** — Different mirrors of the same work may have different quality (formatting, image inclusion, chapter completeness). Show reader which mirror is highest fidelity, most complete, most recently updated by author.

---

## Technical and Operational Ideas

- ✓ **Content-addressable storage for bodies** — Cached bodies stored by hash. If the same work exists on multiple sources with identical content, single storage. Deduplication across the federation potentially.

- ✓ **Deduplication signature** — Fuzzy content hash that identifies "this is the same work with minor differences" across sources. Enables coverage counting even when the mirrors have small variations (source-specific formatting, note additions).

- ✓ **Streaming import for large batches** — Preservation batches from dying archives may be huge. Instead of "job started, come back in 3 days," provide live streaming progress: works imported per minute, estimated completion, current fandom being processed.

- ✓ **Import checkpointing** — Long imports can resume from checkpoints if interrupted. Critical for rescue imports from dying sources where the source may go fully dark mid-import.

- ✓ **Parallel adapter fetching with source-side rate limits** — During preservation batch, multiple adapters may fetch from the same source. Coordinate to respect source rate limits collectively, not per-adapter.

- **Fetch queue prioritization by preservation urgency** — Fetch queue orders: emergency-preservation > user-initiated > author-imported > scheduled > background. Ensures dying-source fetches beat routine background work.

- ✓ **Robots.txt cache with source-declared TTL** — Cache the robots.txt per source with TTL from HTTP headers. Refresh policy: on TTL expiry, on user-triggered import, and on adapter-declared "sensitive operation." Avoids re-fetching robots.txt for every fetch.

- ✓ **Fetch fingerprinting for good citizenship** — Include a distinctive User-Agent that identifies as Lorehaven with instance URL, allowing source administrators to identify and contact Lorehaven if there's a problem. Traceable, not stealth.

- **Voluntary source liaison protocol** — Source administrators can opt into a liaison relationship with Lorehaven: they get a contact channel for concerns, quicker adaptor updates when they change site structure, and can request exclusion or crawl-delay changes. Formal good-neighbor relationship.

- **Preservation snapshot API** — Instance exposes API for other instances to request preservation snapshots of specific fandoms or authors. Enables federation-wide preservation coordination programmatically.

- **Storage tier automation** — Rarely-read cached bodies automatically migrate to cheaper cold storage. Frequently-read stays in hot storage. Cost management for full-mirror instances.

- **Bandwidth-aware fetching** — Adapter respects source's apparent capacity (response times, error rates). Backs off if source seems stressed. Good citizen behavior regardless of robots.txt stance.

---

## Governance and Community Dimensions

- **Preservation council role** — Distinct from moderation council: a group of TL≥N users focused on preservation decisions, coordinating rescue imports, evaluating cache proposals, maintaining archive relationships. Separate expertise domain.

- **Preservation policy referendum** — Instance-level policy changes affecting preservation posture (moving from catalog to hybrid, changing cache access trust levels) require community consultation, not just admin decree. Preservation posture is a covenant with users.

- **Instance charter includes preservation stance** — When a user joins an instance, the "what you're joining" summary includes preservation posture: "This instance is metadata-only; works you read here mostly live elsewhere. This instance may go down without local content loss because we don't host bodies."

- **Public preservation ledger** — Every preservation action logged in a public ledger: this batch imported at this time from this source under these grounds. Transparency about what has been preserved and why.

- **Author-facing preservation appeal** — If preservation decisions affect an author's work (caching, cross-posting, rescue imports), the author has a clear appeal path with a stated response window. Preservation urgency doesn't erase author rights.

- **Community consultation on rescue import scope** — Before a major rescue import from a dying archive, community consultation: which works to prioritize? Which to skip? How much of our storage budget to commit? Preservation is a community act.

- **Preservation transparency report** — Annual report: how many works preserved, how many cross-posted, how many rescued, how much storage used, what was declined and why. Institutional accountability.

- **DMCA and takedown handling for preserved content** — Explicit workflow for preserved content: takedown notice → immediate honor → preservation-batch flag (does this affect the whole batch?) → notify author or rights holder → offer appeal → transparent log. Preservation doesn't override legal takedowns.

- **Preservation ethics review board** — For contested preservation decisions (author unreachable, unclear rights, contested claims), independent review board of TL≥N users adjudicates. Slow, careful, transparent. For hard cases only.

- **Cultural preservation policy** — Some works have cultural significance beyond their author's individual rights (e.g., fandom-defining works whose authors have vanished). Instance may adopt cultural preservation policy for such works with quorum + admin approval + public announcement.

---

## Legal, Ethical, and Positioning Ideas

- **Jurisdiction-aware policy defaults** — Instance operator declares primary jurisdiction (informational, not enforced). Policy defaults adjust: EU jurisdictions get more conservative metadata-scraping defaults due to database rights; US gets fair-use-friendlier defaults. Operator can override with acknowledgment.

- **Legal disclosure page** — Instance publishes clear legal disclosures: what's our position on scraping, what's our takedown process, who to contact, what jurisdictions we operate under. Legibility for authors, readers, and rights holders.

- **DMCA agent designation** — Standard support for designating a DMCA agent (US) or equivalent (other jurisdictions). Preservation-heavy instances especially benefit from formal takedown processes.

- **Preservation-liability insurance model** — Ambitious: consortium of preservation-focused instances pools resources for legal defense of good-faith preservation activity. Shared infrastructure for shared risk.

- **"We preserve, we don't monetize" positioning** — Preservation-focused instances that don't monetize preserved content have stronger legal footing and community credibility. Explicit stance: "cached bodies are for preservation, not for our business model."

- **License auto-detection** — Adapters detect declared licenses on imported content (CC, all-rights-reserved, etc.). Metadata records the detected license. Preservation and cross-post decisions factor license into policy.

- **Author permission archiving** — When author grants preservation permission (via any channel: comment, email, in-platform), the permission is archived with timestamp and context. Provable consent trail for future questions.

- **Preservation as public good framing** — Explicit positioning: preservation of transformative works is a cultural public good. Instance operators aren't competing with sources; they're insurance against source loss. Framing that appeals to fandom values.

- **Anti-hoarding pledge** — Preservation-focused instances publicly pledge to always allow author revocation, to never monetize preserved content, to always disclose preservation posture, to prefer distributed preservation over centralized hoarding. Voluntary standard.

- **"Preservationist's Oath" for cache-decision voters** — TL≥N users participating in cache decisions accept a lightweight ethical framework: preserve fic, honor authors, minimize harm, transparent decisions, revocable when challenged. Not legal; cultural.

---

## Meta and Discovery Around Preservation

- **Preservation-focused discovery feed** — Optional widget: "works at risk that need preservation." Surfaces content on unhealthy sources without local mirrors. Community can rally around specific rescue needs.

- **"Save this fic" one-tap action** — Reader encounters a work they love that's only on one archive. One tap: adds it to the preservation queue for community consideration. Lowers activation energy for reader-initiated preservation.

- **Fandom preservation report** — Per-fandom dashboard: how well is this fandom preserved? What percentage of works exist on ≥3 archives? Which authors are single-source? Community-visible fandom health.

- **Preservation-priority tag** — Community can tag works as preservation-priority (canonical to fandom, historically important, at-risk source). Preservation council considers tagged works first in rescue operations.

- **Historical preservation memorials** — Instance maintains a memorial page for archives that have died and works preserved from them. "In memory of Archive X (2005-2024). N works preserved here." Honors the sources that came before.

- **"Preserved from X" badge on works** — Works rescued from dying archives carry a badge: "Preserved from Archive X, 2024." Historical context; community recognition of rescue effort.

- **Preservation timeline for a work** — Every preserved work has a timeline: originally published on X in 2015, imported to Y in 2020, cross-posted to Z in 2022, rescued from X in 2024. Full lifecycle visible.

- **Discovery feed for restored works** — "Recently rescued works you might have missed" — surfaces preservation batch imports so readers can rediscover works from archives they'd forgotten.

- **Cross-archive reading recommendation** — "Readers of works on this instance also enjoy works on partner Instance B" — federated recommendation surfaces content across the preservation network without merging catalogs.

- **Preservation stories** — Curated feature: "How we preserved 12,000 fics from Archive X" — narrative essays about preservation operations. Community folklore around preservation acts. Attracts preservation-minded users.

---

## Speculative and Long-Term

- **Blockchain-anchored preservation proof** — For each preserved work, publish a cryptographic proof (hash + timestamp) to a public chain. Provides verifiable "this work existed on this date" evidence independent of any single archive. Overkill for most cases; useful for legally contested works.

- **IPFS-backed distributed body storage** — Cached bodies stored on IPFS with instance running an IPFS node. Distributed by design; multiple instances can serve the same content without duplicating storage. Federation meets content-addressable storage.

- **Preservation NFT for authors** — Fringe idea: authors can mint a "canonical edition" NFT of their work, providing provenance beyond archive-level preservation. Not for monetization; for authenticity across a fragmented archive landscape.

- **Time-capsule works** — Author publishes a work with instructions: preserve for N years, then release under CC. Preservation network holds it, respects the terms. Long-term literary planning.

- **Digital literary estate integration** — Formal integration with services managing digital legacy after death. If author is verified deceased, their preservation instructions execute automatically.

- **Preservation-focused instance federation** — Formal federation of preservation-focused instances with shared protocols, shared reputation systems, shared rescue coordination, shared legal defense fund. "The Preservation Union" as a subset of the Lorehaven federation.

- **Cross-platform preservation standards** — Lorehaven contributes to and adopts open standards for preservation metadata, cross-post declarations, and archive federation. Not just Lorehaven-to-Lorehaven; standards that other platforms can implement.

- **Preservation-as-a-service for external archives** — External small archives (fandom-specific sites, personal collections) can pay a small fee to be included in Lorehaven preservation guarantees. Distributed archive-of-last-resort as a service.

- **Wayback Machine integration** — For metadata-only works, Lorehaven optionally links to Wayback Machine snapshots. If source dies and Lorehaven didn't cache, Wayback might have. Layered preservation.

- **Community-owned preservation trust** — Long-term: a nonprofit trust that owns preservation infrastructure independent of any single instance. Instances can hand off preservation duties to the trust. Ensures preservation outlives individual operators.

---

### Need fleshing out

- By default each lorehaven instance only keeps percentage of total works that fit instance theme based on configurable percentage.
- Lorehaven instances coordinate by default to preserve works so they are hosted in at least 3 instances with good uptime at any time.
- Allow author to crosspost/takedown work across all instances and supported sites at once supplying credentials for each source.
- Invite graph visible by admin and/or TL≥N users.
- Web of trust.
- Proof of humanity as a journey not as verified badge.
