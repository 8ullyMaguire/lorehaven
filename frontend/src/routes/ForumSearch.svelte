<script lang="ts">
  import { onMount } from 'svelte';
  import { searchForum, type ForumSearchResult } from '../lib/api';
  import { session } from '../lib/session.svelte';

  let query = '';
  let category = '';
  let author = '';
  let results: ForumSearchResult[] = [];
  let loading = false;
  let error = '';
  let totalResults = 0;

  async function doSearch() {
    if (!query.trim()) return;
    loading = true;
    error = '';
    try {
      const params: Record<string, string | number> = { q: query.trim(), limit: 20 };
      if (category) params.category = category;
      if (author) params.author = author;
      results = await searchForum(params);
      totalResults = results.length;
    } catch (e: any) {
      error = e?.message || 'Search failed';
      results = [];
    } finally {
      loading = false;
    }
  }

  function submit(e: Event) {
    e.preventDefault();
    doSearch();
  }

  function formatDate(s: string) {
    try {
      return new Date(s).toLocaleDateString();
    } catch {
      return s;
    }
  }
</script>

<div class="forum-search">
  <h2>Search Forums</h2>
  <form on:submit={submit} class="search-form">
    <div class="row">
      <input
        type="search"
        bind:value={query}
        placeholder="Search posts and topics..."
        class="search-input"
      />
      <button type="submit" disabled={loading || !query.trim()}>
        {#if loading}Searching...{:else}Search{/if}
      </button>
    </div>
    <div class="filters">
      <label>
        Category
        <select bind:value={category}>
          <option value="">All</option>
          <option value="general">General</option>
          <option value="fanworks">Fanworks</option>
          <option value="discussion">Discussion</option>
          <option value="help">Help</option>
        </select>
      </label>
      <label>
        Author
        <input type="text" bind:value={author} placeholder="Filter by author" />
      </label>
    </div>
  </form>

  {#if error}
    <p class="error">{error}</p>
  {/if}

  {#if totalResults > 0}
    <p class="results-info">{totalResults} result{totalResults === 1 ? '' : 's'} found</p>
  {/if}

  <ul class="results">
    {#each results as r (r.id)}
      <li class="result">
        <div class="result-kind">{r.kind}</div>
        <div class="result-body">
          <a href={`/forums/topics/${r.id}`} class="result-title">{r.title}</a>
          <p class="snippet">{r.snippet}...</p>
          <div class="meta">
            <span>by {r.author_pseud}</span>
            <span>{formatDate(r.created_at)}</span>
            {#if r.score > 0}
              <span class="score">score: {r.score}</span>
            {/if}
          </div>
        </div>
      </li>
    {/each}
  </ul>

  {#if !loading && query && results.length === 0 && !error}
    <p class="no-results">No results found for "{query}"</p>
  {/if}
</div>

<style>
  .forum-search {
    max-width: 720px;
    margin: 0 auto;
    padding: 1rem;
  }
  h2 {
    margin-bottom: 1rem;
  }
  .search-form {
    margin-bottom: 1.5rem;
  }
  .row {
    display: flex;
    gap: 0.5rem;
    margin-bottom: 0.75rem;
  }
  .search-input {
    flex: 1;
    padding: 0.5rem;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-elevated);
    color: var(--text);
  }
  button {
    padding: 0.5rem 1rem;
    background: var(--accent);
    color: white;
    border: none;
    border-radius: 4px;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .filters {
    display: flex;
    gap: 1rem;
    flex-wrap: wrap;
  }
  .filters label {
    display: flex;
    flex-direction: column;
    font-size: 0.85rem;
    color: var(--text-muted);
  }
  .filters select,
  .filters input {
    padding: 0.3rem;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-elevated);
    color: var(--text);
  }
  .results {
    list-style: none;
    padding: 0;
  }
  .result {
    display: flex;
    gap: 0.75rem;
    padding: 0.75rem 0;
    border-bottom: 1px solid var(--border);
  }
  .result-kind {
    font-size: 0.75rem;
    text-transform: uppercase;
    color: var(--text-muted);
    min-width: 40px;
  }
  .result-title {
    font-weight: 600;
    color: var(--accent);
    text-decoration: none;
  }
  .result-title:hover {
    text-decoration: underline;
  }
  .snippet {
    margin: 0.25rem 0;
    color: var(--text-muted);
    font-size: 0.9rem;
  }
  .meta {
    font-size: 0.8rem;
    color: var(--text-muted);
    display: flex;
    gap: 1rem;
  }
  .error {
    color: var(--danger);
  }
  .no-results {
    color: var(--text-muted);
    text-align: center;
    padding: 2rem;
  }
  .results-info {
    color: var(--text-muted);
    font-size: 0.9rem;
    margin-bottom: 0.5rem;
  }
</style>
