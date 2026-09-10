<script lang="ts">
  /**
   * A work's public page.
   *
   * The same route answers a contributor with the author's view, so this page
   * says which one it is rather than pretending: a contributor gets a note and
   * a link to the editor, everyone else gets the reading page.
   */
  import { fetchWork, isAuthorWork, type AuthorWork, type PublicWork } from '../lib/api';
  import { describeCompletion, describeLifecycle, describeRating, describeVisibility } from '../lib/labels';
  import { handleLinkClick } from '../lib/router';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  interface Props {
    workId: string;
  }

  let { workId }: Props = $props();

  let work = $state<PublicWork | AuthorWork | null>(null);
  let error = $state<unknown>(null);
  let loading = $state(true);

  const authorView = $derived(work !== null && isAuthorWork(work) ? (work as AuthorWork) : null);

  $effect(() => {
    void load();
  });

  async function load() {
    loading = true;
    error = null;
    try {
      work = await fetchWork(workId);
    } catch (failure) {
      error = failure;
      work = null;
    } finally {
      loading = false;
    }
  }
</script>

{#if loading}
  <Skeleton lines={4} />
{:else if error}
  <h1>Not found</h1>
  <ErrorSummary error={error} />
  <p>
    <a href="/" onclick={(event) => handleLinkClick(event, '/')}>Back to the front page</a>
  </p>
{:else if work}
  {#if authorView}
    <p class="draft-note" role="status">
      {#if authorView.lifecycle === 'published'}
        You contribute to this work, so you see it here along with its editing controls.
      {:else}
        This work is {describeLifecycle(authorView.lifecycle).toLowerCase()}, so only its
        contributors can see it. Others see it once it is published.
      {/if}
      <a href={`/write/${work.id}`} onclick={(event) => handleLinkClick(event, `/write/${work.id}`)}>
        Edit it
      </a>
    </p>
  {/if}

  <h1>{work.title.trim() === '' ? 'Untitled' : work.title}</h1>

  <p class="byline">
    {#each (work as PublicWork).authors ?? [] as author, index (author.handle)}
      {#if index > 0}· {/if}<a href={`/pseud/${author.handle}`} onclick={(event) => handleLinkClick(event, `/pseud/${author.handle}`)}>@{author.handle}</a>
    {/each}
  </p>

  <p class="badges">
    <span>{describeRating(work.rating)}</span>
    <span>{describeVisibility(work.visibility)}</span>
    <span>{describeCompletion(work.completion)}</span>
    {#if work.published_at}<span>Published {work.published_at.slice(0, 10)}</span>{/if}
  </p>

  {#if work.summary.trim() !== ''}
    <p class="summary">{work.summary}</p>
  {/if}

  <h2>Chapters</h2>
  {#if work.chapters.length === 0}
    <p class="note">This work has no chapters.</p>
  {:else}
    <ol class="chapters">
      {#each work.chapters as chapter (chapter.id)}
        <li>
          <a
            href={`/works/${work.id}/chapters/${chapter.id}`}
            onclick={(event) => handleLinkClick(event, `/works/${work.id}/chapters/${chapter.id}`)}
          >
            {chapter.title.trim() === '' ? 'Untitled chapter' : chapter.title}
          </a>
          <span class="note">{chapter.word_count} words</span>
        </li>
      {/each}
    </ol>
  {/if}
{/if}

<style>
  .byline {
    font-size: var(--text-lg);
  }

  .badges {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .summary {
    max-width: 62ch;
  }

  .draft-note {
    background: var(--color-surface);
    border: var(--border-width) solid var(--color-accent);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    font-size: var(--text-sm);
  }

  .chapters {
    padding-left: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .note {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }
</style>
