<script lang="ts">
  import { onMount } from 'svelte';
  import { apiFetch } from '../lib/api';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  interface Item { id: string; title: string; summary: string | null; format: string; rating: string }
  interface Page { items: Item[]; total: number; next_cursor: string | null }
  let query = $state(new URLSearchParams(window.location.search).get('q') ?? '');
  let applied = $state('');
  let results = $state<Page | null>(null);
  let loading = $state(false);
  let error = $state<unknown>(null);
  let generation = 0;
  let controller: AbortController | null = null;
  const feed = $derived(`/api/v1/media/feed?${new URLSearchParams({ q: applied })}`);

  async function load(more = false) {
    const current = ++generation;
    controller?.abort();
    controller = new AbortController();
    loading = true;
    error = null;
    if (!more) { applied = query.trim(); results = null; }
    const params = new URLSearchParams({ q: applied, limit: '24' });
    if (more && results?.next_cursor) params.set('cursor', results.next_cursor);
    try {
      const page = await apiFetch<Page>(`/media?${params}`, { signal: controller.signal });
      if (current !== generation) return;
      results = more && results ? { ...page, items: [...results.items, ...page.items] } : page;
    } catch (failure) {
      if (current === generation) error = failure;
    } finally {
      if (current === generation) loading = false;
    }
  }
  onMount(() => {
    void load();
    return () => { generation++; controller?.abort(); };
  });
</script>

<svelte:head><title>Browse media · Lorehaven</title></svelte:head>
<section class="catalogue">
  <header>
    <p class="eyebrow">THE CATALOGUE</p>
    <h1>Browse media</h1>
    <p class="lede">Stories, editions, and archives. Find something worth keeping.</p>
  </header>
  <form role="search" onsubmit={(event) => { event.preventDefault(); void load(); }}>
    <label for="media-query">Search media</label>
    <div class="search-row">
      <input id="media-query" type="search" bind:value={query} placeholder="A title, creator, or tag" aria-describedby="query-help" />
      <button type="submit">Search</button>
    </div>
    <details id="query-help">
      <summary>Search tips</summary>
      <p>Combine fields with AND, OR, and NOT. Quote phrases containing spaces.</p>
      <p><code>title:"quiet archive" AND format:prose</code></p>
      <p>Try author:, tag:, language:, rating:, edition:, or updated:2026-01-01..2026-12-31.</p>
    </details>
  </form>
  <div class="toolbar">
    <p role="status">{loading ? 'Loading media…' : results ? `${results.total} ${results.total === 1 ? 'result' : 'results'}` : ''}</p>
    <nav aria-label="Tools">
      <a href="/media/search">Reverse search</a>
      <a href={`${feed}&format=atom`}>Atom feed</a>
      <a href={`${feed}&format=rss`}>RSS feed</a>
    </nav>
  </div>
  {#if error}<ErrorSummary {error} onretry={() => load()} />{/if}
  {#if loading && !results}
    <Skeleton lines={6} label="Loading catalogue" />
  {:else if results?.items.length}
    <ul class="cards" aria-label="Media results" aria-busy={loading}>
      {#each results.items as item (item.id)}
        <li>
          <div class="metadata"><span>{item.format.replaceAll('_', ' ')}</span><span>{item.rating}</span></div>
          <h2><a href={`/works/${item.id}`}>{item.title}</a></h2>
          <p class="description">{item.summary || 'No summary provided.'}</p>
        </li>
      {/each}
    </ul>
    {#if results.next_cursor}
      <button class="more" disabled={loading} onclick={() => load(true)}>{loading ? 'Loading…' : 'Load more'}</button>
    {/if}
  {:else if results}
    <EmptyState title="No media found" description="Try fewer filters or a different phrase. Only media available to you appears here." />
  {/if}
</section>

<style>
  .catalogue { max-width: 1080px; margin: 0 auto; padding: var(--space-4); overflow-x: hidden; }
  header { margin-block: var(--space-4) var(--space-6); }
  .eyebrow { color: var(--color-muted); font-size: var(--text-sm); letter-spacing: .12em; }
  h1 { margin-block: var(--space-2); }
  .lede, details, .description { color: var(--color-muted); }
  form { padding: var(--space-4); border: 1px solid var(--color-border); border-radius: 8px; }
  label { display: block; font-weight: 600; margin-bottom: var(--space-2); }
  .search-row { display: flex; gap: var(--space-2); }
  input { flex: 1; min-width: 0; padding: var(--space-3); font: inherit; color: inherit; background: var(--color-surface); border: 1px solid var(--color-border); border-radius: 4px; }
  button { min-height: 44px; padding: var(--space-2) var(--space-4); font: inherit; cursor: pointer; }
  details { margin-top: var(--space-3); font-size: var(--text-sm); overflow-wrap: anywhere; }
  summary { cursor: pointer; }
  .toolbar { display: flex; justify-content: space-between; align-items: center; gap: var(--space-3); flex-wrap: wrap; margin-block: var(--space-4); }
  nav { display: flex; gap: var(--space-4); }
  .cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 280px), 1fr)); gap: var(--space-4); list-style: none; padding: 0; }
  li { border: 1px solid var(--color-border); border-radius: 8px; padding: var(--space-4); overflow-wrap: anywhere; }
  .metadata { display: flex; gap: var(--space-3); text-transform: capitalize; font-size: var(--text-sm); color: var(--color-muted); }
  h2 { font-size: var(--text-lg); margin-block: var(--space-3); }
  h2 a { color: inherit; text-decoration-thickness: 1px; text-underline-offset: 4px; }
  .description { line-height: 1.6; }
  .more { display: block; margin: var(--space-6) auto; }
  @media (max-width: 400px) { .search-row { flex-direction: column; } }
</style>
