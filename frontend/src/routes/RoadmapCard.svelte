<!--
  One roadmap card in full: title, and the body §44.1 gives it.

  The board lists titles only, because a board of 667 pages of prose is not a
  board. This is where the prose is read, and it is where a voter decides
  whether the feature is the thing they want.

  The body is rendered as PLAIN TEXT, never with {@html}. Bodies are
  operator-authored (docs/requirements.csv), so there is no injection surface
  today — and the rule is written into the component so that the day bodies
  become member-authored, this is the place that has to change, visibly,
  rather than something to discover later.
-->
<script lang="ts">
  import { fetchRoadmapCard, type RoadmapCard } from '../lib/api';
  import { handleLinkClick } from '../lib/router.ts';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  let { params }: { params?: Record<string, string> } = $props();

  let card = $state<RoadmapCard | null>(null);
  let loading = $state(true);
  let error = $state<unknown>(null);

  const STAGE_LABELS: Record<string, string> = {
    idea: 'Idea',
    up_next: 'Up Next',
    in_progress: 'In Progress',
    finished: 'Finished',
    shipped: 'Shipped',
    medium_term: 'Medium Term',
    long_term: 'Long Term',
    rejected: 'Rejected',
  };

  // Re-reads when the route's params change, so navigating from one card
  // straight to another does not need a reload. The empty-prose case is
  // handled below rather than here: an absent body is a supported value, not
  // an error.
  $effect(() => {
    const cardId = params?.cardId;
    if (!cardId) {
      loading = false;
      error = new Error('No card was named in the address.');
      return;
    }
    loading = true;
    error = null;
    card = null;
    fetchRoadmapCard(cardId)
      .then((r) => {
        card = r.card;
      })
      .catch((e) => {
        error = e;
      })
      .finally(() => {
        loading = false;
      });
  });
</script>

<svelte:head>
  <title>{card ? `${card.title} · Roadmap` : 'Roadmap card'} · Lorehaven</title>
</svelte:head>

<div class="card-page">
  <p class="back">
    <a href="/roadmap" onclick={(e) => handleLinkClick(e, '/roadmap')}>← Roadmap</a>
  </p>

  {#if loading}
    <Skeleton lines={8} label="Loading feature card" />
  {:else if error}
    <ErrorSummary {error} />
  {:else if card}
    <article>
      <h1>{card.title}</h1>

      <div class="meta">
        <span class="elo" title="Elo rating">★ {Math.round(card.elo_rating)}</span>
        <span class="stage">{STAGE_LABELS[card.stage] ?? card.stage}</span>
        {#if card.category !== 'general'}
          <span class="category">{card.category}</span>
        {/if}
        {#if card.matches_played > 0}
          <span class="matches">{card.matches_played} votes</span>
        {/if}
      </div>

      <!--
        Plain text, whitespace preserved. `white-space: pre-wrap` is what makes
        a paragraph of prose written in the CSV read as paragraphs rather than
        as one run-on line.
      -->
      {#if (card.body ?? '').trim()}
        <div class="body">{card.body}</div>
      {:else}
        <p class="no-body">
          This card has no description yet. A card with a body explains what the
          feature is and why it exists; this one has only its title.
        </p>
      {/if}
    </article>
  {/if}
</div>

<style>
  .card-page {
    padding: 1.5rem 1rem;
    max-width: 760px;
    margin: 0 auto;
  }

  .back {
    margin-bottom: 1.5rem;
    font-size: 0.875rem;
  }

  h1 {
    margin-bottom: 0.5rem;
    line-height: 1.25;
  }

  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    align-items: center;
    font-size: 0.8125rem;
    color: var(--muted);
    padding-bottom: 1rem;
    border-bottom: 1px solid var(--border);
    margin-bottom: 1.5rem;
  }

  .elo {
    color: var(--accent);
    font-weight: 600;
  }

  .stage,
  .category {
    background: var(--surface);
    padding: 0 0.375rem;
    border-radius: 3px;
  }

  .body {
    white-space: pre-wrap;
    line-height: 1.65;
    font-size: 0.9375rem;
  }

  .no-body {
    color: var(--muted);
    font-style: italic;
    line-height: 1.6;
  }
</style>
