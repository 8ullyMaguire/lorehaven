<script lang="ts">
  /**
   * The reader's history: what they have been reading (spec §9.6).
   *
   * History is per **pseud**, not per account: the face that read the work is
   * the face that saw it, so switching pseud shows the other face's list. That
   * is why the page reloads when the acting pseud changes rather than filtering
   * in the browser.
   *
   * Two deliberate choices:
   *
   *  * **Clearing asks first.** It is the only irreversible action here, and
   *    the server has no undo for it.
   *  * **A failure to delete says so** rather than optimistically removing the
   *    row: the row is still on the server, and pretending otherwise would be a
   *    lie the next page load would expose.
   */
  import { clearHistory, deleteHistoryEntry, fetchHistory, type HistoryItem } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte.ts';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  let items = $state<HistoryItem[]>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);
  let confirming = $state(false);

  // Reload when the acting pseud changes: history belongs to the face.
  const actingPseud = $derived(session.activePseud?.id ?? null);

  $effect(() => {
    void actingPseud;
    void load();
  });

  async function load() {
    loading = true;
    error = null;
    try {
      const view = await fetchHistory();
      items = view.items;
    } catch (failure) {
      error = failure;
      items = [];
    } finally {
      loading = false;
    }
  }

  async function remove(id: string) {
    error = null;
    try {
      await deleteHistoryEntry(id);
      items = items.filter((item) => item.id !== id);
    } catch (failure) {
      error = failure;
    }
  }

  async function clearAll() {
    error = null;
    try {
      await clearHistory();
      items = [];
      confirming = false;
    } catch (failure) {
      error = failure;
    }
  }
</script>

<h1>History</h1>

{#if !session.isSignedIn}
  <p class="note">
    <a href="/sign-in" onclick={(event) => handleLinkClick(event, '/sign-in')}>Sign in</a> to keep a
    history of what you read. Nobody else can see it.
  </p>
{:else if loading}
  <Skeleton lines={5} />
{:else if error && items.length === 0}
  <ErrorSummary error={error} />
{:else}
  {#if items.length === 0}
    <p class="note">You haven't read anything yet.</p>
  {:else}
    <p class="meta">
      {items.length} {items.length === 1 ? 'entry' : 'entries'} for
      @{session.activePseud?.handle ?? 'this pseud'}
    </p>

    <ul class="history">
      {#each items as item (item.id)}
        <li class="history-entry">
          <span class="title">
            {#if item.subject_type === 'work'}
              <a
                href={`/works/${item.subject_id}`}
                onclick={(event) => handleLinkClick(event, `/works/${item.subject_id}`)}
              >
                {item.title.trim() === '' ? 'Untitled' : item.title}
              </a>
            {:else}
              {item.title}
            {/if}
          </span>
          {#if item.authors.length > 0}
            <span class="authors">by {item.authors.join(', ')}</span>
          {/if}
          <span class="date">{item.last_read_at.slice(0, 10)}</span>
          <button
            type="button"
            class="quiet"
            onclick={() => remove(item.id)}
            aria-label="Remove {item.title} from history"
          >
            Remove
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  {#if error}
    <ErrorSummary error={error} />
  {/if}

  {#if confirming}
    <p role="alert">Clear your whole history? This cannot be undone.</p>
    <div class="confirm">
      <button type="button" onclick={clearAll}>Yes, clear it</button>
      <button type="button" class="quiet" onclick={() => (confirming = false)}>Cancel</button>
    </div>
  {:else if items.length > 0}
    <button type="button" class="quiet clear-all" onclick={() => (confirming = true)}>
      Clear all history
    </button>
  {/if}
{/if}

<style>
  .meta,
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
    flex-wrap: wrap;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
  }

  .history-entry .title {
    flex: 1;
    min-width: 12ch;
  }

  .history-entry .authors,
  .history-entry .date {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .quiet {
    background: none;
    border: var(--border-width) solid var(--color-border);
    color: var(--color-muted);
    font-size: var(--text-sm);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
  }

  .clear-all {
    margin-top: var(--space-4);
  }

  .confirm {
    display: flex;
    gap: var(--space-3);
    margin-top: var(--space-2);
  }
</style>
