<script lang="ts">
  /**
   * The reader's own rating of a work (spec §9.5).
   *
   * Two things this component is careful about, both from the spec:
   *
   *  * **A rating is private by default.** Sharing it is a separate, explicit
   *    act, so the checkbox starts unchecked and the copy says what sharing
   *    means (it joins the public average, shown with a count and a method).
   *  * **What the server refuses is not offered.** The control is only shown
   *    to a signed-in reader; a visitor is told that rating needs an account
   *    rather than being shown stars that would fail on click.
   */
  import {
    deleteRating,
    fetchRating,
    upsertRating,
    type RatingView,
  } from '../api';
  import ErrorSummary from './ErrorSummary.svelte';

  interface Props {
    workId: string;
    /** Whether anyone is signed in; a visitor sees a prompt, not stars. */
    signedIn: boolean;
  }

  let { workId, signedIn }: Props = $props();

  let rating = $state<RatingView | null>(null);
  let stars = $state(0);
  let isPublic = $state(false);
  let loading = $state(false);
  let error = $state<unknown>(null);
  let saved = $state(false);

  $effect(() => {
    if (!signedIn) return;
    void load();
  });

  async function load() {
    try {
      rating = await fetchRating(workId);
      if (rating) {
        stars = rating.stars;
        isPublic = rating.is_public;
      }
    } catch {
      // A rating that cannot be read is not an error worth interrupting a
      // reader for: the form still works, it just starts empty.
    }
  }

  async function save() {
    if (stars === 0) return;
    loading = true;
    error = null;
    saved = false;
    try {
      rating = await upsertRating(workId, {
        stars,
        is_public: isPublic,
        expected_version: rating?.version,
      });
      saved = true;
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  async function remove() {
    loading = true;
    error = null;
    try {
      await deleteRating(workId);
      rating = null;
      stars = 0;
      isPublic = false;
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }
</script>

<section class="rating">
  <h2>Your rating</h2>

  {#if !signedIn}
    <p class="note">Sign in to keep a private rating, or to write a review.</p>
  {:else}
    <p class="note">
      A rating is yours alone unless you say otherwise.
      {#if rating && !rating.is_public}
        Sharing it would add it to the work's public average.
      {/if}
    </p>

    <div class="stars" role="radiogroup" aria-label="Your rating, out of five">
      {#each [1, 2, 3, 4, 5] as value (value)}
        <button
          type="button"
          role="radio"
          aria-checked={stars === value}
          aria-label="{value} out of 5"
          class:chosen={stars === value}
          onclick={() => (stars = value)}
        >
          {stars >= value ? '★' : '☆'}
        </button>
      {/each}
    </div>

    <label class="share">
      <input type="checkbox" bind:checked={isPublic} />
      Share this rating publicly
    </label>

    {#if error}
      <ErrorSummary error={error} />
    {/if}

    <div class="actions">
      <button type="button" onclick={save} disabled={loading || stars === 0}>
        {rating ? 'Update rating' : 'Save rating'}
      </button>
      {#if rating}
        <button type="button" class="quiet" onclick={remove} disabled={loading}>
          Remove
        </button>
      {/if}
      {#if saved}<span class="saved" role="status">Saved</span>{/if}
    </div>
  {/if}
</section>

<style>
  .rating {
    margin: var(--space-5) 0;
    padding: var(--space-4);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
  }

  .rating h2 {
    font-size: var(--text-lg);
    margin-top: 0;
  }

  .note {
    color: var(--color-muted);
    font-size: var(--text-sm);
    max-width: 60ch;
  }

  .stars {
    display: flex;
    gap: var(--space-1);
  }

  .stars button {
    font-size: var(--text-xl);
    line-height: 1;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--color-muted);
    padding: var(--space-1);
  }

  .stars button.chosen {
    color: var(--color-accent);
  }

  .share {
    display: block;
    font-size: var(--text-sm);
    margin-top: var(--space-2);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-top: var(--space-3);
  }

  .quiet {
    background: none;
    border: var(--border-width) solid var(--color-border);
    color: var(--color-muted);
  }

  .saved {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }
</style>
