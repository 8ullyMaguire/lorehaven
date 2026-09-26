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
    recomputeTasteProfile,
    type DiscoveryItem,
    type SortValue,
  } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte.ts';
  import Button from '../lib/components/Button.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
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
