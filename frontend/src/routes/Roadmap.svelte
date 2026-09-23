<script lang="ts">
  import {
    fetchArenaBallot,
    fetchRoadmapBoard,
    fetchRoadmapChangelog,
    submitRoadmapVote,
    suggestFeature,
    type ArenaBallot,
    type RoadmapBoard,
    type RoadmapChangelog,
  } from '../lib/api';
  import { handleLinkClick } from '../lib/router.ts';
  import { session } from '../lib/session.svelte.ts';
  import Button from '../lib/components/Button.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  // Stage columns in canonical order.
  const STAGES = ['idea', 'up_next', 'in_progress', 'finished', 'shipped', 'rejected'] as const;
  const STAGE_LABELS: Record<string, string> = {
    idea: 'Ideas',
    up_next: 'Up Next',
    in_progress: 'In Progress',
    finished: 'Finished',
    shipped: 'Shipped',
    rejected: 'Rejected',
  };

  let board = $state<RoadmapBoard | null>(null);
  let changelog = $state<RoadmapChangelog | null>(null);
  let ballot = $state<ArenaBallot | null>(null);
  let loading = $state(true);
  let error = $state<unknown>(null);
  let activeTab = $state<'board' | 'arena' | 'changelog'>('board');

  // Arena state.
  let bestId = $state<string | null>(null);
  let worstId = $state<string | null>(null);
  let voting = $state(false);
  let voteError = $state<unknown>(null);

  // Suggestion state.
  let suggestTitle = $state('');
  let suggesting = $state(false);
  let suggestError = $state<unknown>(null);

  async function loadBoard() {
    loading = true;
    error = null;
    try {
      board = await fetchRoadmapBoard();
    } catch (e) {
      error = e;
    } finally {
      loading = false;
    }
  }

  async function loadChangelog() {
    try {
      changelog = await fetchRoadmapChangelog();
    } catch {
      /* non-fatal */
    }
  }

  async function loadBallot() {
    if (!session.isSignedIn) return;
    try {
      ballot = await fetchArenaBallot();
      bestId = null;
      worstId = null;
      voteError = null;
    } catch (e) {
      voteError = e;
    }
  }

  async function submitVote() {
    if (!ballot || !bestId || !worstId) return;
    voting = true;
    voteError = null;
    try {
      await submitRoadmapVote({
        ballot_id: ballot.ballot_id,
        best_id: bestId,
        worst_id: worstId,
      });
      ballot = null;
      await loadBallot();
    } catch (e) {
      voteError = e;
    } finally {
      voting = false;
    }
  }

  async function submitSuggestion() {
    if (!suggestTitle.trim()) return;
    suggesting = true;
    suggestError = null;
    try {
      await suggestFeature({ title: suggestTitle.trim() });
      suggestTitle = '';
      await loadBoard();
    } catch (e) {
      suggestError = e;
    } finally {
      suggesting = false;
    }
  }

  function formatDate(s: string): string {
    try {
      return new Date(s).toLocaleDateString();
    } catch {
      return s;
    }
  }

  // Load on mount.
  $effect(() => {
    loadBoard();
    loadChangelog();
  });

  $effect(() => {
    if (activeTab === 'arena' && !ballot) {
      loadBallot();
    }
  });
</script>

<svelte:head>
  <title>Roadmap · Lorehaven</title>
</svelte:head>

<div class="roadmap-page">
  <header class="roadmap-header">
    <h1>Roadmap</h1>
    <p class="roadmap-tagline">
      Community-ranked features. Vote in the arena, suggest ideas, and watch the board evolve.
    </p>
  </header>

  <nav class="roadmap-tabs" aria-label="Roadmap sections">
    <button
      class="tab"
      class:active={activeTab === 'board'}
      onclick={() => (activeTab = 'board')}
    >
      Board
    </button>
    <button
      class="tab"
      class:active={activeTab === 'arena'}
      onclick={() => (activeTab = 'arena')}
    >
      Arena
    </button>
    <button
      class="tab"
      class:active={activeTab === 'changelog'}
      onclick={() => (activeTab = 'changelog')}
    >
      Changelog
    </button>
  </nav>

  {#if error}
    <ErrorSummary error={error} />
  {/if}

  {#if activeTab === 'board'}
    {#if loading}
      <Skeleton lines={8} label="Loading roadmap board" />
    {:else if board}
      <section class="board" aria-label="Roadmap board">
        {#each STAGES as stage}
          <div class="board-column">
            <h2 class="column-title">{STAGE_LABELS[stage] ?? stage}</h2>
            <div class="column-cards">
              {#if board.board[stage] && board.board[stage].length > 0}
                {#each board.board[stage] as card (card.id)}
                  <article class="card">
                    <h3 class="card-title">{card.title}</h3>
                    <div class="card-meta">
                      <span class="card-elo" title="Elo rating">
                        ★ {Math.round(card.elo_rating)}
                      </span>
                      {#if card.category !== 'general'}
                        <span class="card-category">{card.category}</span>
                      {/if}
                    </div>
                  </article>
                {/each}
              {:else}
                <p class="empty">Nothing here yet.</p>
              {/if}
            </div>
          </div>
        {/each}
      </section>

      {#if session.isSignedIn}
        <section class="suggest-section">
          <h2>Suggest a feature</h2>
          <form
            class="suggest-form"
            onsubmit={(e) => {
              e.preventDefault();
              submitSuggestion();
            }}
          >
            <input
              type="text"
              placeholder="What would you like to see?"
              bind:value={suggestTitle}
              maxlength="200"
            />
            <Button type="submit" disabled={suggesting || !suggestTitle.trim()}>
              {suggesting ? 'Submitting…' : 'Suggest'}
            </Button>
          </form>
          {#if suggestError}
            <ErrorSummary error={suggestError} />
          {/if}
        </section>
      {/if}
    {/if}

  {:else if activeTab === 'arena'}
    <section class="arena-section" aria-label="Arena voting">
      {#if !session.isSignedIn}
        <p class="arena-signin">
          <a href="/sign-in" onclick={(e) => handleLinkClick(e, '/sign-in')}>Sign in</a>
          to vote on upcoming features.
        </p>
      {:else if voteError}
        <ErrorSummary error={voteError} />
        <Button onclick={() => loadBallot()}>Try again</Button>
      {:else if !ballot}
        <Skeleton lines={6} label="Loading arena ballot" />
      {:else if ballot.cards.length < 4}
        <p class="arena-empty">
          Not enough cards in the "idea" stage to form a ballot. Check back after more suggestions!
        </p>
      {:else}
        <div class="ballot-instructions">
          <p>Pick the <strong>most valuable</strong> and <strong>least valuable</strong> of these four.</p>
        </div>
        <div class="ballot-cards">
          {#each ballot.cards as card (card.id)}
            <button
              class="ballot-card"
              class:best={bestId === card.id}
              class:worst={worstId === card.id}
              onclick={() => {
                if (bestId === card.id) {
                  bestId = null;
                } else if (worstId === card.id) {
                  worstId = null;
                } else if (!bestId) {
                  bestId = card.id;
                } else if (!worstId) {
                  worstId = card.id;
                } else {
                  // Replace the worst if both full.
                  worstId = card.id;
                }
              }}
            >
              <h3>{card.title}</h3>
              {#if bestId === card.id}
                <span class="badge best">Most Valuable</span>
              {:else if worstId === card.id}
                <span class="badge worst">Least Valuable</span>
              {/if}
            </button>
          {/each}
        </div>
        <div class="ballot-submit">
          <Button
            onclick={submitVote}
            disabled={!bestId || !worstId || voting}
          >
            {voting ? 'Submitting…' : 'Submit Vote'}
          </Button>
        </div>
      {/if}
    </section>

  {:else if activeTab === 'changelog'}
    <section class="changelog-section" aria-label="Stage move changelog">
      {#if !changelog}
        <Skeleton lines={4} label="Loading changelog" />
      {:else if changelog.moves.length === 0}
        <p class="empty">No moves yet.</p>
      {:else}
        <ul class="changelog-list">
          {#each changelog.moves as move (move.id)}
            <li class="changelog-item">
              <span class="move-date">{formatDate(move.created_at)}</span>
              <span class="move-card">{move.card_title}</span>
              <span class="move-arrow">
                {STAGE_LABELS[move.from_stage] ?? move.from_stage}
                →
                {STAGE_LABELS[move.to_stage] ?? move.to_stage}
              </span>
              {#if move.reason}
                <span class="move-reason">{move.reason}</span>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  {/if}
</div>

<style>
  .roadmap-page {
    padding: 1.5rem 1rem;
    max-width: 1200px;
    margin: 0 auto;
  }

  .roadmap-header {
    text-align: center;
    margin-bottom: 2rem;
  }

  .roadmap-header h1 {
    margin-bottom: 0.5rem;
  }

  .roadmap-tagline {
    color: var(--muted);
    max-width: 500px;
    margin: 0 auto;
  }

  .roadmap-tabs {
    display: flex;
    gap: 0.5rem;
    justify-content: center;
    margin-bottom: 2rem;
    border-bottom: 1px solid var(--border);
    padding-bottom: 0.5rem;
  }

  .tab {
    padding: 0.5rem 1rem;
    border: none;
    background: none;
    cursor: pointer;
    font-weight: 500;
    border-radius: 4px;
  }

  .tab.active {
    background: var(--accent);
    color: white;
  }

  .board {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 1rem;
  }

  .board-column {
    background: var(--surface);
    border-radius: 8px;
    padding: 1rem;
    border: 1px solid var(--border);
  }

  .column-title {
    font-size: 1rem;
    margin-bottom: 1rem;
    padding-bottom: 0.5rem;
    border-bottom: 2px solid var(--accent);
  }

  .column-cards {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .card {
    padding: 0.75rem;
    border-radius: 6px;
    background: var(--bg);
    border: 1px solid var(--border);
  }

  .card-title {
    font-size: 0.875rem;
    margin: 0 0 0.5rem 0;
    line-height: 1.3;
  }

  .card-meta {
    display: flex;
    gap: 0.5rem;
    font-size: 0.75rem;
    color: var(--muted);
  }

  .card-elo {
    color: var(--accent);
    font-weight: 600;
  }

  .card-category {
    background: var(--surface);
    padding: 0 0.25rem;
    border-radius: 3px;
  }

  .empty {
    color: var(--muted);
    font-style: italic;
    font-size: 0.875rem;
  }

  .suggest-section {
    margin-top: 2rem;
    text-align: center;
  }

  .suggest-form {
    display: flex;
    gap: 0.5rem;
    max-width: 500px;
    margin: 1rem auto;
  }

  .suggest-form input {
    flex: 1;
    padding: 0.5rem;
    border: 1px solid var(--border);
    border-radius: 4px;
  }

  .arena-section {
    max-width: 600px;
    margin: 0 auto;
  }

  .ballot-instructions {
    text-align: center;
    margin-bottom: 1.5rem;
    color: var(--muted);
  }

  .ballot-cards {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1rem;
    margin-bottom: 1.5rem;
  }

  .ballot-card {
    padding: 1rem;
    border: 2px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
    cursor: pointer;
    text-align: left;
    transition: border-color 0.2s;
  }

  .ballot-card:hover {
    border-color: var(--accent);
  }

  .ballot-card.best {
    border-color: #4caf50;
    background: #f1f8f4;
  }

  .ballot-card.worst {
    border-color: #f44336;
    background: #fef1f0;
  }

  .ballot-card h3 {
    margin: 0 0 0.5rem 0;
    font-size: 1rem;
  }

  .badge {
    display: inline-block;
    padding: 0.125rem 0.5rem;
    border-radius: 4px;
    font-size: 0.75rem;
    font-weight: 600;
  }

  .badge.best {
    background: #4caf50;
    color: white;
  }

  .badge.worst {
    background: #f44336;
    color: white;
  }

  .ballot-submit {
    text-align: center;
  }

  .arena-signin,
  .arena-empty {
    text-align: center;
    color: var(--muted);
    padding: 2rem 0;
  }

  .changelog-list {
    list-style: none;
    padding: 0;
    max-width: 700px;
    margin: 0 auto;
  }

  .changelog-item {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    padding: 0.75rem;
    border-bottom: 1px solid var(--border);
    align-items: center;
  }

  .move-date {
    color: var(--muted);
    font-size: 0.75rem;
    min-width: 90px;
  }

  .move-card {
    font-weight: 600;
    flex: 1;
    min-width: 150px;
  }

  .move-arrow {
    font-size: 0.875rem;
    color: var(--muted);
  }

  .move-reason {
    font-size: 0.75rem;
    color: var(--muted);
    font-style: italic;
    width: 100%;
    padding-left: 0.5rem;
  }
</style>
