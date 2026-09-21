<script lang="ts">
  /**
   * Community hub: forums, groups, and messaging (spec §17).
   *
   * The community surfaces are real. Forums have categories, topics, and replies.
   * Groups have membership, roles, and visibility. Direct messages are block-aware.
   * Presence shows who is online.
   *
   * Tabs switch between the four surfaces; each tab loads its data independently
   * so an empty forums list does not hold up the groups list.
   */
  import {
    fetchBlocks,
    fetchConversations,
    fetchForums,
    fetchGroups,
    type Forum,
    type Group,
    type Conversation,
    type Block,
  } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte.ts';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  type Tab = 'forums' | 'groups' | 'messages' | 'blocks';

  let activeTab = $state<Tab>('forums');
  let forums = $state<Forum[]>([]);
  let groups = $state<Group[]>([]);
  let conversations = $state<Conversation[]>([]);
  let blocks = $state<Block[]>([]);
  let error = $state<unknown>(null);
  let loading = $state(true);

  let filterQuery = $state('');
  let filteredForums = $derived.by(() => {
    if (!filterQuery.trim()) return forums;
    const q = filterQuery.toLowerCase();
    return forums.filter((f) =>
      f.name.toLowerCase().includes(q) ||
      (f.description?.toLowerCase().includes(q) ?? false)
    );
  });

  async function loadForums() {
    try {
      forums = await fetchForums();
    } catch (failure) {
      error = failure;
    }
  }

  async function loadGroups() {
    try {
      groups = await fetchGroups();
    } catch (failure) {
      error = failure;
    }
  }

  async function loadConversations() {
    try {
      conversations = await fetchConversations();
    } catch (failure) {
      error = failure;
    }
  }

  async function loadBlocks() {
    try {
      blocks = await fetchBlocks();
    } catch (failure) {
      error = failure;
    }
  }

  $effect(() => {
    loading = true;
    error = null;
    if (activeTab === 'forums') loadForums().finally(() => loading = false);
    else if (activeTab === 'groups') loadGroups().finally(() => loading = false);
    else if (activeTab === 'messages') loadConversations().finally(() => loading = false);
    else if (activeTab === 'blocks') loadBlocks().finally(() => loading = false);
  });
</script>

<section class="community">
  <header>
    <h1>Community</h1>
    <p class="lede">Forums, groups, and direct messages between readers.</p>
  </header>

  <div class="tabs" role="tablist">
    <button
      role="tab"
      aria-selected={activeTab === 'forums'}
      class:active={activeTab === 'forums'}
      onclick={() => activeTab = 'forums'}
    >
      Forums
    </button>
    <button
      role="tab"
      aria-selected={activeTab === 'groups'}
      class:active={activeTab === 'groups'}
      onclick={() => activeTab = 'groups'}
    >
      Groups
    </button>
    <button
      role="tab"
      aria-selected={activeTab === 'messages'}
      class:active={activeTab === 'messages'}
      onclick={() => activeTab = 'messages'}
      disabled={!session.isSignedIn}
    >
      Messages
    </button>
    <button
      role="tab"
      aria-selected={activeTab === 'blocks'}
      class:active={activeTab === 'blocks'}
      onclick={() => activeTab = 'blocks'}
      disabled={!session.isSignedIn}
    >
      Blocks
    </button>
  </div>

  {#if error}
    <ErrorSummary {error} />
  {/if}

  {#if loading}
    <Skeleton lines={5} label={`Loading ${activeTab}`} />
  {:else}
    {#if activeTab === 'forums'}
      <div class="forum-toolbar">
        <input
          type="search"
          placeholder="Filter forums..."
          class="forum-filter-input"
          bind:value={filterQuery}
        />
        <a href="/community/search" onclick={(e) => handleLinkClick(e, '/community/search')}>
          <button>Search posts</button>
        </a>
      </div>
      <ul class="card-list">
        {#each filteredForums as forum}
          <li>
            <a
              href={`/community/forums/${encodeURIComponent(forum.id)}`}
              onclick={(event) => handleLinkClick(event, `/community/forums/${encodeURIComponent(forum.id)}`)}
            >
              <span class="card-title">{forum.name}</span>
              {#if forum.description}<span class="card-desc">{forum.description}</span>{/if}
            </a>
          </li>
        {:else}
          <li class="empty">No forums yet.</li>
        {/each}
      </ul>
    {:else if activeTab === 'groups'}
      <ul class="card-list">
        {#each groups as group}
          <li>
            <a
              href={`/community/groups/${encodeURIComponent(group.id)}`}
              onclick={(event) => handleLinkClick(event, `/community/groups/${encodeURIComponent(group.id)}`)}
            >
              <span class="card-title">{group.name}</span>
              {#if group.description}<span class="card-desc">{group.description}</span>{/if}
              <span class="card-meta">{group.privacy}</span>
            </a>
          </li>
        {:else}
          <li class="empty">No groups yet.</li>
        {/each}
      </ul>
    {:else if activeTab === 'messages'}
      <ul class="card-list">
        {#each conversations as conv}
          <li>
            <a
              href={`/community/messages/${encodeURIComponent(conv.id)}`}
              onclick={(event) => handleLinkClick(event, `/community/messages/${encodeURIComponent(conv.id)}`)}
            >
              <span class="card-title">{conv.other_handle}</span>
              {#if conv.last_message}<span class="card-desc">{conv.last_message}</span>{/if}
            </a>
          </li>
        {:else}
          <li class="empty">No conversations yet.</li>
        {/each}
      </ul>
    {:else if activeTab === 'blocks'}
      <ul class="card-list">
        {#each blocks as block}
          <li>
            <span class="card-title">{block.blocked}</span>
            <span class="card-meta">blocked</span>
          </li>
        {:else}
          <li class="empty">No blocks.</li>
        {/each}
      </ul>
    {/if}
  {/if}
</section>

<style>
  .community {
    max-width: 72rem;
    margin-inline: auto;
    padding: 2rem 1rem;
  }
  header h1 {
    margin-bottom: 0.25rem;
  }
  .lede {
    color: var(--text-muted);
    margin-bottom: 1.5rem;
  }
  .tabs {
    display: flex;
    gap: 0.25rem;
    border-bottom: 1px solid var(--border);
    margin-bottom: 1.5rem;
  }
  .tabs button {
    padding: 0.5rem 1rem;
    border: none;
    background: none;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    color: var(--text-muted);
  }
  .tabs button.active {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .tabs button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .card-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: grid;
    gap: 0.75rem;
  }
  .card-list a {
    display: flex;
    flex-direction: column;
    padding: 1rem;
    border: 1px solid var(--border);
    border-radius: 0.5rem;
    text-decoration: none;
    color: inherit;
  }
  .card-list a:hover {
    background: var(--surface-hover);
  }
  .card-title {
    font-weight: 600;
  }
  .card-desc {
    color: var(--text-muted);
    font-size: 0.875rem;
    margin-top: 0.25rem;
  }
  .card-meta {
    font-size: 0.75rem;
    color: var(--text-muted);
    margin-top: 0.25rem;
  }
  .empty {
    padding: 2rem;
    text-align: center;
    color: var(--text-muted);
    background: var(--surface-muted);
    border-radius: 0.5rem;
  }
</style>
