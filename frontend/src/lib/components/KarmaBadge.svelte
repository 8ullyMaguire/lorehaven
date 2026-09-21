<script lang="ts">
  /**
   * Karma badge (spec §35.2).
   *
   * Shows a pseud's karma: a display number (karma_bp / 1000), the count of
   * votes received, and the weighted total. Karma is a display signal — it
   * does not gate trust, ranking, or credits.
   */
  import { getKarma, type KarmaResponse } from '../api';

  interface Props {
    pseud: string;
  }

  let { pseud }: Props = $props();

  let karma = $state<KarmaResponse | null>(null);
  let error = $state<unknown>(null);
  let loading = $state(true);

  $effect(() => {
    void load();
  });

  async function load() {
    loading = true;
    error = null;
    try {
      karma = await getKarma(pseud);
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }
</script>

{#if loading}
  <span class="karma-badge loading">…</span>
{:else if error}
  <span class="karma-badge error" title="Could not load karma">—</span>
{:else if karma}
  <span class="karma-badge" title="{karma.votes_received} votes received · {karma.weighted_received_bp} weighted bp">
    <span class="star" aria-hidden="true">★</span>
    <span class="score">{karma.karma.toFixed(1)}</span>
  </span>
{/if}

<style>
  .karma-badge {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.2rem 0.5rem;
    border-radius: 999px;
    background: var(--surface-muted, #f5f5f5);
    color: var(--text, #222);
    font-size: 0.875rem;
    font-variant-numeric: tabular-nums;
  }
  .star {
    color: var(--accent, #4a90d9);
    font-size: 0.9em;
  }
  .score {
    font-weight: 600;
  }
  .loading {
    opacity: 0.5;
  }
  .error {
    color: var(--danger, #b00020);
    background: var(--danger-subtle, #fce8e8);
  }
</style>
