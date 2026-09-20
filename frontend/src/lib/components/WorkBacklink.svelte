<script lang="ts">
  /**
   * Backlink card: the work linked to a discussion topic (spec §35.0).
   *
   * Shown on the topic page when the topic was created as a work-linked
   * thread. Lets a reader jump from the discussion back to the work.
   */
  import { fetchLinkedWork, type LinkedWorkResponse } from '../api';
  import { handleLinkClick } from '../router';

  interface Props {
    topicId: string;
  }

  let { topicId }: Props = $props();

  let linked = $state<LinkedWorkResponse | null>(null);
  let loading = $state(true);

  $effect(() => {
    void load();
  });

  async function load() {
    loading = true;
    try {
      linked = await fetchLinkedWork(topicId);
    } catch {
      linked = null;
    } finally {
      loading = false;
    }
  }
</script>

{#if !loading && linked}
  <div class="backlink-card">
    <h3 class="backlink-heading">Linked work</h3>
    <a
      href={`/works/${linked.id}`}
      class="backlink-title"
      onclick={(event) => handleLinkClick(event, `/works/${linked.id}`)}
    >
      {linked.title}
    </a>
    {#if linked.author_handles.length > 0}
      <p class="backlink-authors">
        by {#each linked.author_handles as handle, i (handle)}
          {#if i > 0}<span class="comma">, </span>{/if}<a
            href={`/pseud/${handle}`}
            onclick={(event) => handleLinkClick(event, `/pseud/${handle}`)}
          >@{handle}</a>
        {/each}
      </p>
    {/if}
  </div>
{/if}

<style>
  .backlink-card {
    border: var(--border-width) solid var(--color-border);
    border-left: 3px solid var(--color-accent);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    margin-bottom: var(--space-4);
    background: var(--color-surface);
  }

  .backlink-heading {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--color-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin: 0 0 var(--space-1);
  }

  .backlink-title {
    font-weight: 600;
    color: var(--color-text);
    text-decoration: none;
  }

  .backlink-title:hover {
    text-decoration: underline;
  }

  .backlink-authors {
    margin: var(--space-1) 0 0;
    font-size: var(--text-sm);
    color: var(--color-muted);
  }

  .backlink-authors a {
    color: var(--color-muted);
    text-decoration: none;
  }

  .backlink-authors a:hover {
    text-decoration: underline;
  }
</style>
