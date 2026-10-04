<script lang="ts">
  /**
   * The discovery feed: recommendations, the reader's taste profile, and the
   * controls that shape it (spec §16).
   *
   * The feed is real on first paint — either public recommendations (for an
   * anonymous reader) or personalised ones (for a signed-in reader). The taste
   * profile panel and the clear/recompute controls are signed-in only, because
   * they describe *this reader's* behaviour and the server refuses to share it.
   */
  import {
    clearTasteProfile,
    fetchDiscoveryFeed,
    fetchMostBookmarked,
    fetchNewInYourFandoms,
    recomputeTasteProfile,
    type DiscoveryItem,
    type SortValue,
    type SurfaceWork,
  } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte.ts';
  import WhyRecommended from '../lib/components/WhyRecommended.svelte';
  import Button from '../lib/components/Button.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import MostBookmarkedThisWeek from '../lib/components/MostBookmarkedThisWeek.svelte';
  import NewInYourFandoms from '../lib/components/NewInYourFandoms.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import SortControl from '../lib/components/SortControl.svelte';

  let items = $state<DiscoveryItem[]>([]);
  let error = $state<unknown>(null);
  let loading = $state(true);
  let recomputing = $state(false);
  /**
   * The reader's chosen order, or undefined when they have not chosen.
   *
   * Undefined is meaningful and must not be collapsed to a default: the server
   * resolves `query param > stored preference > surface default`, so passing
   * anything before the reader has picked would override their own stored
   * preference with the surface default on every anonymous-first paint.
   */
  let chosenSort = $state<SortValue | undefined>(undefined);

  /**
   * The two reader-surface rails: items 14 and 27 of the 100-idea audit.
   *
   * Both are loaded SEPARATELY from the feed and both are allowed to fail on their own
   * without taking the page down. The feed is the product; a leaderboard is an addition
   * to it, and a leaderboard that 500s must not blank the page a reader came for.
   *
   * Each has its own loading flag because each renders nothing when empty, and a shared
   * flag would make one section's empty response suppress the other's heading during
   * loading.
   */
  let newInFandoms = $state<SurfaceWork[]>([]);
  let fandomLoading = $state(true);
  let mostBookmarked = $state<SurfaceWork[]>([]);
  let bookmarkedLoading = $state(true);

  async function loadReaderSurface() {
    fandomLoading = true;
    try {
      // Item 14 is per-reader and the server refuses an anonymous call, so it is only
      // requested for a signed-in reader. Asking anyway would produce a 401 that lands
      // in the same `error` slot as a real failure.
      newInFandoms = session.isSignedIn ? await fetchNewInYourFandoms() : [];
    } catch {
      // Swallowed deliberately, with the section left empty. The empty case is already
      // a designed state with copy ("nothing new"), so a failure and "nothing new"
      // render identically -- which is the honest outcome for a rail nobody is
      // currently looking at.
      newInFandoms = [];
    } finally {
      fandomLoading = false;
    }

    bookmarkedLoading = true;
    try {
      // Item 27 is public and reads only public rows, so this is safe to request
      // signed out.
      mostBookmarked = await fetchMostBookmarked();
    } catch {
      mostBookmarked = [];
    } finally {
      bookmarkedLoading = false;
    }
  }

  function openWork(id: string) {
    window.location.assign(`/works/${encodeURIComponent(id)}`);
  }

  async function load() {
    loading = true;
    error = null;
    try {
      const feed = await fetchDiscoveryFeed(chosenSort);
      items = feed.items ?? [];
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  async function recompute() {
    recomputing = true;
    try {
      await recomputeTasteProfile();
      await load();
    } catch (failure) {
      error = failure;
    } finally {
      recomputing = false;
    }
  }

  async function clear() {
    try {
      await clearTasteProfile();
      items = [];
    } catch (failure) {
      error = failure;
    }
  }

  $effect(() => {
    void load();
  });

  // Re-runs when signed-in state changes, because item 14's answer depends on it.
  $effect(() => {
    void session.isSignedIn;
    void loadReaderSurface();
  });
</script>

<section class="discovery">
  <header class="discovery-header">
    <h1>Discover</h1>
    <p class="lede">
      Works the community is reading, weighted toward what you enjoy. The feed is
      private — no one else sees these recommendations.
    </p>
  </header>

  {#if error}
    <ErrorSummary {error} onretry={load} />
  {/if}

  <div class="sort-row">
    <SortControl surface="discover" onchange={(sort) => { chosenSort = sort; void load(); }} />
  </div>

  {#if session.isSignedIn}
    <div class="taste-controls">
      <Button variant="secondary" onclick={recompute} disabled={recomputing}>
        {recomputing ? 'Recomputing…' : 'Recompute taste profile'}
      </Button>
      <Button variant="quiet" onclick={clear}>Clear profile</Button>
    </div>
  {/if}

  <!--
    The two reader-surface rails sit ABOVE the feed, not below it. "New in your
    fandoms" is the returning reader's first question and the feed is the answer to a
    later one; putting the rails under a long feed would mean the reader scrolls past
    the thing they came for.
  -->
  <NewInYourFandoms works={newInFandoms} loading={fandomLoading} onopen={openWork} />
  <MostBookmarkedThisWeek works={mostBookmarked} loading={bookmarkedLoading} onopen={openWork} />

  {#if loading}
    <Skeleton lines={6} label="Loading discovery feed" />
  {:else if items.length === 0}
    <p class="empty">
      Nothing to recommend yet. Read, rate, or tag a few works and the feed will
      fill in.
    </p>
  {:else}
    <ul class="feed">
      {#each items as item}
        <li>
          <a
            href={`/works/${encodeURIComponent(item.work_id)}`}
            onclick={(event) => handleLinkClick(event, `/works/${encodeURIComponent(item.work_id)}`)}
          >
            <span class="feed-work">{item.title ?? item.work_id}{#if item.title && item.author_handle}&nbsp;·{/if}{#if item.author_handle}&nbsp;by {item.author_handle}{/if}</span>
            {#if item.title}<span class="feed-title">{item.title}</span>{/if}
          </a>
          <!--
            Item 9: why this appeared. OUTSIDE the <a>, deliberately.

            The whole row is one link, so a button inside it is a button inside an anchor:
            invalid HTML, and in practice the click navigates and the disclosure never
            opens. It also breaks cmd-click-to-open-in-new-tab for the work itself. A
            sibling is the only shape where both controls do their own job.
          -->
          <WhyRecommended slotId={item.slot_id} />
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .discovery {
    max-width: 72rem;
    margin-inline: auto;
    padding: 2rem 1rem;
  }
  .sort-row {
    margin-block: 1rem;
  }
  .discovery-header h1 {
    margin-bottom: 0.25rem;
  }
  .lede {
    color: var(--text-muted);
  }
  .taste-controls {
    display: flex;
    gap: 0.5rem;
    margin-block: 1.5rem;
  }
  .empty {
    padding: 2rem;
    text-align: center;
    background: var(--surface-muted);
    border-radius: 0.5rem;
    color: var(--text-muted);
  }
  .feed li {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .feed {
    list-style: none;
    padding: 0;
    margin: 0;
    display: grid;
    gap: 0.75rem;
  }
  .feed a {
    display: flex;
    flex-direction: column;
    padding: 1rem;
    border: 1px solid var(--border);
    border-radius: 0.5rem;
    text-decoration: none;
    color: inherit;
  }
  .feed a:hover {
    background: var(--surface-hover);
  }
  .feed-work {
    font-family: var(--font-mono);
    font-size: 0.875rem;
    color: var(--text-muted);
  }
  .feed-title {
    font-weight: 600;
    margin-top: 0.25rem;
  }
</style>
