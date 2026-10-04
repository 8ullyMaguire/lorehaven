<script lang="ts">
  /**
   * Surprise Me: one work from outside your taste profile (item 7, spec §16.10).
   *
   * The store is `surprise_me_work` in `crates/db/src/discovery.rs` and the endpoint is
   * `GET /api/v1/discovery/surprise-me`.
   *
   * **Unlike Blind Date, this page shows the work, and the difference is the feature.**
   * Blind Date deliberately renders nothing but a link, because a blind date you can see the
   * shape of is not blind. Surprise Me inverts its taste profile — that is what "inverts
   * usual weighting" means in §16.10 — and a reader cannot judge whether a departure was a
   * departure unless they know what they usually read. So the title and summary are shown,
   * and the copy says which way the pick moved.
   *
   * **The two empty states are kept apart, which is the one thing this page must not get
   * wrong.** The endpoint returns 200 with `work: null` both when the public catalogue is
   * empty and when the reader's profile covers everything worth showing. Those are
   * different messages: the first is about the instance, the second is about the reader, and
   * conflating them tells someone with strong taste that the button is broken. `profileEmpty`
   * separates them, and it is why the client type carries the flag at all.
   *
   * The exclusion is not a negative score — see the store's header for why that distinction
   * is the whole difference between a feature and an inverted ranking. Nothing here needs to
   * know that; it needs to not claim anything the query does not guarantee.
   */
  import { fetchSurpriseMe, type SurpriseMeResponse } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  let result = $state<SurpriseMeResponse | null>(null);
  let error = $state<unknown>(null);
  let loading = $state(true);

  /**
   * The work URL, or null when there is no work.
   *
   * A `$derived` rather than a template expression so the link and the href cannot drift
   * apart, and so the null case is handled once. `{#if result.work}` narrows at runtime but
   * not in the type checker, which is why this exists.
   */
  let workUrl = $derived(
    result?.work ? `/works/${encodeURIComponent(result.work.workId)}` : null,
  );

  async function load() {
    loading = true;
    error = null;
    try {
      result = await fetchSurpriseMe();
    } catch (caught) {
      // Cleared so a retry that fails does not leave the previous pick on screen beside an
      // error — which reads as "this is a fresh departure" and is the one way a stale card
      // could mislead about what the button just did.
      result = null;
      error = caught;
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void load();
  });
</script>

<header class="page-header">
  <h1>Surprise Me</h1>
  <p class="lede">
    One work from outside what you usually read.
  </p>
</header>

<!--
  `error` is checked FIRST and exclusively. It used to sit in its own `{#if}` ABOVE this
  chain, so a failed request rendered the error banner AND then fell through to an empty-state
  branch — telling the reader "everything here shares a tag with your profile" underneath a 404.
  A specific confident wrong claim printed under an error is worse than no message at all.
-->
{#if error}
  <ErrorSummary {error} onretry={load} />
{:else if loading}
  <Skeleton lines={3} label="Looking away from your profile" />
{:else if result && result.work && workUrl}
  <!--
    The heading says which way the pick moved, because "surprise" is only meaningful
    relative to what the reader would otherwise have been served.
  -->
  <article class="card">
    <h2>
      <a href={workUrl} onclick={(event) => handleLinkClick(event, workUrl)}>
        {result.work.title}
      </a>
    </h2>
    {#if result.work.summary}
      <p class="summary">{result.work.summary}</p>
    {/if}
    <p class="meta">
      {#if result.profileEmpty}
        You have no taste profile yet, so this was picked without one.
      {:else}
        Picked from outside the tags your reading has weighted.
      {/if}
    </p>
  </article>
{:else if result && result.profileEmpty}
  <!--
    An empty catalogue, for a reader who never expressed a preference. Still the instance's
    state, so the wording is about the catalogue — but the reader is not told the feature
    worked and found nothing.
  -->
  <p class="empty">
    Nothing to offer yet. Surprise Me skips anything sharing a tag with your profile, and
    there is no public work published on this instance to choose from.
  </p>
{:else}
  <!--
    A reader WITH a profile, and nothing left outside it. This is a real state on a small
    instance and it is about the reader's taste, not the catalogue — saying "the catalogue is
    empty" here would be false and would hide why the button returns nothing.
  -->
  <p class="empty">
    Everything here shares a tag with your profile. Surprise Me deliberately steps outside
    it, so it has nothing to offer until there is work tagged differently — the profile is
    what it is avoiding.
  </p>
{/if}

<style>
  .card {
    max-inline-size: 60ch;
  }

  .card h2 {
    margin-block-end: var(--space-2);
  }

  .summary {
    margin-block-end: var(--space-3);
  }

  .meta {
    opacity: 0.7;
    font-size: 0.9rem;
  }

  .empty {
    max-inline-size: 46ch;
  }
</style>
