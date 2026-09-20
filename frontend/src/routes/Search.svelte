<script lang="ts">
  import { searchWorks, type SearchResultList } from '../lib/api';
  import Combobox from '../lib/components/Combobox.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  type FilterField = 'fandom' | 'tag' | 'mood' | 'rating' | 'language' | 'status';

  const FILTER_OPTIONS: Record<FilterField, string[]> = {
    fandom: ['Any fandom'],
    tag: ['Any tag'],
    mood: ['Angst', 'Fluff', 'Humor', 'Romance', 'Drama', 'Hurt/Comfort'],
    rating: ['General', 'Teen', 'Mature', 'Explicit'],
    language: ['English', 'Spanish', 'French', 'German', 'Japanese', 'Chinese', 'Korean'],
    status: ['Complete', 'In-Progress', 'Hiatus'],
  };

  let query = $state('');
  let results = $state<SearchResultList | null>(null);
  let loading = $state(false);
  let error = $state<unknown>(null);
  let searched = $state(false);

  let selectedField = $state<FilterField>('fandom');
  let selectedValue = $state('');
  let selectedFilters = $state<{ field: FilterField; value: string }[]>([]);

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

  function addFilter() {
    if (!selectedValue || selectedValue.startsWith('Any ')) return;
    selectedFilters = [...selectedFilters, { field: selectedField, value: selectedValue }];
    selectedValue = '';
    void runSearch();
  }

  function removeFilter(index: number) {
    selectedFilters = selectedFilters.filter((_, i) => i !== index);
    void runSearch();
  }

  function buildQuery(): string {
    const freeText = query.trim();
    const filterParts = selectedFilters.map((f) => `${f.field}:"${f.value}"`);
    if (freeText && filterParts.length > 0) {
      return `${freeText} AND ${filterParts.join(' AND ')}`;
    }
    return freeText || filterParts.join(' AND ');
  }

  async function runSearch() {
    const fullQuery = buildQuery();
    if (!fullQuery.trim()) {
      results = null;
      return;
    }
    loading = true;
    error = null;
    searched = true;
    try {
      results = await searchWorks(fullQuery, 20);
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

  function onFieldChange(field: FilterField) {
    selectedField = field;
    selectedValue = '';
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
      placeholder='e.g. fandom:"Harry Potter" AND tag:draco'
      aria-label="Search query"
    />
    <button type="submit" disabled={loading}>
      {loading ? 'Searching…' : 'Search'}
    </button>
  </form>

  <fieldset class="filters">
    <legend>Refine by</legend>
    <div class="filter-row">
      <div class="field">
        <label for="filter-field">Field</label>
        <select id="filter-field" bind:value={selectedField} onchange={() => onFieldChange(selectedField)}>
          {#each Object.keys(FILTER_OPTIONS) as field}
            <option value={field}>{field.charAt(0).toUpperCase() + field.slice(1)}</option>
          {/each}
        </select>
      </div>
      <Combobox
        label="Value"
        options={FILTER_OPTIONS[selectedField].map((v) => ({ value: v, label: v }))}
        value={selectedValue}
        onchange={(v) => (selectedValue = v)}
        placeholder="Select a value..."
      />
      <button type="button" class="add-filter" onclick={addFilter} disabled={!selectedValue || selectedValue.startsWith('Any ')}>
        Add
      </button>
    </div>

    {#if selectedFilters.length > 0}
      <ul class="active-filters">
        {#each selectedFilters as filter, i}
          <li>
            <span class="filter-tag">{filter.field}: {filter.value}</span>
            <button type="button" class="remove" onclick={() => removeFilter(i)} aria-label="Remove filter">
              &times;
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </fieldset>

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

  .filters {
    border: 1px solid var(--color-border);
    border-radius: 4px;
    padding: var(--space-3);
    margin-bottom: var(--space-4);
  }

  .filters legend {
    font-weight: 600;
    padding: 0 var(--space-1);
  }

  .filter-row {
    display: flex;
    gap: var(--space-2);
    align-items: flex-end;
    flex-wrap: wrap;
  }

  .filter-row .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .filter-row label {
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .filter-row select {
    padding: var(--space-2);
    border: 1px solid var(--color-border);
    border-radius: 4px;
    min-height: 2.75rem;
    font: inherit;
  }

  .add-filter {
    min-height: 2.75rem;
    padding: var(--space-2) var(--space-3);
  }

  .active-filters {
    list-style: none;
    padding: 0;
    margin: var(--space-2) 0 0;
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .filter-tag {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
    background: var(--color-surface-alt);
    color: var(--color-primary);
    border-radius: 4px;
    font-size: var(--text-sm);
  }

  .remove {
    background: none;
    border: none;
    color: var(--color-primary);
    cursor: pointer;
    font-size: var(--text-base);
    padding: 0;
  }
</style>
