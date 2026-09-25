<script lang="ts">
  import {
    fetchGovernanceState,
    createCategoryProposal,
    fetchChangelog,
    toggleFreeze,
    proposeEntryMod,
    fetchDirectoryEntries,
    type GovernanceState,
    type GovernanceCategory,
    type ChangelogEntry,
    type DirectoryEntry,
  } from '../api';
  import { session } from '../session.svelte.ts';
  import Button from './Button.svelte';
  import Dialog from './Dialog.svelte';
  import ErrorSummary from './ErrorSummary.svelte';
  import Skeleton from './Skeleton.svelte';

  // Trust level thresholds (spec §19.1, §45.2)
  const TL_STEWARD = 4;
  const TL_OPERATOR = 5;

  let governance = $state<GovernanceState | null>(null);
  let entries = $state<DirectoryEntry[]>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);

  // Proposal form state
  let proposalAction = $state<string>('rename');
  let proposalCategory = $state<string>('');
  let proposalPayload = $state<Record<string, unknown>>({});
  let proposalSubmitting = $state(false);

  let selectedChangelog = $state<ChangelogEntry[]>([]);
  let showChangelog = $state(false);

  // Entry moderation
  let modAction = $state<'move' | 'remove'>('move');
  let modTargetCategory = $state<string>('');
  let modSubmitting = $state(false);

  const isSteward = $derived((session.me?.trust_level ?? 0) >= TL_STEWARD);
  const isOperator = $derived((session.me?.trust_level ?? 0) >= TL_OPERATOR);

  async function load() {
    loading = true;
    error = null;
    try {
      const [govResult, entriesResult] = await Promise.all([
        fetchGovernanceState(),
        fetchDirectoryEntries({ limit: 50 }),
      ]);
      governance = govResult;
      entries = entriesResult.items ?? [];
      if (govResult.items.length > 0 && proposalCategory === '') {
        proposalCategory = govResult.items[0].slug;
      }
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  async function handleCreateProposal() {
    if (!proposalCategory || !proposalAction) return;
    proposalSubmitting = true;
    try {
      await createCategoryProposal({
        category_slug: proposalCategory,
        action: proposalAction,
        payload: proposalPayload,
      });
      proposalPayload = {};
      await load();
    } catch (failure) {
      error = failure;
    } finally {
      proposalSubmitting = false;
    }
  }

  async function handleToggleFreeze() {
    try {
      await toggleFreeze();
      await load();
    } catch (failure) {
      error = failure;
    }
  }

  async function handleShowChangelog(category: GovernanceCategory) {
    try {
      const result = await fetchChangelog(category.slug);
      selectedChangelog = result.items;
      showChangelog = true;
    } catch (failure) {
      error = failure;
    }
  }

  async function handleProposeEntryMod(entryId: string) {
    if (modAction === 'move' && !modTargetCategory) return;
    modSubmitting = true;
    try {
      await proposeEntryMod(entryId, modAction, modTargetCategory || undefined);
      await load();
    } catch (failure) {
      error = failure;
    } finally {
      modSubmitting = false;
    }
  }

  $effect(() => {
    if (isSteward) void load();
  });

  const stateBadgeClass = (state: string) => {
    switch (state) {
      case 'active':
        return 'badge-success';
      case 'deprecated':
        return 'badge-warning';
      case 'merged':
        return 'badge-muted';
      default:
        return 'badge-muted';
    }
  };
</script>

{#if !isSteward}
  <div class="governance-closed">
    <p>Category governance is available to Stewards and above only.</p>
  </div>
{:else if loading}
  <Skeleton lines={6} label="Loading governance state" />
{:else if error}
  <ErrorSummary {error} onretry={load} />
{:else if governance}
  <section class="governance">
    <header class="governance-header">
      <h2>Category Governance</h2>
      <div class="governance-controls">
        {#if isOperator}
          <Button
            variant={governance.frozen ? 'secondary' : 'danger'}
            onclick={handleToggleFreeze}
          >
            {governance.frozen ? 'Unfreeze governance' : 'Freeze governance'}
          </Button>
        {/if}
        {#if governance.frozen}
          <span class="frozen-badge">FROZEN</span>
        {/if}
      </div>
    </header>

    <!-- Proposal creation -->
    {#if !governance.frozen}
      <div class="proposal-form">
        <h3>Propose an action</h3>
        <form
          onsubmit={(event) => {
            event.preventDefault();
            void handleCreateProposal();
          }}
        >
          <label>
            Category
            <select bind:value={proposalCategory}>
              {#each governance.items as cat}
                <option value={cat.slug}>{cat.label}</option>
              {/each}
            </select>
          </label>
          <label>
            Action
            <select
              bind:value={proposalAction}
              onchange={() => {
                proposalPayload = {};
              }}
            >
              <option value="rename">Rename</option>
              <option value="merge">Merge into another</option>
              <option value="deprecate">Deprecate</option>
              <option value="create">Create new</option>
            </select>
          </label>

          {#if proposalAction === 'rename'}
            <label>
              New label
              <input
                type="text"
                bind:value={proposalPayload.new_label}
                placeholder="New category name"
              />
            </label>
          {:else if proposalAction === 'merge'}
            <label>
              Target category
              <select bind:value={proposalPayload.target_slug}>
                {#each governance.items.filter((c) => c.slug !== proposalCategory) as cat}
                  <option value={cat.slug}>{cat.label}</option>
                {/each}
              </select>
            </label>
          {:else if proposalAction === 'create'}
            <label>
              New slug
              <input
                type="text"
                bind:value={proposalPayload.slug}
                placeholder="new-category-slug"
              />
            </label>
            <label>
              New label
              <input
                type="text"
                bind:value={proposalPayload.label}
                placeholder="New category name"
              />
            </label>
          {/if}

          <Button
            type="submit"
            disabled={proposalSubmitting || governance.frozen}
          >
            {proposalSubmitting ? 'Proposing...' : 'Propose'}
          </Button>
        </form>
      </div>
    {/if}

    <!-- Category list -->
    <div class="category-list">
      <h3>Categories</h3>
      <table class="governance-table">
        <thead>
          <tr>
            <th>Label</th>
            <th>Slug</th>
            <th>State</th>
            <th>Source</th>
            <th>Open proposals</th>
            <th>Actions</th>
          </tr>
        </thead>
        <tbody>
          {#each governance.items as cat}
            <tr>
              <td>{cat.label}</td>
              <td><code>{cat.slug}</code></td>
              <td><span class="badge {stateBadgeClass(cat.state)}">{cat.state}</span></td>
              <td>{cat.source}</td>
              <td>{cat.open_proposals}</td>
              <td>
                <button
                  type="button"
                  class="link-button"
                  onclick={() => handleShowChangelog(cat)}
                >
                  Changelog
                </button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    <!-- Entry moderation -->
    <div class="entry-moderation">
      <h3>Entry moderation</h3>
      <p class="hint">
        Propose moving or removing a directory entry. Requires quorum.
      </p>
      <div class="mod-form">
        <label>
          Action
          <select bind:value={modAction}>
            <option value="move">Move to category</option>
            <option value="remove">Remove</option>
          </select>
        </label>
        {#if modAction === 'move'}
          <label>
            Target category
            <select bind:value={modTargetCategory}>
              {#each governance.items as cat}
                <option value={cat.slug}>{cat.label}</option>
              {/each}
            </select>
          </label>
        {/if}
      </div>
      <ul class="mod-entries">
        {#each entries as entry}
          <li>
            <span class="entry-title">{entry.title}</span>
            <span class="entry-cat">{entry.category}</span>
            <button
              type="button"
              class="entry-mod-button"
              onclick={() => handleProposeEntryMod(entry.id)}
              disabled={modSubmitting || governance.frozen}
            >
              Propose
            </button>
          </li>
        {/each}
      </ul>
    </div>

    <!-- Changelog. Dialog handles focus trapping, Escape and focus restore. -->
    <Dialog
      open={showChangelog}
      title="Changelog"
      onclose={() => (showChangelog = false)}
    >
      <ul class="changelog-list">
        {#each selectedChangelog as entry}
          <li>
            <span class="changelog-event">{entry.event}</span>
            <span class="changelog-actor">{entry.actor}</span>
            <time>{entry.created_at}</time>
            <pre>{entry.document}</pre>
          </li>
        {/each}
      </ul>
    </Dialog>
  </section>
{/if}

<style>
  .governance {
    margin-top: var(--space-8);
    padding: var(--space-6);
    background: var(--surface);
    border-radius: var(--radius);
    border: 1px solid var(--border);
  }

  .governance-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: var(--space-4);
  }

  .governance-header h2 {
    margin: 0;
  }

  .governance-controls {
    display: flex;
    gap: var(--space-3);
    align-items: center;
  }

  .frozen-badge {
    padding: 4px 8px;
    background: var(--danger-subtle);
    color: var(--danger);
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
    font-weight: 600;
    text-transform: uppercase;
  }

  .proposal-form {
    margin-bottom: var(--space-6);
    padding: var(--space-4);
    background: var(--surface-elevated);
    border-radius: var(--radius);
  }

  .proposal-form h3 {
    margin-top: 0;
  }

  .proposal-form form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .proposal-form label {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .proposal-form input,
  .proposal-form select {
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--background);
    color: var(--text);
  }

  .category-list h3 {
    margin-top: 0;
  }

  .governance-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-sm);
  }

  .governance-table th,
  .governance-table td {
    padding: var(--space-2);
    text-align: left;
    border-bottom: 1px solid var(--border);
  }

  .governance-table th {
    font-weight: 600;
    color: var(--text-muted);
  }

  .badge {
    display: inline-block;
    padding: 2px 8px;
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .badge-success {
    background: var(--success-subtle);
    color: var(--success);
  }

  .badge-warning {
    background: var(--warning-subtle);
    color: var(--warning);
  }

  .badge-muted {
    background: var(--surface-elevated);
    color: var(--text-muted);
  }

  .link-button {
    background: none;
    border: none;
    color: var(--accent);
    cursor: pointer;
    font-size: var(--text-xs);
    text-decoration: underline;
  }

  .entry-moderation {
    margin-top: var(--space-6);
  }

  .hint {
    color: var(--text-muted);
    font-size: var(--text-sm);
  }

  .mod-form {
    display: flex;
    gap: var(--space-4);
    margin-bottom: var(--space-4);
  }

  .mod-form label {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--text-sm);
  }

  .mod-form select {
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--background);
    color: var(--text);
  }

  .mod-entries {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .mod-entries li {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2);
    background: var(--surface-elevated);
    border-radius: var(--radius-sm);
  }

  .entry-title {
    flex: 1;
    font-weight: 500;
  }

  .entry-cat {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .entry-mod-button {
    padding: var(--space-1) var(--space-2);
    background: var(--accent);
    color: white;
    border: none;
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-size: var(--text-xs);
  }

  .entry-mod-button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* Dialog supplies the overlay, panel, header and scroll container. These
     rules style only the changelog content rendered inside it. */
  .changelog-list {
    list-style: none;
    padding: 0;
    margin: 0;
  }
  .changelog-list li {
    padding: var(--space-3);
    border-bottom: 1px solid var(--border);
  }
  .changelog-list time {
    color: var(--text-muted);
  }
  .changelog-list pre {
    margin-top: var(--space-2);
    padding: var(--space-2);
    background: var(--surface-muted);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
    overflow-x: auto;
  }
  .changelog-event {
    font-weight: 600;
    margin-right: var(--space-2);
  }

  .changelog-actor {
    color: var(--text-muted);
    font-size: var(--text-sm);
    margin-right: var(--space-2);
  }

  .governance-closed {
    padding: var(--space-4);
    color: var(--text-muted);
    text-align: center;
  }
</style>
