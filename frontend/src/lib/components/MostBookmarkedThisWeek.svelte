<script module lang="ts">
  /**
   * Exported from the module context so callers and tests can type their fixtures
   * against the real shape. An `export interface` in the instance `<script>` is not
   * importable in Svelte 5.
   */
  import type { SurfaceWork } from '../api';
  export type { SurfaceWork };
</script>

<script lang="ts">
  /**
   * "Most bookmarked this week" — idea 27 of the 100-idea audit.
   *
   * ## The privacy rule, and why the heading is honest about it
   *
   * This leaderboard counts **public** bookmarks only. `bookmarks.is_public` defaults
   * to false, so a leaderboard built over all bookmarks would publish, in aggregate,
   * exactly what readers chose to keep private — and it would be visible to everyone,
   * including people who do not read here at all.
   *
   * So the count that is rendered is the count of readers who made their bookmark
   * public, and the label says "public bookmarks". A number that quietly included
   * private rows would be a lie a reader could not detect, which is worse than the
   * leaderboard being smaller.
   *
   * The server enforces this (`most_bookmarked_counts_only_public_bookmarks` in
   * `crates/db/tests/reader_surface_t1.rs`). This component must not "fix" the label to
   * say simply "bookmarks" — the label is the reader-facing half of the same rule.
   */
  interface Props {
    /** Ranked by distinct public bookmarkers, best first. */
    works?: SurfaceWork[];
    loading?: boolean;
    onopen?: (id: string) => void;
  }

  // No `windowDays` prop. The first version had one and svelte-check caught it as
  // declared-but-never-read, which was right: the window is the SERVER's business --
  // it computes the bound value and owns the arithmetic -- and a component prop that
  // nothing reads is a second, divergent copy of a number that already has one home.
  // Changing the window is `fetchMostBookmarked(days)`, not a prop here.
  let { works = [], loading = false, onopen }: Props = $props();

  let visible = $derived(loading || works.length > 0);
</script>

{#if visible}
  <section class="rail" aria-labelledby="most-bookmarked-heading">
    <h2 id="most-bookmarked-heading">Most bookmarked this week</h2>
    <p class="hint">Counted from public bookmarks only. Private bookmarks are never counted.</p>
    <ol>
      {#each works as work, index (work.id)}
        <li>
          <span class="rank" aria-hidden="true">{index + 1}</span>
          {#if onopen}
            <button type="button" class="title" onclick={() => onopen?.(work.id)}>
              {work.title}
            </button>
          {:else}
            <span class="title">{work.title}</span>
          {/if}
          {#if work.recent_bookmarks !== undefined}
            <!--
              The screen-reader text names the unit, because "12" alone on a list of
              numbers is a column of nothing. It says "public bookmarks" for the same
              reason the hint does.
            -->
            <span class="count" aria-label="{work.recent_bookmarks} public bookmarks">
              {work.recent_bookmarks}
            </span>
          {/if}
        </li>
      {/each}
    </ol>
  </section>
{/if}

<style>
  .rail {
    margin-block: var(--space-6);
  }

  h2 {
    margin: 0 0 var(--space-1);
    font-size: var(--text-xl);
  }

  .hint {
    margin: 0 0 var(--space-3);
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  ol {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  li {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
  }

  .rank {
    color: var(--color-muted);
    font-size: var(--text-sm);
    /* Room for two digits, so a leaderboard that grows past nine does not reflow. */
    min-width: 1.5rem;
    text-align: right;
  }

  .title {
    background: transparent;
    border: 0;
    padding: 0;
    font: inherit;
    color: var(--color-primary);
    cursor: pointer;
    text-align: left;
    flex: 1;
  }

  .title:hover {
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .count {
    color: var(--color-muted);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
  }
</style>