<script lang="ts">
  /**
   * The draw (§16.10a, `docs/spec-amendments/draw-slot.md`).
   *
   * Three cards dealt from the reader's own ranked pool. The reader takes one, or
   * asks for another hand — and asking again passes a NEW NONCE, because the same
   * nonce on the same day is the same hand. That is the whole interaction, and it
   * is the part of `ascend`'s draw that was a discovery mechanic: the reveal and
   * the choice. There is no rarity here and no collection, deliberately — D-1
   * forbids a progression economy and a card's "rarity" would be an opinion about
   * the reader dressed as a property of the work.
   *
   * **Every card is already a recorded slot on the server** before it is rendered,
   * so the page does not "record a view": the take is the write, and a hand that
   * was dealt and abandoned is already visible to the north-star metric as three
   * served slots with no `chosen_at`.
   *
   * The pick is idempotent server-side, and this page still treats a second tap as
   * a no-op: `chosenWorkId` is set optimistically and never cleared by a later
   * tap, because the record the server keeps is the FIRST take and a page that
   * showed the second one would be showing a choice that was not recorded.
   */
  import { fetchDraw, pickDrawCard, type DrawHand } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte.ts';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import SignInGate from '../lib/components/SignInGate.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  let hand = $state<DrawHand | null>(null);
  let error = $state<unknown>(null);
  let loading = $state(true);
  let dealing = $state(false);
  let nonce = $state(0);
  /** The work this reader took from THIS hand, or null. */
  let chosenWorkId = $state<string | null>(null);
  /** The take's error, kept apart from the deal's: a failed pick must not blank the hand. */
  let pickError = $state<unknown>(null);
  /** The work the last failed take was for, so a retry re-sends the same pick. */
  let lastPickAttempt = $state<string | null>(null);

  async function deal(nextNonce: number) {
    loading = true;
    error = null;
    pickError = null;
    chosenWorkId = null;
    try {
      hand = await fetchDraw({ nonce: nextNonce });
      nonce = hand.nonce;
    } catch (caught) {
      hand = null;
      error = caught;
    } finally {
      loading = false;
    }
  }

  async function redeal() {
    dealing = true;
    try {
      await deal(nonce + 1);
    } finally {
      dealing = false;
    }
  }

  async function take(workId: string) {
    if (!hand?.requestId || chosenWorkId) return;
    pickError = null;
    lastPickAttempt = workId;
    // Optimistic: the card the reader tapped is marked immediately, and a failure
    // clears it again rather than leaving a take on screen the server never kept.
    chosenWorkId = workId;
    try {
      await pickDrawCard(hand.requestId, workId);
    } catch (caught) {
      chosenWorkId = null;
      pickError = caught;
    }
  }

  $effect(() => {
    if (session.isSignedIn) void deal(0);
  });
</script>

<header class="page-header">
  <h1>Draw</h1>
  <p class="lede">
    Three cards from the works this instance thinks you might read. Take one, or ask for another
    hand.
  </p>
</header>

<SignInGate purpose="draw a hand of recommendations">
{#if error}
  <ErrorSummary {error} onretry={() => deal(nonce)} />
{:else if loading}
  <Skeleton lines={4} label="Dealing" />
{:else if hand && hand.cards.length > 0}
  <ul class="hand">
    {#each hand.cards as card (card.slotId || card.workId)}
      <li class="card" class:taken={chosenWorkId === card.workId}>
        <h2>
          <a href={`/works/${encodeURIComponent(card.workId)}`}
             onclick={(event) =>
               handleLinkClick(event, `/works/${encodeURIComponent(card.workId)}`)}>
            {card.title || 'Untitled work'}
          </a>
        </h2>
        {#if card.authorHandle}
          <p class="meta">by {card.authorHandle}</p>
        {/if}
        <div class="actions">
          {#if chosenWorkId === card.workId}
            <p class="taken-note">Taken. It is in your hands now.</p>
          {:else}
            <button
              type="button"
              onclick={() => take(card.workId)}
              disabled={chosenWorkId !== null}
              aria-label={`Take ${card.title || 'this work'}`}
            >
              Take this one
            </button>
          {/if}
        </div>
      </li>
    {/each}
  </ul>

  {#if pickError}
    <ErrorSummary error={pickError} onretry={() => (lastPickAttempt ? take(lastPickAttempt) : undefined)} />
  {/if}

  <p class="deal-again">
    <button type="button" onclick={redeal} disabled={dealing}>
      {dealing ? 'Dealing…' : 'Deal another hand'}
    </button>
    <span class="meta">
      Hand {nonce + 1} of today. Asking again gives different cards — the same hand is only ever
      dealt once.
    </span>
  </p>
{:else}
  <!--
    The empty state is the server's sentence, not this page's guess: the endpoint
    distinguishes "nothing is published" from "your engines produced nothing", and a
    page that wrote its own wording would collapse the two into one wrong claim.
  -->
  <p class="empty">
    {hand?.emptyReason ?? 'There is nothing to deal right now.'}
  </p>
{/if}
</SignInGate>

<style>
  .hand {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(16rem, 1fr));
    gap: 1rem;
    list-style: none;
    margin: 0 0 1.5rem;
    padding: 0;
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 1rem;
    border: 1px solid var(--color-border, #d8d4cc);
    border-radius: 0.75rem;
    background: var(--color-surface, #fffdf8);
  }

  .card.taken {
    border-color: var(--color-accent, #7a5cff);
  }

  .card h2 {
    font-size: 1.05rem;
    margin: 0;
  }

  .actions {
    margin-top: auto;
  }

  .taken-note {
    margin: 0;
    font-size: 0.9rem;
    color: var(--color-accent, #7a5cff);
  }

  .meta {
    margin: 0;
    font-size: 0.85rem;
    opacity: 0.75;
  }

  .deal-again {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.75rem;
  }

  .empty {
    max-inline-size: 60ch;
  }
</style>
