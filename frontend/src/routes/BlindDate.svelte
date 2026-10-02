<script lang="ts">
  /**
   * Blind Date: one work a day, chosen without looking at your profile.
   *
   * Gap B; spec §16.1a lists Blind Date among the discovery surfaces beside Recent and
   * trending, and §16.10's blind-date pool is what feeds it. The store and endpoint live
   * in `crates/db/src/discovery.rs` and `crates/app/src/routes/discovery.rs`.
   *
   * **The page shows as little as possible, on purpose.** The endpoint returns a bare
   * work id, and this component deliberately does not fetch metadata for it: no title, no
   * author, no tags, no fandom. A blind date you can see the shape of is not blind — the
   * reader's judgement about whether they like long AUs, this author, or that trope is
   * the entire mechanism, and rendering it destroys it. What they get is a single link,
   * and the work reveals itself on the work page.
   *
   * There is deliberately no "show me another" control. The pick is deterministic in
   * (account, day), and a reroll button would make the surface a random-work generator
   * wearing a daily-feature costume.
   */
  import { fetchBlindDate } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  let workId = $state<string | null>(null);
  /**
   * The work URL, or null when there is no work.
   *
   * A `$derived` rather than a template expression so the link and the href cannot drift
   * apart, and so the null case is handled once. `{#if workId}` narrows `workId` at
   * runtime but not in the type checker, which is why this exists.
   */
  let workUrl = $derived(workId === null ? null : `/works/${encodeURIComponent(workId)}`);
  let date = $state<string>('');
  let error = $state<unknown>(null);
  let loading = $state(true);

  async function load() {
    loading = true;
    error = null;
    try {
      const response = await fetchBlindDate();
      workId = response.workId;
      date = response.date;
    } catch (caught) {
      // The work id is cleared so a retry that fails does not leave the previous day's
      // link on screen next to an error — which reads as "this is today's pick" and is
      // the one way a stale link could mislead.
      workId = null;
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
  <h1>Blind Date</h1>
  <p class="lede">
    One work a day, picked without looking at your profile.
  </p>
</header>

{#if error}
  <ErrorSummary {error} onretry={load} />
{/if}

{#if loading}
  <Skeleton lines={3} label="Choosing today's work" />
{:else if workId === null}
  <!--
    An empty catalogue is a quiet surface, not a broken one. The endpoint returns 200
    with a null id for this, so reaching here is normal -- and the wording says what to
    do about it rather than apologising.
  -->
  <p class="empty">
    Nothing to offer today. Blind Date skips works you have bookmarked and authors you
    have already read, so an empty day usually means the catalogue is still small.
  </p>
{:else}
  {#if workUrl}
    <p class="reveal">
      <a href={workUrl} onclick={(event) => handleLinkClick(event, workUrl)}>
        Open today's work
      </a>
    </p>
  {/if}
  <p class="meta">Picked {date}. One a day, and the same one tomorrow.</p>
{/if}

<style>
  .reveal {
    margin-block: 2rem;
    font-size: 1.15rem;
  }

  .meta {
    opacity: 0.7;
    font-size: 0.9rem;
  }

  .empty {
    max-inline-size: 46ch;
  }
</style>