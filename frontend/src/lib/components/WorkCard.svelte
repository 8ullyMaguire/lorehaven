<script module lang="ts">
  /**
   * Exported from the module context so callers and tests can type their
   * fixtures against the real shape. An `export interface` in the instance
   * `<script>` is not importable in Svelte 5.
   */
  export interface WorkSummary {
    id: string;
    title: string;
    authorDisplayName: string;
    summary?: string;
    completion: 'in_progress' | 'complete' | 'hiatus' | 'abandoned';
    rating: 'general' | 'teen' | 'mature' | 'explicit';
    wordCount?: number;
    chapters?: number;
    mainCharacters?: string[];
    centralRelationships?: string[];
    /** Whether the reader has saved this work. */
    bookmarked?: boolean;
    /** Public engagement counts, when the owner permits them. */
    metrics?: import('../api').WorkMetricsView | null;
  }
</script>

<script lang="ts">
  /**
   * A work, in one of three densities.
   *
   * The variants exist because the same work is shown in places that want
   * different amounts of it:
   *
   *  * `full` — a work's own page or a single result: everything known.
   *  * `row` — a long list the reader scans: title, byline, facts, actions.
   *  * `compact` — a grid of many: title, byline, and the state markers, with
   *    the summary clamped away.
   *
   * All three are the same component rather than three components because the
   * *content* is identical and only the arrangement differs; three copies would
   * be three places for a chip to go missing. The library's cards also carry
   * what only a library has — a reading status, the reader's own tags, the
   * shelves a work sits on — and those are drawn in every variant, because a
   * marker that disappears at a smaller size is a marker the reader will think
   * they lost.
   */
  import MetadataChip from './MetadataChip.svelte';
  // `WorkSummary` is declared in this file's module <script>, which the
  // instance script can see without importing it back from itself.

  /** How much of a work to draw. */
  export type WorkCardVariant = 'full' | 'row' | 'compact';

  interface Props {
    work: WorkSummary;
    onopen?: (id: string) => void;
    /** How much of the work to draw. Defaults to `full`. */
    variant?: WorkCardVariant;
    /** The reader's reading status, already worded for display. */
    readingStatus?: string;
    /** The reader's own tags on this work. */
    tags?: string[];
    /** The shelves the work sits on. */
    shelves?: string[];
    /** Whether this card can be picked in a batch. */
    selectable?: boolean;
    /** Whether it is currently picked. */
    selected?: boolean;
    /** Called when it is picked or unpicked. */
    onselect?: (id: string, selected: boolean) => void;
    /** Actions, drawn in the footer. */
    actions?: import('svelte').Snippet;
  }

  let {
    work,
    onopen,
    variant = 'full',
    readingStatus,
    tags = [],
    shelves = [],
    selectable = false,
    selected = false,
    onselect,
    actions,
  }: Props = $props();

  const COMPLETION_LABEL: Record<WorkSummary['completion'], string> = {
    in_progress: 'In progress',
    complete: 'Complete',
    hiatus: 'On hiatus',
    abandoned: 'Abandoned',
  };

  const RATING_LABEL: Record<WorkSummary['rating'], string> = {
    general: 'General',
    teen: 'Teen',
    mature: 'Mature',
    explicit: 'Explicit',
  };

  function formatWords(count?: number): string | undefined {
    if (count === undefined) return undefined;
    if (count < 1000) return `${count} words`;
    return `${(count / 1000).toFixed(count < 10000 ? 1 : 0)}k words`;
  }

  /** Format a metric count for display: 1.2k, 3.4k, etc. */
  function formatCount(n: number): string {
    if (n < 1000) return `${n}`;
    return `${(n / 1000).toFixed(n < 10000 ? 1 : 0)}k`;
  }

  /** Whether the metric bar is worth drawing (any non-zero count). */
  let hasMetrics = $derived(
    work.metrics !== null && work.metrics !== undefined
      && (work.metrics.views > 0 || work.metrics.kudos > 0 || work.metrics.reactions > 0
        || work.metrics.bookmarks > 0 || work.metrics.complete_reads > 0
        || work.metrics.collection_adds > 0 || work.metrics.reviews > 0),
  );

  /** Whether anything only a library knows is worth drawing. */
  let hasLibraryFacts = $derived(
    Boolean(readingStatus) || tags.length > 0 || shelves.length > 0,
  );

  function toggle() {
    onselect?.(work.id, !selected);
  }
</script>

<article class="card {variant}" class:selected>
  {#if work.bookmarked}
    <!-- The bookmark-ribbon motif from the theme, not a colour-coded badge. -->
    <span class="ribbon" aria-label="Saved to your library"></span>
  {/if}

  {#if selectable}
    <!-- A real checkbox rather than a card-wide click: the card also holds links
         and buttons, and making the whole thing a control would swallow them.
         The label is the work's title, so the control has a name of its own. -->
    <label class="pick">
      <input type="checkbox" checked={selected} onchange={toggle} />
      <span class="pick-label">Select {work.title}</span>
    </label>
  {/if}

  <header>
    <h3>
      {#if onopen}
        <button type="button" class="title" onclick={() => onopen?.(work.id)}>{work.title}</button>
      {:else}
        {work.title}
      {/if}
    </h3>
    <p class="byline">by {work.authorDisplayName}</p>
  </header>

  {#if variant === 'full' && work.summary}
    <p class="summary">{work.summary}</p>
  {/if}

  <div class="chips">
    <MetadataChip label={COMPLETION_LABEL[work.completion]} tone={work.completion === 'complete' ? 'primary' : 'neutral'} />
    {#if variant !== 'compact'}
      <MetadataChip label={RATING_LABEL[work.rating]} />
    {/if}
    {#if work.chapters !== undefined}
      <MetadataChip label="Chapters" value={work.chapters} />
    {/if}
    {#if formatWords(work.wordCount)}
      <MetadataChip label={formatWords(work.wordCount)!} />
    {/if}
  </div>

  {#if hasMetrics}
    <!-- Engagement counts, gated by the owner's public-ratings preference.
         Drawn in every variant: a number the reader relies on should not
         vanish at grid density. -->
    <ul class="metrics" aria-label="Engagement">
      <li>{formatCount(work.metrics!.views)} views</li>
      <li>{formatCount(work.metrics!.kudos)} kudos</li>
      {#if variant === 'full'}
        <li>{formatCount(work.metrics!.reactions)} reactions</li>
        <li>{formatCount(work.metrics!.complete_reads)} finished</li>
        <li>{formatCount(work.metrics!.bookmarks)} bookmarks</li>
        <li>{formatCount(work.metrics!.collection_adds)} in collections</li>
        <li>{formatCount(work.metrics!.reviews)} reviews</li>
      {/if}
    </ul>
  {/if}

  {#if hasLibraryFacts}
    <!-- The library's own facts, drawn in every variant: a status or a tag that
         vanished at grid density would look like it had been lost. -->
    <ul class="library-facts">
      {#if readingStatus}
        <li class="status">{readingStatus}</li>
      {/if}
      {#each shelves as shelf (shelf)}
        <li class="shelf">{shelf}</li>
      {/each}
      {#each tags as tag (tag)}
        <li class="tag">{tag}</li>
      {/each}
    </ul>
  {/if}

  {#if variant === 'full' && (work.mainCharacters?.length || work.centralRelationships?.length)}
    <dl class="catalog">
      {#if work.mainCharacters?.length}
        <dt>Main characters</dt>
        <dd>{work.mainCharacters.join(', ')}</dd>
      {/if}
      {#if work.centralRelationships?.length}
        <dt>Central relationships</dt>
        <dd>{work.centralRelationships.join(', ')}</dd>
      {/if}
    </dl>
  {/if}

  {#if actions}
    <div class="actions">
      {@render actions()}
    </div>
  {/if}
</article>

<style>
  /* Text-first rows, per the theme's "a useful catalog" guidance. */
  .card {
    position: relative;
    padding: var(--space-4) var(--space-5);
    background: var(--color-surface);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-lg);
    overflow: hidden;
  }

  /* The grid density: less air, and the heading steps down a size. */
  .compact {
    padding: var(--space-3);
  }

  .compact h3 {
    font-size: var(--text-lg);
  }

  .row {
    padding: var(--space-3) var(--space-4);
  }

  .selected {
    border-color: var(--color-primary);
  }

  .ribbon {
    position: absolute;
    top: 0;
    right: var(--space-5);
    width: 0.75rem;
    height: 1.5rem;
    background: var(--color-accent);
    clip-path: polygon(0 0, 100% 0, 100% 100%, 50% 78%, 0 100%);
  }

  .pick {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }

  /* The label names the control for a screen reader; the checkbox is the mark. */
  .pick-label {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  h3 {
    margin: 0;
    font-size: var(--text-xl);
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

  .byline {
    margin: var(--space-1) 0 0;
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .summary {
    margin: var(--space-3) 0;
    color: var(--color-text);
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-top: var(--space-3);
  }

  .library-facts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    list-style: none;
    padding: 0;
    margin: var(--space-3) 0 0;
    font-size: var(--text-sm);
  }

  .library-facts li {
    padding: 0 var(--space-2);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-sm);
  }

  .metrics {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    list-style: none;
    padding: 0;
    margin: var(--space-3) 0 0;
    font-size: var(--text-sm);
    color: var(--color-muted);
  }

  .metrics li {
    padding: 0 var(--space-2);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-sm);
  }

  .status {
    color: var(--color-accent);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin-top: var(--space-3);
    align-items: center;
  }

  .catalog {
    margin: var(--space-3) 0 0;
    font-size: var(--text-sm);
    display: grid;
    grid-template-columns: auto 1fr;
    gap: var(--space-1) var(--space-3);
  }

  dt {
    color: var(--color-muted);
    font-weight: 600;
  }

  dd {
    margin: 0;
  }
</style>
