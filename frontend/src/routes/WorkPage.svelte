<script lang="ts">
  /**
   * A work's page (spec §9.1).
   *
   * The same URL answers a contributor with the author's view, so this page
   * says which one it is rather than pretending: a contributor gets a note and
   * a link to the editor, everyone else gets the reading page.
   *
   * It is also where the reader's own things appear — a resume offer, a private
   * rating, the public reviews — and the rule for all three is the same: the
   * page must not offer what the server will refuse. A visitor is told that
   * rating needs an account rather than shown stars that would fail on click.
   */
  import {
    fetchReviews,
    fetchWork,
    fetchWorkPricing,
    getProgress,
    isAuthorWork,
    purchaseWork,
    upsertReview,
    type AuthorWork,
    type ProgressView,
    type PublicWork,
    type PublicPricingResponse,
    type ReviewView,
  } from '../lib/api';
  import { ApiError } from '../lib/api';
  import { describeCompletion, describeLifecycle, describeRating, describeVisibility } from '../lib/labels';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import NotePanel from '../lib/components/NotePanel.svelte';
  import Rating from '../lib/components/Rating.svelte';
  import ResumePrompt from '../lib/components/ResumePrompt.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  interface GalleryItem {
    id: string;
    media_type: string;
    storage_key: string;
    alt_text: string;
  }

  interface Props {
    workId: string;
  }

  let { workId }: Props = $props();

  let work = $state<PublicWork | AuthorWork | null>(null);
  let error = $state<unknown>(null);
  let loading = $state(true);

  // Paywall state: set when fetchWork returns 403 CONTENT_RESTRICTED.
  let paywall = $state<PublicPricingResponse | null>(null);

  let progress = $state<ProgressView | null>(null);
  let reviews = $state<ReviewView[]>([]);
  let reviewDraft = $state('');
  let reviewPublic = $state(false);
  let reviewError = $state<unknown>(null);

  const authorView = $derived(work !== null && isAuthorWork(work) ? (work as AuthorWork) : null);
  const firstChapterId = $derived(work && work.chapters.length > 0 ? work.chapters[0].id : null);

  $effect(() => {
    void load();
  });

  async function load() {
    loading = true;
    error = null;
    try {
      work = await fetchWork(workId);
      // Public reviews are readable by anyone; a failure here must not hide
      // the work itself.
      try {
        reviews = (await fetchReviews(workId)).items;
      } catch {
        reviews = [];
      }
      if (session.isSignedIn) {
        try {
          progress = await getProgress('work', workId);
        } catch {
          progress = null;
        }
      }
      void loadGallery();
    } catch (failure) {
      // Paywall: a priced work returns 403 CONTENT_RESTRICTED for non-buyers.
      // Fetch public pricing so we can render a buy screen.
      if (failure instanceof ApiError && failure.code === 'CONTENT_RESTRICTED') {
        try {
          paywall = await fetchWorkPricing(workId);
        } catch {
          paywall = null;
        }
      }
      error = failure;
      work = null;
    } finally {
      loading = false;
    }
  }

  async function loadGallery() {
    try {
      const res = await apiFetch<{ items: GalleryItem[] }>(
        `/works/${workId}/gallery`,
      );
      gallery = res.items;
    } catch {
      gallery = null;
    }
  }

  let gallery = $state<GalleryItem[] | null>(null);
  let reviewReceipt = $state<string | null>(null);

  async function publishReview() {
    if (reviewDraft.trim() === '') return;
    reviewError = null;
    reviewReceipt = null;
    try {
      const saved = await upsertReview(workId, { body: reviewDraft, is_public: reviewPublic });
      // The receipt is the only delivery signal the server sends (spec §12.4):
      // posted vs held for review, never the class or the author's settings.
      reviewReceipt = saved.receipt ?? null;
      reviewDraft = '';
      reviewPublic = false;
      reviews = (await fetchReviews(workId)).items;
    } catch (failure) {
      reviewError = failure;
    }
  }
</script>

{#if loading}
  <Skeleton lines={4} />
{:else if paywall}
  <!-- Paywall (spec §20.9): work exists and is priced, but the reader
       has not purchased it. Show the price and a buy button. -->
  <h1>This work is for purchase</h1>
  <p class="paywall-price">
    {#each paywall.pricing as p (p.currency)}
      {#if p.model === 'purchase'}
        {Math.round(p.price_minor / 100)} {p.currency}
      {/if}
    {/each}
  </p>
  <p>Buy to unlock full access.</p>
  <button
    type="button"
    disabled={!session.isSignedIn}
    onclick={async () => {
      if (!session.isSignedIn) return;
      try {
        await purchaseWork(workId);
        paywall = null;
        void load();
      } catch (failure) {
        error = failure;
      }
    }}
  >
    {#if !session.isSignedIn}
      Sign in to buy
    {:else}
      Buy now
    {/if}
  </button>
{:else if error}
  <h1>Not found</h1>
  <ErrorSummary {error} />
  <p>
    <a href="/" onclick={(event) => handleLinkClick(event, '/')}>Back to the front page</a>
  </p>
{:else if work}
  <!--
    Bound once, as a `const`.

    `work.id` inside an `onclick` closure was an error — "'work' is possibly
    'null'" — because `work` is a `let` binding and TypeScript will not carry a
    narrowing from the surrounding `{#if}` into a callback that might run after
    it changed. The `href` attribute on the same element read it happily, which
    is what made the errors look arbitrary. A `{@const}` is a fresh binding whose
    narrowed type is fixed at the point of declaration, so both readers see it
    the same way.
  -->
  {@const workHref = `/works/${work.id}`}
  {@const editorHref = `/write/${work.id}`}

  {#if authorView}
    <p class="draft-note" role="status">
      {#if authorView.lifecycle === 'published'}
        You contribute to this work, so you see it here along with its editing controls.
      {:else}
        This work is {describeLifecycle(authorView.lifecycle).toLowerCase()}, so only its
        contributors can see it. Others see it once it is published.
      {/if}
      <a href={editorHref} onclick={(event) => handleLinkClick(event, editorHref)}>Edit it</a>
    </p>
  {/if}

  <h1>{work.title.trim() === '' ? 'Untitled' : work.title}</h1>

  <p class="byline">
    {#each (work as PublicWork).authors ?? [] as author, index (author.handle)}
      {#if index > 0}· {/if}<a href={`/pseud/${author.handle}`} onclick={(event) => handleLinkClick(event, `/pseud/${author.handle}`)}>@{author.handle}</a>
    {/each}
  </p>

  <p class="badges">
    <span>{describeRating(work.rating)}</span>
    <span>{describeVisibility(work.visibility)}</span>
    <span>{describeCompletion(work.completion)}</span>
    {#if work.published_at}<span>Published {work.published_at.slice(0, 10)}</span>{/if}
  </p>

  {#if work.summary.trim() !== ''}
    <p class="summary">{work.summary}</p>
  {/if}

  {#if gallery && gallery.length > 0}
    <h2>Gallery</h2>
    <ul class="gallery">
      {#each gallery as item (item.id)}
        <li>
          <figure>
            {#if item.media_type === 'image'}
              <img src={`/api/v1/media/files/${item.storage_key}`} alt={item.alt_text} loading="lazy" />
            {:else if item.media_type === 'audio'}
              <audio controls src={`/api/v1/media/files/${item.storage_key}`}>
                <track kind="descriptions" label={item.alt_text} />
              </audio>
            {:else}
              <a href={`/api/v1/media/files/${item.storage_key}`} download>{item.alt_text}</a>
            {/if}
            {#if item.alt_text.trim() !== ''}
              <figcaption>{item.alt_text}</figcaption>
            {/if}
          </figure>
        </li>
      {/each}
    </ul>
  {/if}

  {#if progress && progress.resolution.kind !== 'no_position'}
    <ResumePrompt workId={workId} resolution={progress.resolution} firstChapterId={firstChapterId} />
  {/if}

  <h2>Chapters</h2>
  {#if work.chapters.length === 0}
    <p class="note">This work has no chapters.</p>
  {:else}
    <ol class="chapters">
      {#each work.chapters as chapter (chapter.id)}
        {@const chapterHref = `${workHref}/chapters/${chapter.id}`}
        <li>
          <a href={chapterHref} onclick={(event) => handleLinkClick(event, chapterHref)}>
            {chapter.title.trim() === '' ? 'Untitled chapter' : chapter.title}
          </a>
          <span class="note">{chapter.word_count} words</span>
        </li>
      {/each}
    </ol>
  {/if}

  <div id="rate">
    <Rating workId={workId} signedIn={session.isSignedIn} />
  </div>

  <section class="reviews">
    <h2>Reviews</h2>
    {#if reviews.length === 0}
      <p class="note">No public reviews yet. A review stays private until its writer publishes it.</p>
    {:else}
      <ul>
        {#each reviews as review (review.id)}
          <li>
            <p class="review-author">@{review.author_handle}</p>
            {#if review.contains_spoilers}
              <!-- Spoiler reveal is a deliberate click, never automatic (§9.2). -->
              <details>
                <summary>This review mentions spoilers</summary>
                <p>{review.body}</p>
              </details>
            {:else}
              <p>{review.body}</p>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}

    {#if session.isSignedIn}
      <h3>Your review</h3>
      <label for="review-body" class="label">What did you think?</label>
      <textarea id="review-body" rows="3" bind:value={reviewDraft}></textarea>
      <label class="share">
        <input type="checkbox" bind:checked={reviewPublic} />
        Publish this review under @{session.activePseud?.handle ?? 'your pseud'}
      </label>
      {#if reviewError}
        <ErrorSummary error={reviewError} />
      {/if}
      {#if reviewReceipt}
        <p class="note" role="status">{reviewReceipt}</p>
      {/if}
      <button type="button" onclick={publishReview} disabled={reviewDraft.trim() === ''}>
        Save review
      </button>
    {:else}
      <p class="note">Sign in to write a review.</p>
    {/if}
  </section>

  {#if session.isSignedIn}
    <NotePanel subjectType="work" subjectId={workId} signedIn={session.isSignedIn} />
  {/if}
{/if}

<style>
  .byline {
    font-size: var(--text-lg);
  }

  .badges {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .summary {
    max-width: 62ch;
  }

  .draft-note {
    background: var(--color-surface);
    border: var(--border-width) solid var(--color-accent);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    font-size: var(--text-sm);
  }

  .chapters {
    padding-left: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .reviews ul {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .reviews li {
    border-left: 2px solid var(--color-border);
    padding-left: var(--space-3);
  }

  .review-author {
    color: var(--color-muted);
    font-size: var(--text-sm);
    margin: 0 0 var(--space-1);
  }

  .reviews textarea {
    width: 100%;
    max-width: 60ch;
    font: inherit;
    color: var(--color-text);
    background: var(--color-bg);
    border: var(--border-width) solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    padding: var(--space-2);
  }

  .label {
    display: block;
    margin: var(--space-3) 0 var(--space-1);
  }

  .share {
    display: block;
    font-size: var(--text-sm);
    margin: var(--space-2) 0;
  }

  .note {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .gallery {
    list-style: none;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: var(--space-3);
    margin: var(--space-4) 0;
  }

  .gallery figure {
    margin: 0;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--color-border);
    border-radius: 8px;
    overflow: hidden;
  }

  .gallery img {
    width: 100%;
    height: 200px;
    object-fit: cover;
  }

  .gallery audio {
    width: 100%;
    padding: var(--space-2);
  }

  .gallery figcaption {
    padding: var(--space-2);
    font-size: var(--text-sm);
    color: var(--color-muted);
    background: var(--color-surface);
  }

  .paywall-price {
    font-size: var(--text-xl);
    font-weight: bold;
    color: var(--color-text);
  }
</style>
