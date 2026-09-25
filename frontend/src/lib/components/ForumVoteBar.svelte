<script lang="ts">
  /**
   * Per-post typed-vote bar (spec §35.2).
   *
   * Shows the vote types configured for this post's category, each with its
   * count. Clicking casts (or retracts) the caller's vote. The caller's own
   * remaining budget is shown alongside so they can see the cost of a negative
   * vote before making it.
   */
  import {
    castVote,
    getPostVotes,
    getVoteBudget,
    getCategoryVoteTypes,
    ApiError,
    type PostVotesResponse,
  } from '../api';
  import { session } from '../session.svelte.ts';
  import ErrorSummary from './ErrorSummary.svelte';

  interface Props {
    postId: string;
    categoryId: string;
  }

  let { postId, categoryId }: Props = $props();

  interface VoteType {
    id: string;
    label: string;
    weight: number;
    cost: number;
    is_negative: boolean;
  }

  interface VoteCounts {
    vote_type: string;
    count: number;
  }

  let types = $state<VoteType[]>([]);
  let counts = $state<VoteCounts[]>([]);
  let total = $state(0);
  let weightedBp = $state(0);
  let mine = $state<string | null>(null);
  let transparency = $state('aggregate');
  let votes = $state<PostVotesResponse['votes']>(null);
  let budget = $state<{ limit: number; spent: number; remaining: number; exhausted: boolean } | null>(
    null,
  );
  let busy = $state(false);
  let error = $state<unknown>(null);
  let loaded = $state(false);

  $effect(() => {
    void load();
  });

  async function load() {
    error = null;
    try {
      const [typesRes, votesRes, budgetRes] = await Promise.all([
        getCategoryVoteTypes(categoryId),
        getPostVotes(postId),
        session.isSignedIn ? getVoteBudget() : Promise.resolve(null),
      ]);
      types = typesRes.items;
      counts = votesRes.counts;
      total = votesRes.total;
      weightedBp = votesRes.weighted_bp;
      mine = votesRes.mine;
      transparency = votesRes.transparency;
      votes = votesRes.votes;
      if (budgetRes) {
        budget = {
          limit: budgetRes.limit,
          spent: budgetRes.spent,
          remaining: budgetRes.remaining,
          exhausted: budgetRes.exhausted,
        };
      }
      loaded = true;
    } catch (failure) {
      error = failure;
    }
  }

  function countFor(voteType: string): number {
    return counts.find((c) => c.vote_type === voteType)?.count ?? 0;
  }

  async function toggle(voteType: string) {
    if (!session.isSignedIn || busy) return;
    busy = true;
    error = null;
    try {
      // Clicking the active vote retracts it.
      const next = mine === voteType ? null : voteType;
      await castVote(postId, next);
      await load();
    } catch (failure) {
      error = failure;
      if (failure instanceof ApiError && failure.code === 'VOTE_BUDGET_EXHAUSTED') {
        // Let the budget badge update on next load; surface a clear message.
        error = { code: 'VOTE_BUDGET_EXHAUSTED', message: 'You have exhausted your vote budget.' };
      }
    } finally {
      busy = false;
    }
  }
</script>

{#if error}
  <ErrorSummary {error} />
{/if}

{#if loaded}
  <div class="vote-bar" role="group" aria-label="Typed votes">
    {#each types as t (t.id)}
      <button
        type="button"
        class="vote-btn"
        class:pressed={mine === t.id}
        class:negative={t.is_negative}
        onclick={() => toggle(t.id)}
        disabled={!session.isSignedIn || busy}
        title={t.is_negative ? 'Costs more budget' : ''}
      >
        <span class="label">{t.label}</span>
        <span class="count">{countFor(t.id)}</span>
      </button>
    {/each}
  </div>

  {#if total > 0}
    <p class="totals">
      {total} vote{total !== 1 ? 's' : ''}
      {#if weightedBp !== 0}
        · {(weightedBp / 100).toFixed(1)} net
      {/if}
    </p>
  {/if}

  {#if transparency === 'individual_votes' && votes}
    <details class="who-voted">
      <summary>Who voted</summary>
      <ul>
        {#each votes as v}
          <li><span class="pseud">{v.pseud}</span> → {v.vote_type}</li>
        {/each}
      </ul>
    </details>
  {/if}

  {#if budget}
    <div class="budget-badge" class:low={budget.remaining < 5} class:exhausted={budget.exhausted}>
      Budget: {budget.remaining}/{budget.limit}
      {#if budget.exhausted}(exhausted){/if}
    </div>
  {/if}

  {#if !session.isSignedIn}
    <p class="signin-note">Sign in to vote.</p>
  {/if}
{/if}

<style>
  .vote-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin: 0.5rem 0;
  }
  .vote-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.3rem 0.7rem;
    border: 1px solid var(--border, #ccc);
    border-radius: 999px;
    background: var(--surface, #fff);
    color: var(--text, #222);
    font-size: 0.875rem;
    cursor: pointer;
    transition: border-color 0.15s, background 0.15s;
  }
  .vote-btn:hover:not(:disabled) {
    border-color: var(--accent, #4a90d9);
  }
  .vote-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .vote-btn.pressed {
    border-color: var(--accent, #4a90d9);
    background: var(--accent-subtle, #e8f0fe);
    font-weight: 500;
  }
  .vote-btn.negative.pressed {
    border-color: var(--danger, #b00020);
    background: var(--danger-subtle, #fce8e8);
  }
  .label {
    white-space: nowrap;
  }
  .count {
    font-variant-numeric: tabular-nums;
    font-weight: 600;
    min-width: 1.5ch;
    text-align: center;
  }
  .totals {
    font-size: 0.8rem;
    color: var(--text-muted, #666);
    margin: 0.25rem 0;
  }
  .budget-badge {
    display: inline-block;
    font-size: 0.75rem;
    padding: 0.2rem 0.5rem;
    border-radius: 999px;
    background: var(--surface-muted, #f5f5f5);
    color: var(--text-muted, #666);
    margin-top: 0.25rem;
  }
  .budget-badge.low {
    color: var(--warning, #b07d00);
    background: var(--warning-subtle, #fff8e0);
  }
  .budget-badge.exhausted {
    color: var(--danger, #b00020);
    background: var(--danger-subtle, #fce8e8);
  }
  .who-voted {
    font-size: 0.8rem;
    margin-top: 0.5rem;
  }
  .who-voted summary {
    cursor: pointer;
    color: var(--text-muted, #666);
  }
  .who-voted ul {
    margin: 0.25rem 0 0;
    padding-left: 1.25rem;
  }
  .signin-note {
    font-size: 0.8rem;
    color: var(--text-muted, #666);
  }
</style>
