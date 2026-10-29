# Monetisation — darkhold

Programme spec: `/home/alvaro/secondbrain/10-Projects/2026-10-29-estate-monetization-spec.md`
Phases and order: `/home/alvaro/secondbrain/10-Projects/2026-10-29-monetization-roadmap.md`

## This product

Membership first, adult display second (ExoClick). Never a banner inside the chapter reader.

## Where the code goes

* `frontend/src/App.svelte — SPA shell where a slot component mounts`
* `frontend/src/routes/ — browse, work page, chapter view`
* `the NSFW gate — no slot may render above it`
* `darkhold.toml — add MONETIZATION=off|pilot|on (default off)`

## Known gap

SPA: pageviews need a POST /__pv beacon before RPM is measured rather than estimated.

## Rules that are not negotiable (full list in the programme spec)

* No ad before an age gate, on an error page, or on any `/api/*` path — the estate's no-trap gate checks
  the last one.
* Every slot has a reserved height; CLS stays at 0. A banner that pushes text is worse than no banner.
* Consent before any personalised ad; non-personalised is the default, not the fallback.
* Adult inventory uses adult networks only. A mainstream network on this surface ends that account.

## Measurement

`~/estate-gates/pageviews.sh` on thinkcentre reports this host's pageviews; `rpm.py` prints RPM against
`revenue.csv`. Nothing here counts as measured until both are non-zero for a full week.
