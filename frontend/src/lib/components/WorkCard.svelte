<script lang="ts">
  import MetadataChip from './MetadataChip.svelte';

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
  }

  interface Props {
    work: WorkSummary;
    onopen?: (id: string) => void;
  }

  let { work, onopen }: Props = $props();

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
</script>

<article class="card">
  {#if work.bookmarked}
    <!-- The bookmark-ribbon motif from the theme, not a colour-coded badge. -->
    <span class="ribbon" aria-label="Saved to your library"></span>
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

  {#if work.summary}
    <p class="summary">{work.summary}</p>
  {/if}

  <div class="chips">
    <MetadataChip label={COMPLETION_LABEL[work.completion]} tone={work.completion === 'complete' ? 'primary' : 'neutral'} />
    <MetadataChip label={RATING_LABEL[work.rating]} />
    {#if work.chapters !== undefined}
      <MetadataChip label="Chapters" value={work.chapters} />
    {/if}
    {#if formatWords(work.wordCount)}
      <MetadataChip label={formatWords(work.wordCount)!} />
    {/if}
  </div>

  {#if work.mainCharacters?.length || work.centralRelationships?.length}
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

  .ribbon {
    position: absolute;
    top: 0;
    right: var(--space-5);
    width: 0.75rem;
    height: 1.5rem;
    background: var(--color-accent);
    clip-path: polygon(0 0, 100% 0, 100% 100%, 50% 78%, 0 100%);
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
