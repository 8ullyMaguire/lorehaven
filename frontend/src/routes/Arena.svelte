<script lang="ts">
  /**
   * Taste Calibration Arena (spec §0.4.2a).
   *
   * Presents 4 works sharing at least one major attribute. The reader picks
   * best and worst; the forced tradeoff reveals which dimensions of taste
   * matter most. After voting, an optional "why?" micro-tag lets the reader
   * label the dimension that drove the choice.
   */
  import {
    fetchArenaNext,
    submitArenaVote,
    dismissArena,
    fetchArenaWeights,
    type ArenaRound,
    type DimensionSummary,
    type DimensionWeightSummary,
  } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import Button from '../lib/components/Button.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  let round = $state<ArenaRound | null>(null);
  let dimensions = $state<DimensionSummary[]>([]);
  let weights = $state<DimensionWeightSummary[]>([]);
  let error = $state<unknown>(null);
  let loading = $state(true);
  let voting = $state(false);
  let dismissed = $state(false);
  let bestId = $state<string | null>(null);
  let worstId = $state<string | null>(null);
  let showWhy = $state(false);
  let selectedReasons = $state<string[]>([]);

  const REASON_OPTIONS = [
    { key: 'prose', label: 'Prose' },
    { key: 'pacing', label: 'Pacing' },
    { key: 'characters', label: 'Characters' },
    { key: 'premise', label: 'Premise' },
    { key: 'vibe', label: 'Vibe' },
  ];

  async function load() {
    loading = true;
    error = null;
    try {
      const res = await fetchArenaNext();
      round = res.round;
      dimensions = res.dimensions;
      // Also fetch weights summary.
      try {
        const w = await fetchArenaWeights();
        weights = w.dimensions;
      } catch {
        // Weights are optional.
      }
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  function selectBest(id: string) {
    bestId = id;
    if (worstId === id) worstId = null;
  }

  function selectWorst(id: string) {
    worstId = id;
    if (bestId === id) bestId = null;
  }

  function toggleReason(key: string) {
    if (selectedReasons.includes(key)) {
      selectedReasons = selectedReasons.filter((r) => r !== key);
    } else {
      selectedReasons = [...selectedReasons, key];
    }
  }

  async function submitVote() {
    if (!bestId || !worstId) return;
    voting = true;
    error = null;
    try {
      const res = await submitArenaVote({
        best_work_id: bestId,
        worst_work_id: worstId,
        reason_tags: selectedReasons,
      });
      if (res.next_round) {
        round = res.next_round;
        bestId = null;
        worstId = null;
        selectedReasons = [];
        showWhy = false;
      } else {
        // No more rounds — reload to get a new one or show completion.
        await load();
      }
    } catch (failure) {
      error = failure;
    } finally {
      voting = false;
    }
  }

  async function dismiss() {
    try {
      await dismissArena();
      dismissed = true;
    } catch (failure) {
      error = failure;
    }
  }

  $effect(() => {
    void load();
  });

  function wordCountLabel(wc: number): string {
    if (wc >= 1000) return `${(wc / 1000).toFixed(1)}k`;
    return wc.toString();
  }
</script>

<section class="arena">
  <header class="arena-header">
    <h1>Taste Calibration Arena</h1>
    <p class="lede">
      Pick the work you'd most want to read and the one you'd least want to
      read. The comparison reveals what matters most to you — not just
      whether a work is "good."
    </p>
  </header>

  {#if error}
    <ErrorSummary {error} />
  {/if}

  {#if dismissed}
    <div class="arena-dismissed">
      <p>Arena dismissed. You can always return later from the Discover page.</p>
      <a href="/discover" onclick={(event) => handleLinkClick(event, "/discover")}>Back to Discover</a>
    </div>
  {:else if loading}
    <Skeleton lines={6} label="Loading arena round" />
  {:else if round && round.cards.length >= 4}
    <div class="arena-cards">
      {#each round.cards as card (card.work_id)}
        <div
          class="arena-card"
          class:selected-best={bestId === card.work_id}
          class:selected-worst={worstId === card.work_id}
        >
          <div class="card-header">
            <h3>{card.title}</h3>
            <span class="fandom">{card.fandom}</span>
          </div>
          <div class="card-meta">
            <span>{wordCountLabel(card.word_count)} words</span>
            {#each card.tags.slice(0, 3) as tag}
              <span class="tag">{tag}</span>
            {/each}
          </div>
          <p class="excerpt">{card.excerpt}</p>
          <div class="card-actions">
            <Button
              variant={bestId === card.work_id ? 'primary' : 'secondary'}
              size="sm"
              onclick={() => selectBest(card.work_id)}
            >
              {bestId === card.work_id ? '✓ Best' : 'Best'}
            </Button>
            <Button
              variant={worstId === card.work_id ? 'danger' : 'secondary'}
              size="sm"
              onclick={() => selectWorst(card.work_id)}
            >
              {worstId === card.work_id ? '✗ Worst' : 'Worst'}
            </Button>
          </div>
        </div>
      {/each}
    </div>

    {#if bestId && worstId}
      {#if !showWhy}
        <div class="arena-why-prompt">
          <button class="link" onclick={() => (showWhy = true)}>
            Why? (optional, 2 seconds)
          </button>
        </div>
      {:else}
        <div class="arena-why">
          <p>What drove your choice?</p>
          <div class="reason-tags">
            {#each REASON_OPTIONS as reason}
              <button
                class="reason-tag"
                class:selected={selectedReasons.includes(reason.key)}
                onclick={() => toggleReason(reason.key)}
              >
                {reason.label}
              </button>
            {/each}
          </div>
        </div>
      {/if}

      <div class="arena-submit">
        <Button variant="primary" onclick={submitVote} disabled={!bestId || !worstId || voting}>
          {voting ? 'Recording...' : 'Submit & Next Round'}
        </Button>
      </div>
    {/if}

    {#if dimensions.length > 0}
      <div class="arena-progress">
        <h2>Your Calibration Progress</h2>
        <ul>
          {#each dimensions as dim}
            <li>
              <span class="dim-label">{dim.label}</span>
              <span class="dim-matches">{dim.matches_played} rounds</span>
            </li>
          {/each}
        </ul>
      </div>
    {/if}

    {#if weights.length > 0}
      <div class="arena-weights">
        <h2>What Matters to You</h2>
        <ul>
          {#each weights as w}
            <li>
              <span class="weight-label">{w.label}</span>
              <span class="weight-influence">{w.influence}</span>
            </li>
          {/each}
        </ul>
      </div>
    {/if}

    <div class="arena-dismiss">
      <button class="link" onclick={dismiss}>
        Skip arena for now
      </button>
    </div>
  {:else}
    <div class="arena-empty">
      <p>Not enough works in the archive for an arena round yet.</p>
      <a href="/discover" onclick={(event) => handleLinkClick(event, "/discover")}>Back to Discover</a>
    </div>
  {/if}
</section>

<style>
  .arena {
    max-width: 900px;
    margin: 0 auto;
    padding: 1rem;
  }

  .arena-header h1 {
    margin-bottom: 0.5rem;
  }

  .lede {
    color: var(--text-muted);
    margin-bottom: 1.5rem;
  }

  .arena-cards {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 1rem;
    margin-bottom: 1.5rem;
  }

  .arena-card {
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    transition: border-color 0.15s;
  }

  .arena-card.selected-best {
    border-color: var(--success);
    background: var(--success-bg);
  }

  .arena-card.selected-worst {
    border-color: var(--danger);
    background: var(--danger-bg);
  }

  .card-header {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }

  .card-header h3 {
    margin: 0;
    font-size: 1rem;
  }

  .fandom {
    font-size: 0.8rem;
    color: var(--text-muted);
  }

  .card-meta {
    display: flex;
    gap: 0.5rem;
    font-size: 0.8rem;
    color: var(--text-muted);
  }

  .tag {
    background: var(--tag-bg);
    padding: 0.1rem 0.4rem;
    border-radius: 4px;
  }

  .excerpt {
    font-size: 0.9rem;
    line-height: 1.4;
    color: var(--text);
    flex: 1;
  }

  .card-actions {
    display: flex;
    gap: 0.5rem;
  }

  .arena-why-prompt,
  .arena-submit,
  .arena-dismiss {
    text-align: center;
    margin: 1rem 0;
  }

  .arena-why {
    margin: 1rem 0;
    text-align: center;
  }

  .reason-tags {
    display: flex;
    gap: 0.5rem;
    justify-content: center;
    flex-wrap: wrap;
    margin-top: 0.5rem;
  }

  .reason-tag {
    padding: 0.3rem 0.8rem;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--bg);
    cursor: pointer;
    font-size: 0.85rem;
  }

  .reason-tag.selected {
    background: var(--primary);
    color: white;
    border-color: var(--primary);
  }

  .arena-progress,
  .arena-weights {
    margin-top: 2rem;
    padding-top: 1rem;
    border-top: 1px solid var(--border);
  }

  .arena-progress h2,
  .arena-weights h2 {
    font-size: 1rem;
    margin-bottom: 0.5rem;
  }

  .arena-progress ul,
  .arena-weights ul {
    list-style: none;
    padding: 0;
  }

  .arena-progress li,
  .arena-weights li {
    display: flex;
    justify-content: space-between;
    padding: 0.3rem 0;
  }

  .dim-matches,
  .weight-influence {
    color: var(--text-muted);
    font-size: 0.85rem;
  }

  .link {
    background: none;
    border: none;
    color: var(--primary);
    cursor: pointer;
    text-decoration: underline;
    font-size: 0.9rem;
  }

  .arena-dismissed,
  .arena-empty {
    text-align: center;
    padding: 2rem;
  }
</style>
