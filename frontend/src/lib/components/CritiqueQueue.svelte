<script lang="ts">
  /**
   * Critique queue (spec §35.3).
   *
   * Shows a topic's critique circle queue. Readers can join the queue
   * and are assigned a position. Used on topics with mode=critique_circle.
   */
  import { getCritiqueQueue, joinCritique, type CritiqueEntry } from '../api';

  let { topicId, isModerator }: { topicId: string; isModerator: boolean } = $props();

  let queue = $state<CritiqueEntry[]>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);
  let joining = $state(false);

  async function load() {
    loading = true;
    error = null;
    try {
      queue = await getCritiqueQueue(topicId);
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  async function join() {
    joining = true;
    error = null;
    try {
      await joinCritique(topicId);
      await load();
    } catch (failure) {
      error = failure;
    } finally {
      joining = false;
    }
  }

  void load();
</script>

<div class="critique-queue">
  <h3>Critique Circle</h3>

  {#if loading}
    <p>Loading…</p>
  {:else if queue.length === 0}
    <p>No one in the queue yet. Join to get feedback on your work!</p>
  {:else}
    <ol>
      {#each queue as entry (entry.position)}
        <li>
          <span class="position">#{entry.position}</span>
          <span class="author">{entry.author_handle ?? entry.author_pseud}</span>
          {#if entry.work_title}
            <span class="work">“{entry.work_title}”</span>
          {/if}
        </li>
      {/each}
    </ol>
  {/if}

  <button onclick={join} disabled={joining}>
    {joining ? 'Joining…' : 'Join critique queue'}
  </button>

  {#if error}
    <p class="error">{error}</p>
  {/if}
</div>

<style>
  .critique-queue {
    margin: 1rem 0;
    padding: 0.75rem;
    border: 1px solid var(--border, #ddd);
    border-radius: 0.5rem;
  }
  ol {
    padding-left: 1.25rem;
  }
  li {
    margin-bottom: 0.5rem;
  }
  .position {
    font-weight: 600;
    margin-right: 0.5rem;
  }
  .author {
    margin-right: 0.5rem;
  }
  .work {
    color: var(--text-muted, #666);
    font-style: italic;
  }
  .error {
    color: var(--error, #c00);
    font-size: 0.875rem;
  }
</style>