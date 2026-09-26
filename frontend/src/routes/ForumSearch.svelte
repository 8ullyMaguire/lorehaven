<script lang="ts">
  import { onMount } from 'svelte';
  import { fetchForums, searchForum, type Forum, type ForumSearchResult } from '../lib/api';

  let query = '';
  let category = '';
  let author = '';
  let minReplies: number | null = null;
  let forums: Forum[] = [];
  let results: ForumSearchResult[] = [];
  let loading = false;
  let searched = false;
  let error = '';
  let totalResults = 0;

  onMount(async () => {
    // The category list comes from the instance rather than being hardcoded.
    // The previous list was invented -- `general`, `fanworks`, `discussion`,
    // `help` -- and none of them is a category a fresh instance creates, so a
    // reader who picked "General" and got nothing had no way to tell the
    // dropdown was lying. A failed fetch leaves the filter out entirely: an
    // empty dropdown that filters to nothing is worse than no dropdown, and the
    // query box still works.
    try {
      forums = await fetchForums();
    } catch {
      forums = [];
    }
  });

  async function doSearch() {
    // A filter with no text is a complete query -- `category:meta` alone. The
    // old guard required text, which made the filters look broken.
    if (!query.trim() && !category && !author && minReplies === null) return;
    loading = true;
    error = '';
    try {
      // Typed as the API's own parameter object so `q` cannot be dropped;
      // a bare Record loses the required-ness the server enforces.
      const params: Parameters<typeof searchForum>[0] = { q: query.trim(), limit: 20 };
      if (category) params.category = category;
      if (author) params.author = author;
      if (minReplies !== null && Number.isFinite(minReplies)) params.min_replies = minReplies;
      results = await searchForum(params);
      totalResults = results.length;
      searched = true;
    } catch (e: any) {
      // A rejected query is a 422 carrying why. Replacing that with "Search
      // failed" throws away the only useful part of the response -- the
      // reason the reader can act on.
      error = e?.message || 'Search failed';
      results = [];
      searched = true;
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
        aria-label="Search posts and topics"
        class="search-input"
      />
      <button type="submit" disabled={loading}>
        {#if loading}Searching...{:else}Search{/if}
      </button>
    </div>
    <div class="filters">
      <label>
        Category
        <select bind:value={category} aria-label="Category">
          <option value="">All</option>
          {#each forums as f (f.id)}
            <option value={f.name}>{f.name}</option>
          {/each}
        </select>
      </label>
      <label>
        Author
        <input type="text" bind:value={author} aria-label="Author" placeholder="Filter by author" />
      </label>
      <label>
        Min replies
        <input
          type="number"
          min="0"
          step="1"
          bind:value={minReplies}
          aria-label="Min replies"
          placeholder="Any"
        />
      </label>
    </div>
    <p class="help">
      You can also type filters directly: <code>replies:&gt;50</code>,
      <code>category:meta</code>, <code>author:nightowl</code>,
      <code>active:&gt;2026-01</code>, <code>locked:true</code>. Combine with
      <code>AND</code>, <code>OR</code> and <code>NOT</code>; quote a phrase with
      <code>"like this"</code>.
    </p>
  </form>

  {#if error}
    <p class="error" role="alert">{error}</p>
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
              <span class="score">{r.score} replies</span>
            {/if}
          </div>
        </div>
      </li>
    {/each}
  </ul>

  {#if !loading && searched && !error && results.length === 0}
    <p class="no-results">No results found for "{query || 'that search'}"</p>
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
  .help {
    margin: 0.75rem 0 0;
    font-size: 0.8rem;
    color: var(--text-muted);
    line-height: 1.6;
  }
  .help code {
    background: var(--bg-elevated);
    padding: 0.1rem 0.3rem;
    border-radius: 3px;
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
