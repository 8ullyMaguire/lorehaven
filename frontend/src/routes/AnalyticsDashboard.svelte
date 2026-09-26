<script lang="ts">
  /**
   * The reader's own analytics, rendered from what the server allows.
   *
   * Two rules this component follows, and both are privacy rules rather than
   * style choices:
   *
   * 1. **No local capability list.** The page renders exactly what
   *    `GET /me/analytics` returns. A component that knew the trust ladder
   *    would be a second, client-side copy of the registry — and the copy
   *    would be the one that is wrong when the operator changes a preset.
   *
   * 2. **No hardcoded floor.** "Fewer than 10" is assembled from the
   *    `fewer_than` the server sent, never from a constant. A client that
   *    bakes in 10 will say "fewer than 10" on a surface whose floor is 5,
   *    which is both wrong and more revealing than the truth.
   *
   * The second point is why a suppressed count is not stored as `0`. The
   * server omits the field rather than sending zero, because zero is a
   * claim — "nobody read this" — and for a work published an hour ago the
   * true statement is "too few people to tell you".
   */
  import { onMount } from 'svelte';
  import {
    fetchAnalytics,
    fetchCapability,
    type AnalyticsMeta,
    type AnalyticsDetail,
    type ReadingTotals,
    ApiError,
  } from '../lib/api';

  let loading = true;
  let error: string | null = null;
  let trustLevel = 0;
  let role = 'reader';
  let preset = 'archive';
  let capabilities: AnalyticsMeta[] = [];

  /**
   * Set when the server refused because nobody is signed in.
   *
   * A 401 on this door is not an error to report, it is the answer to "is this
   * page for me?" — and rendering it as a failure string sends a signed-out
   * visitor to a message they can do nothing about. The door is
   * `RequirePseud`, so the page is about *this* reader's own numbers and there
   * is nothing to show an anonymous caller.
   */
  let signedOut = false;

  /** The capability whose value is expanded, if any. */
  let open: string | null = null;
  let detail: AnalyticsDetail | null = null;
  let detailLoading = false;

  onMount(async () => {
    try {
      const list = await fetchAnalytics();
      trustLevel = list.viewer.trust_level;
      role = list.viewer.role;
      preset = list.viewer.preset;
      capabilities = list.capabilities;
    } catch (e) {
      if (e instanceof ApiError && (e.status === 401 || e.status === 403)) {
        signedOut = true;
      } else {
        error = e instanceof Error ? e.message : 'Could not load your analytics.';
      }
    } finally {
      loading = false;
    }
  });

  async function toggle(meta: AnalyticsMeta) {
    if (open === meta.name) {
      open = null;
      detail = null;
      return;
    }
    open = meta.name;
    detail = null;
    detailLoading = true;
    try {
      detail = await fetchCapability(meta.name);
    } catch (e) {
      error = e instanceof Error ? e.message : `Could not load ${meta.name}.`;
    } finally {
      detailLoading = false;
    }
  }

  /**
   * A count as text, or `null` when the server suppressed it.
   *
   * Returning `null` rather than "0" is the point. Every caller has to decide
   * what to do with a missing number, and the only correct answer for these
   * numbers is to say the count is below the floor.
   */
  function countText(d: AnalyticsDetail | null): string | null {
    if (!d) return null;
    if (typeof d.value.count === 'number') {
      return d.value.count.toLocaleString();
    }
    if (typeof d.value.fewer_than === 'number') {
      return `fewer than ${d.value.fewer_than.toLocaleString()}`;
    }
    return null;
  }

  /**
   * §9.6's reading totals, or `null` for any other capability.
   *
   * Keyed off the capability's own name rather than off "does the value have
   * these fields", so a capability whose fields happen to overlap cannot be
   * rendered with another one's shape. The server is the only thing that knows
   * which shape a capability answers in; this is the client half of that
   * agreement, and the `detail.capability` string is what both sides key on.
   */
  function readingTotals(d: AnalyticsDetail | null): ReadingTotals | null {
    if (!d || d.capability !== 'own.reading.basic') return null;
    return d.value.reading ?? null;
  }

  /**
   * Reading time as text.
   *
   * Hours and minutes rather than a raw seconds count, because a bare 1800
   * tells a reader nothing and invites them to divide by 3600 in their head.
   */
  function durationText(seconds: number): string {
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.round((seconds % 3600) / 60);
    if (hours === 0) return `${minutes} min`;
    if (minutes === 0) return `${hours} h`;
    return `${hours} h ${minutes} min`;
  }

  /** Words with a magnitude, so a reader is not reading raw digits. */
  function wordsText(words: number): string {
    if (words >= 1_000_000) return `${(words / 1_000_000).toFixed(1)} million`;
    if (words >= 1000) return `${(words / 1000).toFixed(1)} thousand`;
    return String(words);
  }

  function subjectLabel(m: AnalyticsMeta): string {
    return m.subject === 'other' ? 'about other people' : 'about you';
  }
</script>

<section class="analytics" aria-labelledby="analytics-heading">
  <header>
    <h1 id="analytics-heading">Your analytics</h1>
    {#if !loading && !error}
      <p class="viewer-note">
        Showing what trust level {trustLevel}
        {#if role !== 'reader'}({role}){/if} on this {preset} instance can see. Higher trust
        unlocks more detail, never more about anyone else.
      </p>
    {/if}
  </header>

  {#if loading}
    <p class="state" role="status">Loading your analytics…</p>
  {:else if signedOut}
    <p class="state">
      These are your own reading numbers, so they need an account. <a href="/sign-in">Sign in</a>, or{' '}
      <a href="/register">register</a>, and your reading history will be waiting.
    </p>
  {:else if error}
    <p class="state error" role="alert">{error}</p>
  {:else if capabilities.length === 0}
    <p class="state">Nothing to show yet. This instance has not opened any analytics to you.</p>
  {:else}
    <ul class="capabilities">
      {#each capabilities as meta (meta.name)}
        <li class:open={open === meta.name}>
          <button
            type="button"
            class="capability"
            aria-expanded={open === meta.name}
            on:click={() => toggle(meta)}
          >
            <span class="name">{meta.name}</span>
            <span class="subject">{subjectLabel(meta)}</span>
          </button>

          <p class="definition">{meta.definition}</p>
          <p class="provenance">
            <span>{meta.freshness}</span>
            <span aria-hidden="true">·</span>
            <span>trust level {meta.minimum_trust_level}</span>
            {#if meta.floor !== null}
              <span aria-hidden="true">·</span>
              <span>floor {meta.floor}</span>
            {/if}
          </p>

          {#if open === meta.name}
            <div class="value">
              {#if detailLoading}
                <p class="state" role="status">Loading…</p>
              {:else if detail && !detail.implemented}
                <p class="state">
                  This one is not built yet. It is registered and gated, and has no query behind
                  it.
                </p>
              {:else if detail}
                {@const totals = readingTotals(detail)}
                {@const shown = countText(detail)}
                {#if totals}
                  <dl class="reading">
                    <div>
                      <dt>Works finished</dt>
                      <dd>{totals.finished_works.toLocaleString()}</dd>
                    </div>
                    <div>
                      <dt>Chapters read</dt>
                      <dd>{totals.chapters_read.toLocaleString()}</dd>
                    </div>
                    <div>
                      <dt>Words read</dt>
                      <dd>{wordsText(totals.words_read)}</dd>
                    </div>
                    <div>
                      <dt>Time reading</dt>
                      <dd>{durationText(totals.reading_seconds)}</dd>
                    </div>
                  </dl>
                {:else if shown}
                  <p class="count">{shown}</p>
                {/if}
                <p class="approximation">{detail.meta.approximation}</p>
              {/if}
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .analytics {
    max-width: 46rem;
    margin: 0 auto;
    padding: 1.5rem;
  }
  h1 {
    font-size: 1.4rem;
    margin-bottom: 0.25rem;
  }
  .viewer-note {
    color: var(--muted, #666);
    font-size: 0.9rem;
    margin-top: 0;
  }
  .state {
    color: var(--muted, #666);
    font-style: italic;
  }
  .state.error {
    color: var(--danger, #b3261e);
    font-style: normal;
  }
  .capabilities {
    list-style: none;
    padding: 0;
    margin: 1.5rem 0 0;
    display: grid;
    gap: 0.75rem;
  }
  .capabilities > li {
    border: 1px solid var(--border, #ddd);
    border-radius: 6px;
    padding: 0.75rem 1rem;
  }
  .capability {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 1rem;
    width: 100%;
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .name {
    font-family: var(--mono, monospace);
    font-size: 0.9rem;
  }
  .subject {
    font-size: 0.8rem;
    color: var(--muted, #666);
  }
  .definition {
    font-size: 0.9rem;
    margin: 0.5rem 0 0.25rem;
  }
  .provenance {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    font-size: 0.78rem;
    color: var(--muted, #666);
    margin: 0;
  }
  .value {
    margin-top: 0.75rem;
    padding-top: 0.75rem;
    border-top: 1px dashed var(--border, #ddd);
  }
  .count {
    font-size: 1.6rem;
    font-weight: 600;
    margin: 0 0 0.25rem;
  }
  .reading {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr));
    gap: 0.75rem 1.25rem;
    margin: 0 0 0.5rem;
  }
  .reading dt {
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted, #666);
  }
  .reading dd {
    margin: 0.15rem 0 0;
    font-size: 1.3rem;
    font-weight: 600;
  }
  .approximation {
    font-size: 0.8rem;
    color: var(--muted, #666);
    margin: 0;
  }
</style>
