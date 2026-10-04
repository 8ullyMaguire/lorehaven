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
   * "Similar works" — idea 33 of the 100-idea audit.
   *
   * This is the highest-intent discovery moment on the site: a reader has just finished
   * something, and this is the only place they are still looking for something to read.
   *
   * ## Why the label says "shares" and not "similar"
   *
   * The score is a weighted Jaccard over tag sets, computed on the server with an
   * honesty floor (`FILTER_FLOOR`, 0.15): below it, "similar" is a claim the reader can
   * check and find false. A match on the fandom is *evidence*; "similar" is a *claim*.
   * So the aria text names the evidence — "shares the fandom Harry Potter" — and the
   * visible percentage gives the reader the number to weigh it themselves.
   *
   * ## The empty case
   *
   * Empty is the CORRECT answer when a work has fewer than two tags or nothing clears
   * the floor. This renders nothing at all in that case: an empty rail headed "Similar
   * works" teaches the reader that the site thinks nothing relates to what they just
   * read, which is a different and wrong claim.
   */
  interface Props {
    /** Best match first, already scored by the server. */
    works?: SurfaceWork[];
    loading?: boolean;
    /** The subject work's title, used to make the heading concrete. */
    subjectTitle?: string;
    onopen?: (id: string) => void;
  }

  let { works = [], loading = false, subjectTitle, onopen }: Props = $props();

  let visible = $derived(loading || works.length > 0);

  /** "Similar works", or "Similar to <title>" when the caller knows the title. */
  let heading = $derived(subjectTitle ? `Similar to “${subjectTitle}”` : 'Similar works');

  /**
   * The score as a whole-number percentage.
   *
   * Only reached when `similarity` is present, because the server omits the field on
   * rows it did not score. Rendering 0% for a missing score would claim a measurement
   * nobody took.
   */
  function percent(work: SurfaceWork): string | undefined {
    if (work.similarity === undefined) return undefined;
    return `${Math.round(work.similarity * 100)}%`;
  }
</script>

{#if visible}
  <section class="rail" aria-labelledby="similar-works-heading">
    <h2 id="similar-works-heading">{heading}</h2>
    <p class="hint">Ranked by shared tags, strongest evidence first.</p>
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
          {#if percent(work)}
            <span class="score" aria-label="{percent(work)} tag overlap">
              {percent(work)}
            </span>
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

  li {
    display: inline-flex;
    align-items: baseline;
    gap: var(--space-2);
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

  .score {
    color: var(--color-muted);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
  }

  .length {
    margin-left: 0.5rem;
    font-size: 0.8rem;
    opacity: 0.7;
    white-space: nowrap;
  }
</style>