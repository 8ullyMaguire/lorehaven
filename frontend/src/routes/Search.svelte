<script lang="ts">
  import { searchWorks, type SearchResultList } from '../lib/api';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  let query = $state('');
  let results = $state<SearchResultList | null>(null);
  let loading = $state(false);
  let error = $state<unknown>(null);
  let searched = $state(false);

  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  function onInput() {
    if (debounceTimer) clearTimeout(debounceTimer);
    if (query.trim().length === 0) {
      results = null;
      searched = false;
      return;
    }
    debounceTimer = setTimeout(() => {
      void runSearch();
    }, 300);
  }

  async function runSearch() {
    loading = true;
    error = null;
    searched = true;
    try {
      results = await searchWorks(query, 20);
    } catch (failure) {
      error = failure;
      results = null;
    } finally {
      loading = false;
    }
  }

  function onSubmit(e: Event) {
    e.preventDefault();
    if (debounceTimer) clearTimeout(debounceTimer);
    void runSearch();
  }
</script>

<section class="search-page">
  <header>
    <h1>Search works</h1>
    <p class="lede">
      Find works by title, author, fandom, character, relationship, tag, or mood.
    </p>
  </header>

  <form class="search-form" onsubmit={onSubmit}>
    <input
      type="search"
      bind:value={query}
      oninput={onInput}
      placeholder='e.g. fandom:&quot;Harry Potter&quot; AND tag:draco'
      aria-label="Search query"
    />
    <button type="submit" disabled={loading}>
      {loading ? 'Searching…' : 'Search'}
    </button>
  </form>

  {#if error}
    <ErrorSummary {error} onretry={runSearch} />
  {:else if loading && !results}
    <Skeleton lines={5} label="Searching" />
  {:else if results && results.items.length > 0}
    <ul class="results">
      {#each results.items as item (item.work_id)}
        <li>
          <a href={`/works/${item.work_id}`} class="result-card">
            <span class="title">{item.title}</span>
            <span class="author">by {item.author_handle}</span>
            <span class="meta">{item.word_count} words</span>
          </a>
        </li>
      {/each}
    </ul>
  {:else if searched}
    <EmptyState
      title="No matches found"
      description="Try a different query, or use fielded search like fandom:name, tag:name, mood:name."
    />
  {:else}
    <EmptyState
      title="Start typing to search"
      description="Search across titles, summaries, authors, and tags."
    />
  {/if}
</section>

<style>
  .search-page {
    max-width: 720px;
    margin: 0 auto;
    padding: var(--space-4);
  }

  header h1 {
    margin-bottom: var(--space-2);
  }

  .lede {
    color: var(--color-muted);
    margin-bottom: var(--space-4);
  }

  .search-form {
    display: flex;
    gap: var(--space-2);
    margin-bottom: var(--space-4);
  }

  .search-form input {
    flex: 1;
    padding: var(--space-2);
    border: 1px solid var(--color-border);
    border-radius: 4px;
    font-size: var(--text-base);
  }

  .results {
    list-style: none;
    padding: 0;
    margin: 0;
  }

  .result-card {
    display: flex;
    flex-direction: column;
    padding: var(--space-3);
    border: 1px solid var(--color-border);
    border-radius: 4px;
    text-decoration: none;
    color: inherit;
    margin-bottom: var(--space-2);
  }

  .result-card:hover {
    background: var(--color-surface-alt);
  }

  .title {
    font-weight: 600;
  }

  .author {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .meta {
    color: var(--color-muted);
    font-size: var(--text-xs);
  }
</style>
