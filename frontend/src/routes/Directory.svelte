<script lang="ts">
  import {
    fetchDirectoryEntries,
    fetchDirectoryCategories,
    submitDirectoryEntry,
    voteDirectoryEntry,
    approveDirectoryEntry,
    removeDirectoryEntry,
    type DirectoryEntry,
    type DirectoryCategory,
  } from '../lib/api';
    import { session } from '../lib/session.svelte.ts';
  import Button from '../lib/components/Button.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import DirectoryGovernance from '../lib/components/DirectoryGovernance.svelte';

  let entries = $state<DirectoryEntry[]>([]);
  let categories = $state<DirectoryCategory[]>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);
  let sort = $state<'top' | 'new'>('top');
  let category = $state<string | undefined>(undefined);
  let submitting = $state(false);
  let submitTitle = $state('');
  let submitUrl = $state('');
  let submitCategory = $state('');
  let submitDescription = $state('');
  let submitTags = $state('');

  async function load() {
    loading = true;
    error = null;
    try {
      const [entriesResult, categoriesResult] = await Promise.all([
        fetchDirectoryEntries({ sort, category, limit: 50 }),
        fetchDirectoryCategories(),
      ]);
      entries = entriesResult.items ?? [];
      categories = categoriesResult.items ?? [];
      if (categories.length > 0 && submitCategory === '') {
        submitCategory = categories[0].category;
      }
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  async function handleSubmit() {
    if (!submitTitle || !submitCategory) return;
    submitting = true;
    try {
      const tags = submitTags
        .split(',')
        .map((t) => t.trim())
        .filter(Boolean);
      await submitDirectoryEntry({
        list: 'main',
        kind: 'external',
        category: submitCategory,
        title: submitTitle,
        url: submitUrl || undefined,
        description: submitDescription || undefined,
        tags,
      });
      submitTitle = '';
      submitUrl = '';
      submitDescription = '';
      submitTags = '';
      await load();
    } catch (failure) {
      error = failure;
    } finally {
      submitting = false;
    }
  }

  async function handleVote(entryId: string, value: 1 | 0 | -1) {
    try {
      await voteDirectoryEntry(entryId, value);
      await load();
    } catch (failure) {
      error = failure;
    }
  }

  async function handleApprove(entryId: string) {
    try {
      await approveDirectoryEntry(entryId);
      await load();
    } catch (failure) {
      error = failure;
    }
  }

  async function handleRemove(entryId: string) {
    try {
      await removeDirectoryEntry(entryId);
      await load();
    } catch (failure) {
      error = failure;
    }
  }

  $effect(() => {
    void load();
  });
</script>

<section class="directory">
  <header class="directory-header">
    <h1>Resource Directory</h1>
    <p class="lede">
      Community-curated links and references, ranked by your peers. Submit
      something useful — an operator will review it.
    </p>
  </header>

  {#if error}
    <ErrorSummary {error} onretry={load} />
  {/if}

  {#if session.isSignedIn}
    <div class="directory-submit">
      <h2>Submit a resource</h2>
      <form
        onsubmit={(event) => {
          event.preventDefault();
          void handleSubmit();
        }}
      >
        <label>
          Title
          <input type="text" bind:value={submitTitle} placeholder="A useful writing tool" required />
        </label>
        <label>
          URL
          <input type="url" bind:value={submitUrl} placeholder="https://example.com" />
        </label>
        <label>
          Category
          <select bind:value={submitCategory}>
            {#each categories as cat}
              <option value={cat.category}>{cat.category}</option>
            {/each}
          </select>
        </label>
        <label>
          Description
          <textarea bind:value={submitDescription} rows="2" placeholder="Brief description"></textarea>
        </label>
        <label>
          Tags (comma-separated)
          <input type="text" bind:value={submitTags} placeholder="writing, research, community" />
        </label>
        <Button type="submit" disabled={submitting || !submitTitle || !submitCategory}>
          {submitting ? 'Submitting...' : 'Submit'}
        </Button>
      </form>
    </div>
  {/if}

  <div class="directory-controls">
    <label>
      Sort
      <select
        bind:value={sort}
        onchange={() => void load()}
      >
        <option value="top">Top</option>
        <option value="new">New</option>
      </select>
    </label>
    <label>
      Category
      <select
        bind:value={category}
        onchange={() => void load()}
      >
        <option value={undefined}>All</option>
        {#each categories as cat}
          <option value={cat.category}>{cat.category} ({cat.approved_count})</option>
        {/each}
      </select>
    </label>
  </div>

  {#if session.isSignedIn && session.me && session.me.trust_level >= 4}
    <DirectoryGovernance />
  {/if}

  {#if loading}
    <Skeleton lines={6} label="Loading directory" />
  {:else if entries.length === 0}
    <p class="empty">No entries yet. Be the first to submit one!</p>
  {:else}
    <ul class="directory-entries">
      {#each entries as entry}
        <li class="directory-entry">
          <div class="entry-vote">
            <button
              type="button"
              class="vote-up"
              class:voted={entry.my_vote === 1}
              onclick={() => handleVote(entry.id, 1)}
              disabled={!session.isSignedIn}
              title="Upvote"
              aria-label="Upvote"
            >
              ▲
            </button>
            <span class="score">{entry.score}</span>
            <button
              type="button"
              class="vote-down"
              class:voted={entry.my_vote === -1}
              onclick={() => handleVote(entry.id, -1)}
              disabled={!session.isSignedIn}
              title="Downvote"
              aria-label="Downvote"
            >
              ▼
            </button>
            <!--
              Withdrawing is its own control, not "click the arrow again":
              voting the same direction a second time now refreshes the vote,
              so the arrow is no longer a toggle and must not look like one.
            -->
            {#if entry.my_vote !== null && entry.my_vote !== undefined}
              <button
                type="button"
                class="vote-clear"
                onclick={() => handleVote(entry.id, 0)}
                disabled={!session.isSignedIn}
                title="Remove your vote"
                aria-label="Remove your vote"
              >
                ✕
              </button>
            {/if}
          </div>
          <div class="entry-content">
            <!--
              Only ever shown to someone who has actually voted, and only when
              it changes what they would do. A reader who has not voted is not
              owed an explanation of the ranking formula; a voter is, because
              the number they are looking at is a moving target and they are
              the only thing that can stop it moving.
            -->
            {#if entry.decay?.enabled && entry.my_vote !== null && entry.my_vote !== undefined}
              <p class="decay-note">
                {#if entry.decay.applies_to_this_entry}
                  Votes here count less over time — vote again to refresh it.
                {:else}
                  This entry does not age votes; it has too few to rank fairly.
                {/if}
              </p>
            {/if}
            <h3>
              {#if entry.url}
                <a href={entry.url} target="_blank" rel="noopener noreferrer">{entry.title}</a>
              {:else}
                {entry.title}
              {/if}
            </h3>
            {#if entry.description}
              <p class="entry-description">{entry.description}</p>
            {/if}
            {#if entry.tags && entry.tags.length > 0}
              <div class="entry-tags">
                {#each entry.tags as tag}
                  <span class="tag">{tag}</span>
                {/each}
              </div>
            {/if}
            {#if entry.approved_by === null}
              <span class="pending-badge">Pending review</span>
            {/if}
            <div class="entry-meta">
              <span>Submitted by {entry.submitted_by}</span>
              {#if session.isSignedIn && entry.approved_by !== null}
                <button type="button" class="entry-action" onclick={() => handleApprove(entry.id)}>
                  Approve
                </button>
                <button type="button" class="entry-action danger" onclick={() => handleRemove(entry.id)}>
                  Remove
                </button>
              {/if}
            </div>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .directory-header {
    margin-bottom: var(--space-6);
  }

  .directory-header h1 {
    margin-bottom: var(--space-2);
  }

  .lede {
    color: var(--text-muted);
    max-width: 60ch;
  }

  .directory-submit {
    margin-bottom: var(--space-6);
    padding: var(--space-4);
    background: var(--surface);
    border-radius: var(--radius);
    border: 1px solid var(--border);
  }

  .directory-submit h2 {
    margin-top: 0;
  }

  .directory-submit form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .directory-submit label {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .directory-submit input,
  .directory-submit select,
  .directory-submit textarea {
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--background);
    color: var(--text);
  }

  .directory-controls {
    display: flex;
    gap: var(--space-4);
    margin-bottom: var(--space-4);
  }

  .directory-controls label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .directory-controls select {
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--background);
    color: var(--text);
  }

  .directory-entries {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .directory-entry {
    display: flex;
    gap: var(--space-4);
    padding: var(--space-4);
    background: var(--surface);
    border-radius: var(--radius);
    border: 1px solid var(--border);
  }

  .entry-vote {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-1);
  }

  .vote-up,
  .vote-down {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-muted);
    font-size: var(--text-sm);
  }

  .vote-up:hover {
    color: var(--accent);
  }

  .vote-down:hover {
    color: var(--danger);
  }

  /* A cast vote is marked by colour AND by `aria-pressed`, so the state is
     not carried by hue alone. */
  .vote-up.voted,
  .vote-down.voted {
    font-weight: 700;
  }

  .vote-up.voted {
    color: var(--accent);
  }

  .vote-down.voted {
    color: var(--danger);
  }

  .vote-clear {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-muted);
    font-size: var(--text-xs);
    padding: 0 var(--space-1);
  }

  .vote-clear:hover {
    color: var(--text);
  }

  .vote-clear:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .decay-note {
    margin: 0 0 var(--space-1);
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .score {
    font-weight: 600;
    font-size: var(--text-lg);
  }

  .entry-content {
    flex: 1;
  }

  .entry-content h3 {
    margin: 0;
  }

  .entry-content h3 a {
    color: var(--accent);
    text-decoration: none;
  }

  .entry-content h3 a:hover {
    text-decoration: underline;
  }

  .entry-description {
    margin: var(--space-2) 0;
    color: var(--text-muted);
    font-size: var(--text-sm);
  }

  .entry-tags {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin-bottom: var(--space-2);
  }

  .tag {
    padding: 2px 6px;
    background: var(--surface-elevated);
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .pending-badge {
    display: inline-block;
    padding: 2px 6px;
    background: var(--warning-subtle);
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
    color: var(--warning);
  }

  .entry-meta {
    display: flex;
    gap: var(--space-3);
    align-items: center;
    margin-top: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .entry-action {
    background: none;
    border: none;
    color: var(--accent);
    cursor: pointer;
    font-size: var(--text-xs);
    text-decoration: underline;
  }

  .entry-action.danger {
    color: var(--danger);
  }

  .empty {
    color: var(--text-muted);
    padding: var(--space-8) 0;
    text-align: center;
  }
</style>
