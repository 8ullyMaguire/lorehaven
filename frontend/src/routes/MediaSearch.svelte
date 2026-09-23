<script lang="ts">
  /**
   * Reverse media search (spec §32.7.3).
   *
   * Search for a media reference by its perceptual hash or original URL.
   * When found, show the works that reference it — useful for finding all
   * stories that use a particular image.
   */

  import {
    reverseMediaSearch,
    type ReverseSearchView,
  } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte.ts';
  import Button from '../lib/components/Button.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import TextField from '../lib/components/TextField.svelte';

  let mode = $state<'hash' | 'url'>('hash');
  let hashInput = $state('');
  let urlInput = $state('');
  let result = $state<ReverseSearchView | null>(null);
  let loading = $state(false);
  let error = $state<unknown>(null);
  let searched = $state(false);

  async function onSubmit(e: SubmitEvent) {
    e.preventDefault();
    if (mode === 'hash' && !hashInput.trim()) return;
    if (mode === 'url' && !urlInput.trim()) return;
    loading = true;
    error = null;
    result = null;
    searched = true;
    try {
      const body = mode === 'hash' ? { hash: hashInput.trim() } : { url: urlInput.trim() };
      result = await reverseMediaSearch(body);
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }
</script>

<svelte:head><title>Reverse Media Search · Lorehaven</title></svelte:head>

<section class="page">
  <header>
    <p class="eyebrow">TOOLS</p>
    <h1>Reverse Media Search</h1>
    <p class="lede">
      Find all works that share a media reference. Search by perceptual hash
      or paste the original URL.
    </p>
  </header>

  <div class="mode-toggle" role="tablist" aria-label="Search mode">
    <button
      class="mode-btn"
      class:active={mode === 'hash'}
      role="tab"
      aria-selected={mode === 'hash'}
      onclick={() => (mode = 'hash')}
    >
      By Hash
    </button>
    <button
      class="mode-btn"
      class:active={mode === 'url'}
      role="tab"
      aria-selected={mode === 'url'}
      onclick={() => (mode = 'url')}
    >
      By URL
    </button>
  </div>

  <form class="search-form" onsubmit={onSubmit}>
    {#if mode === 'hash'}
      <TextField
        label="Perceptual hash"
        bind:value={hashInput}
        placeholder="e.g. 8f3a2b1c..."
      />
    {:else}
      <TextField
        label="URL"
        bind:value={urlInput}
        placeholder="https://..."
      />
    {/if}
    <Button type="submit" loading={loading}>Search</Button>
  </form>

  {#if loading}
    <Skeleton rows={3} />
  {:else if error}
    <ErrorSummary error={error} />
  {:else if result}
    {#if result.references.length === 0}
      <EmptyState
        title="No matching media"
        body="No media reference matched that hash or URL."
      />
    {:else}
      <h2>Matching References</h2>
      <ul class="ref-list">
        {#each result.references as ref (ref.id)}
          <li class="ref-row">
            <div class="ref-meta">
              <code>{ref.perceptual_hash ?? ref.content_hash.slice(0, 16)}</code>
              <span class="badge">{ref.media_kind}</span>
              {#if ref.curator_verified}
                <span class="badge badge-ok">Curator verified</span>
              {/if}
            </div>
          </li>
        {/each}
      </ul>

      {#if result.works.length > 0}
        <h2>Found In ({result.works.length} {result.works.length === 1 ? 'work' : 'works'})</h2>
        <ul class="work-list">
          {#each result.works as w (w.work_id + w.reference_id)}
            <li class="work-row">
              <div class="work-main">
                <a href={`/works/${w.work_id}`} onclick={handleLinkClick}>
                  {w.work_title}
                </a>
                {#if w.display_url}
                  <span class="muted url">{w.display_url}</span>
                {/if}
              </div>
              <Button
                size="sm"
                variant="quiet"
                onclick={() => (window.location.href = `/works/${w.work_id}`)}
              >
                View work
              </Button>
            </li>
          {/each}
        </ul>
      {:else}
        <EmptyState
          title="No works found"
          body="No works reference this media reference."
        />
      {/if}
    {/if}
  {:else if searched}
    <EmptyState
      title="No matches"
      body="Try a different hash or URL."
    />
  {/if}
</section>

<style>
  .page {
    max-width: 800px;
    margin: 0 auto;
    padding: var(--space-6);
  }
  .eyebrow {
    color: var(--color-muted);
    font-size: 0.75rem;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    margin-bottom: var(--space-1);
  }
  h1 {
    margin-bottom: var(--space-2);
  }
  .lede {
    color: var(--color-muted);
    margin-bottom: var(--space-6);
  }
  .mode-toggle {
    display: flex;
    gap: var(--space-2);
    margin-bottom: var(--space-4);
  }
  .mode-btn {
    background: none;
    border: 1px solid var(--color-border);
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius);
    cursor: pointer;
    font-weight: 500;
    color: var(--color-muted);
  }
  .mode-btn.active {
    background: var(--color-accent-bg);
    border-color: var(--color-accent);
    color: var(--color-accent);
  }
  .search-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin-bottom: var(--space-6);
  }
  h2 {
    margin-top: var(--space-6);
    margin-bottom: var(--space-3);
    font-size: 1.1rem;
  }
  .ref-list,
  .work-list {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .ref-row,
  .work-row {
    padding: var(--space-3);
    border: 1px solid var(--color-border);
    border-radius: var(--radius);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .ref-meta {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  code {
    font-family: var(--font-mono);
    font-size: 0.85rem;
    background: var(--color-surface-alt);
    padding: 2px 6px;
    border-radius: 4px;
  }
  .badge {
    font-size: 0.75rem;
    padding: 2px 8px;
    border-radius: 999px;
    font-weight: 500;
    background: var(--color-surface-alt);
    color: var(--color-muted);
  }
  .badge-ok {
    background: var(--color-success-bg);
    color: var(--color-success);
  }
  .work-main {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    flex: 1;
    min-width: 0;
  }
  .muted {
    color: var(--color-muted);
    font-size: 0.85rem;
  }
  .url {
    word-break: break-all;
  }
</style>
