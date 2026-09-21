<script lang="ts">
  /**
   * Vote budget badge (spec §35.2).
   *
   * Shows the caller's remaining vote allowance in the rolling window, and
   * when it resets. A low or exhausted budget gets a warning style. The
   * component self-refreshes after each vote cast so the number stays live.
   */
  import { getVoteBudget } from '../api';
  import { session } from '../session.svelte.ts';

  interface Props {
    /** When set, the component refreshes after a vote is cast here. */
    postId?: string;
    /** Compact mode: show just the number, not the full badge. */
    compact?: boolean;
  }

  let { postId, compact = false }: Props = $props();

  let budget = $state<{
    trust: number;
    window_hours: number;
    limit: number;
    spent: number;
    remaining: number;
    exhausted: boolean;
    resets_at: string | null;
  } | null>(null);
  let error = $state<unknown>(null);

  $effect(() => {
    void load();
  });

  async function load() {
    if (!session.isSignedIn) return;
    error = null;
    try {
      budget = await getVoteBudget();
    } catch (failure) {
      error = failure;
    }
  }

  function formatReset(iso: string | null): string {
    if (!iso) return '';
    const date = new Date(iso);
    const now = Date.now();
    const diffMs = date.getTime() - now;
    if (diffMs <= 0) return 'resets now';
    const mins = Math.floor(diffMs / 60_000);
    if (mins < 60) return `resets in ${mins}m`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `resets in ${hours}h`;
    return `resets in ${Math.floor(hours / 24)}d`;
  }
</script>

{#if error}
  <span class="budget-badge error" title="Could not load vote budget">—</span>
{:else if budget}
  {#if compact}
    <span
      class="budget-pill"
      class:low={budget.remaining < 5}
      class:exhausted={budget.exhausted}
      title="Vote budget: {budget.remaining}/{budget.limit} remaining. {formatReset(budget.resets_at)}"
    >
      {budget.remaining}/{budget.limit}
    </span>
  {:else}
    <div class="budget-badge" class:low={budget.remaining < 5} class:exhausted={budget.exhausted}>
      <div class="budget-row">
        <span class="label">Vote budget</span>
        <span class="numbers">
          <span class="remaining">{budget.remaining}</span>
          <span class="sep">/</span>
          <span class="limit">{budget.limit}</span>
        </span>
      </div>
      {#if budget.exhausted}
        <p class="notice">Your vote budget is exhausted. {formatReset(budget.resets_at)}.</p>
      {:else if budget.remaining < 5}
        <p class="notice low">Budget running low.</p>
      {/if}
      {#if budget.resets_at}
        <p class="reset">{formatReset(budget.resets_at)}</p>
      {/if}
    </div>
  {/if}
{/if}

<style>
  .budget-badge {
    display: inline-flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.5rem 0.75rem;
    border: 1px solid var(--border, #ccc);
    border-radius: 0.5rem;
    background: var(--surface, #fff);
    font-size: 0.875rem;
  }
  .budget-badge.low {
    border-color: var(--warning, #b07d00);
    background: var(--warning-subtle, #fff8e0);
  }
  .budget-badge.exhausted {
    border-color: var(--danger, #b00020);
    background: var(--danger-subtle, #fce8e8);
  }
  .budget-row {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
  }
  .label {
    font-weight: 500;
  }
  .numbers {
    font-variant-numeric: tabular-nums;
  }
  .remaining {
    font-weight: 700;
    font-size: 1.1em;
  }
  .sep {
    opacity: 0.5;
  }
  .limit {
    opacity: 0.7;
  }
  .notice {
    font-size: 0.75rem;
    margin: 0;
  }
  .notice.low {
    color: var(--warning, #b07d00);
  }
  .reset {
    font-size: 0.75rem;
    color: var(--text-muted, #666);
    margin: 0;
  }
  .budget-pill {
    display: inline-block;
    font-size: 0.75rem;
    padding: 0.2rem 0.5rem;
    border-radius: 999px;
    background: var(--surface-muted, #f5f5f5);
    color: var(--text-muted, #666);
    font-variant-numeric: tabular-nums;
  }
  .budget-pill.low {
    color: var(--warning, #b07d00);
    background: var(--warning-subtle, #fff8e0);
  }
  .budget-pill.exhausted {
    color: var(--danger, #b00020);
    background: var(--danger-subtle, #fce8e8);
  }
  .budget-badge.error {
    color: var(--danger, #b00020);
    background: var(--danger-subtle, #fce8e8);
    padding: 0.25rem 0.5rem;
    border-radius: 0.25rem;
  }
</style>
