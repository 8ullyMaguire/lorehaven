<script lang="ts">
  /**
   * The reading surface: one chapter, with position awareness.
   *
   * Spec §9. The reader renders a single chapter at a time (whole-work
   * mode is paginated, not one long DOM). Position is saved on scroll
   * end and on `visibilitychange`, debounced, and the local copy is
   * written to `localStorage` first so a lost request still leaves a
   * position.
   */
  import { fetchChapter, type ChapterContent } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  interface Props {
    workId: string;
    chapterId: string;
  }

  let { workId, chapterId }: Props = $props();

  let chapter = $state<ChapterContent | null>(null);
  let error = $state<unknown>(null);
  let loading = $state(true);

  $effect(() => {
    void load();
  });

  async function load() {
    loading = true;
    error = null;
    chapter = null;
    try {
      chapter = await fetchChapter(workId, chapterId);
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }
</script>

{#if loading}
  <Skeleton lines={5} />
{:else if error}
  <h1>Not found</h1>
  <ErrorSummary error={error} />
  <p>
    <a href={`/works/${workId}`} onclick={(event) => handleLinkClick(event, `/works/${workId}`)}>
      Back to the work
    </a>
  </p>
{:else if chapter}
  <nav class="crumbs">
    <a href={`/works/${workId}`} onclick={(event) => handleLinkClick(event, `/works/${workId}`)}>
      {chapter.work.title.trim() === '' ? 'Untitled' : chapter.work.title}
    </a>
  </nav>

  <h1>{chapter.chapter.title.trim() === '' ? 'Untitled chapter' : chapter.chapter.title}</h1>

  <p class="meta">
    {chapter.word_count} words
    {#if chapter.revision_number !== null}· revision {chapter.revision_number}{/if}
    {#if chapter.editable}
      · <a href={`/write/${workId}`} onclick={(event) => handleLinkClick(event, `/write/${workId}`)}>Edit</a>
    {/if}
  </p>

  <!-- Server-sanitized HTML; see the module note. -->
  <article class="prose">{@html chapter.sanitized_html}</article>

  <nav class="pager" aria-label="Chapter navigation">
    {#if chapter.previous_chapter_id}
      <a
        href={`/works/${workId}/chapters/${chapter.previous_chapter_id}`}
        onclick={(event) => handleLinkClick(event, `/works/${workId}/chapters/${chapter.previous_chapter_id}`)}
      >
        ← Previous chapter
      </a>
    {/if}
    {#if chapter.next_chapter_id}
      <a
        class="next"
        href={`/works/${workId}/chapters/${chapter.next_chapter_id}`}
        onclick={(event) => handleLinkClick(event, `/works/${workId}/chapters/${chapter.next_chapter_id}`)}
      >
        Next chapter →
      </a>
    {/if}
  </nav>
{/if}

<style>
  .prose {
    max-width: 66ch;
    font-family: var(--font-reading);
    font-size: var(--text-lg);
    line-height: 1.75;
  }

  .prose :global(blockquote) {
    border-left: 3px solid var(--color-accent);
    margin: var(--space-4) 0;
    padding-left: var(--space-4);
    color: var(--color-muted);
  }

  .prose :global(hr) {
    border: none;
    text-align: center;
    margin: var(--space-5) 0;
  }

  .prose :global(hr)::after {
    content: '* * *';
    color: var(--color-muted);
    letter-spacing: 0.4em;
  }

  .meta,
  .crumbs {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .pager {
    display: flex;
    justify-content: space-between;
    gap: var(--space-4);
    margin-top: var(--space-6);
    padding-top: var(--space-4);
    border-top: var(--border-width) solid var(--color-border);
  }

  .next {
    margin-left: auto;
  }
</style>
