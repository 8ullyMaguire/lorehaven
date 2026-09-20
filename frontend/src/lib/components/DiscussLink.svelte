<script lang="ts">
  /**
   * "Discuss this work" link card (spec §35.0).
   *
   * Shown when the work's discussion mode enables the linked thread. Looks up
   * the topic id and deep-links to it; if none exists yet, links to the
   * category so the reader can start one.
   */
  import { fetchThread, type ThreadResponse } from '../lib/api';
  import { handleLinkClick } from '../lib/router';

  interface Props {
    workId: string;
  }

  let { workId }: Props = $props();

  let thread = $state<ThreadResponse | null>(null);
  let loading = $state(true);

  $effect(() => {
    void load();
  });

  async function load() {
    loading = true;
    try {
      thread = await fetchThread(workId);
    } catch {
      thread = null;
    } finally {
      loading = false;
    }
  }
</script>

{#if !loading && thread}
  <a
    href={`/forum/topics/${thread.topic_id}`}
    class="discuss-link"
    onclick={(event) => handleLinkClick(event, `/forum/topics/${thread.topic_id}`)}
  >
    <span class="discuss-label">Discuss this work</span>
    <span class="discuss-arrow" aria-hidden="true">→</span>
  </a>
{/if}

<style>
  .discuss-link {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    margin: var(--space-3) 0;
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    background: var(--color-surface);
    color: var(--color-text);
    text-decoration: none;
    font-weight: 500;
    transition: border-color 0.15s;
  }

  .discuss-link:hover {
    border-color: var(--color-accent);
  }

  .discuss-arrow {
    transition: transform 0.15s;
  }

  .discuss-link:hover .discuss-arrow {
    transform: translateX(2px);
  }
</style>
