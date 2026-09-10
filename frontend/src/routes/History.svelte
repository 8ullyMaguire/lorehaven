<script lang="ts">
  /**
   * The reader's history: what they have been reading.
   *
   * Spec §9.8: a signed-in reader can see their history, delete a
   * single entry, or clear the whole log. History is per pseud
   * (switching pseud shows the other face's history).
   */
  import { fetchHistory, deleteHistoryEntry, clearHistory } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  let entries = $state<Array<{ id: string; title: string; last_read_at: string; chapter_title: string }>>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);

  $effect(() => {
    void load();
  });

  async function load() {
    loading = true;
    error = null;
    try {
      const view = await fetchHistory();
      entries = view.entries.map((e) => ({
        id: e.id,
        title: e.subject_type === 'work' ? (e as { title?: string }).title ?? e.subject_id : e.subject_id,
        last_read_at: e.last_read_at,
        chapter_title: '',
      }));
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  async function handleDelete(id: string) {
    try {
      await deleteHistoryEntry(id);
      entries = entries.filter((e) => e.id !== id);
    } catch (failure) {
      error = failure;
    }
  }

  async function handleClear() {
    if (!confirm('Clear all history? This cannot be undone.')) return;
    try {
      await clearHistory();
      entries = [];
    } catch (failure) {
      error = failure;
    }
  }
</script>

{#if loading}
  <Skeleton lines={5} />
{:else if error}
  <h1>History</h1>
  <ErrorSummary error={error} />
{:else if entries.length === 0}
  <h1>History</h1>
  <p class="note">You haven't read anything yet.</p>
{:else}
  <h1>History</h1>
  <p class="meta">{entries.length} {entries.length === 1 ? 'entry' : 'entries'}</p>
  <ul class="history">
    {#each entries as entry (entry.id)}
      <li class="history-entry">
        <span class="title">{entry.title}</span>
        <span class="date">{entry.last_read_at}</span>
        <button onclick={() => handleDelete(entry.id)} class="delete" aria-label="Delete {entry.title} from history">
          Remove
        </button>
      </li>
    {/each}
  </ul>
  <button onclick={handleClear} class="clear-all">Clear all history</button>
{/if}

<style>
  .meta {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .note {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .history {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .history-entry {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
  }

  .history-entry .title {
    flex: 1;
  }

  .history-entry .date {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .history-entry .delete {
    font-size: var(--text-sm);
    padding: var(--space-1) var(--space-2);
  }

  .clear-all {
    margin-top: var(--space-4);
    font-size: var(--text-sm);
    color: var(--color-muted);
    padding: var(--space-2) var(--space-3);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    background: transparent;
    cursor: pointer;
  }
</style>
