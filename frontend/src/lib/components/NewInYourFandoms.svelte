<script module lang="ts">
  /**
   * Exported from the module context so callers and tests can type their fixtures
   * against the real shape. An `export interface` in the instance `<script>` is not
   * importable in Svelte 5.
   */
  import type { SurfaceWork } from '../api';
  import { readingLength } from '../readingLength';
  export type { SurfaceWork };
</script>

<script lang="ts">
  /**
   * "New in your fandoms" — idea 14 of the 100-idea audit.
   *
   * The failure this fixes: a reader who bookmarked three Harry Potter works opens
   * Discover and sees nothing new. This section answers that with one join.
   *
   * ## The empty case is the whole design
   *
   * With no public bookmarks there are no fandoms, and **this component renders
   * nothing** — not a heading over an empty list, and not a fallback to all recent
   * works. A section that changes subject when it has no data is a section nobody can
   * learn to read: the reader learns "new in your fandoms" and then sees something else.
   *
   * That is why the fetching lives in the caller and this component only decides
   * whether there is anything to draw. `{#if works.length}` is load-bearing.
   */
  interface Props {
    /** Newest first, already filtered by the server. */
    works?: SurfaceWork[];
    /** True while the request is in flight; suppresses the empty case. */
    loading?: boolean;
    onopen?: (id: string) => void;
  }

  let { works = [], loading = false, onopen }: Props = $props();

  /**
   * Whether to draw anything at all.
   *
   * `loading` is in the guard on purpose: while the request is in flight `works` is
   * empty, and without it the section would flicker out and back in on every load.
   */
  let visible = $derived(loading || works.length > 0);
</script>

{#if visible}
  <section class="rail" aria-labelledby="new-in-your-fandoms-heading">
    <h2 id="new-in-your-fandoms-heading">New in your fandoms</h2>
    <p class="hint">
      Recently published in fandoms you have bookmarked. Works you already have are left out.
    </p>
    <ul>
      {#each works as work (work.id)}
        <li>
          {#if onopen}
            <button type="button" class="title" onclick={() => onopen?.(work.id)}>
              {work.title}
            </button>
          {:else}
            <span class="title">{work.title}</span>
          {/if}
          {#if readingLength(work.word_count)}
            <span class="length" data-testid="work-length">{readingLength(work.word_count)}</span>
          {/if}
        </li>
      {/each}
    </ul>
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

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-4);
  }

  .title {
    background: transparent;
    border: 0;
    padding: 0;
    font: inherit;
    color: var(--color-primary);
    cursor: pointer;
    text-align: left;
  }

  .title:hover {
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .length {
    margin-left: 0.5rem;
    font-size: 0.8rem;
    opacity: 0.7;
    white-space: nowrap;
  }
</style>