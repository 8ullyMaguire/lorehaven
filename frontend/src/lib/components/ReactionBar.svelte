<script lang="ts">
  /**
   * Typed-vote reaction bar for a work page (spec §35.1).
   *
   * One vote per pseud, changeable and retractable. The server owns the
   * outcome (cast/changed/retracted); this component just reflects it.
   */
  import {
    fetchReactions,
    postReaction,
    type ReactionsResponse,
  } from '../lib/api';
  import { describeReactionType, reactionGlyph } from '../lib/labels';
  import { session } from '../lib/session.svelte';

  interface Props {
    workId: string;
  }

  let { workId }: Props = $props();

  let reactions = $state<ReactionsResponse | null>(null);
  let error = $state<unknown>(null);
  let busy = $state(false);

  $effect(() => {
    void load();
  });

  async function load() {
    error = null;
    try {
      reactions = await fetchReactions(workId);
    } catch (failure) {
      error = failure;
    }
  }

  async function vote(voteType: string) {
    if (!session.isSignedIn || busy) return;
    busy = true;
    error = null;
    try {
      // Clicking the active vote retracts it.
      const next = reactions?.mine === voteType ? null : voteType;
      const res = await postReaction(workId, next);
      reactions = await fetchReactions(workId);
      void res;
    } catch (failure) {
      error = failure;
    } finally {
      busy = false;
    }
  }
</script>

{#if reactions}
  <div class="reaction-bar" role="group" aria-label="Reactions">
    {#each reactions.types as voteType (voteType)}
      <button
        type="button"
        class="reaction"
        class:active={reactions.mine === voteType}
        onclick={() => vote(voteType)}
        disabled={!session.isSignedIn || busy}
        title={describeReactionType(voteType)}
      >
        <span class="glyph" aria-hidden="true">{reactionGlyph(voteType)}</span>
        <span class="label">{describeReactionType(voteType)}</span>
        <span class="count">{reactions.counts[voteType] ?? 0}</span>
      </button>
    {/each}
  </div>
  {#if !session.isSignedIn}
    <p class="note">Sign in to react.</p>
  {:else if error}
    <p class="error">Could not record your reaction.</p>
  {/if}
{/if}

<style>
  .reaction-bar {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin: var(--space-3) 0;
  }

  .reaction {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-full);
    background: var(--color-surface);
    color: var(--color-text);
    font-size: var(--text-sm);
    cursor: pointer;
    transition: border-color 0.15s, background 0.15s;
  }

  .reaction:hover:not(:disabled) {
    border-color: var(--color-accent);
  }

  .reaction:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .reaction.active {
    border-color: var(--color-accent);
    background: var(--color-accent-subtle, color-mix(in srgb, var(--color-accent) 15%, var(--color-surface)));
  }

  .glyph {
    font-size: 1.1em;
    line-height: 1;
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

  .note {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .error {
    color: var(--color-danger, #b00020);
    font-size: var(--text-sm);
  }
</style>
