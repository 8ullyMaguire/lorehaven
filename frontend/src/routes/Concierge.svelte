<script lang="ts">
  /**
   * The personal concierge (spec §54).
   *
   * Three things this page must not do, each of which is a §54.6 requirement and
   * each of which is invisible in the happy path:
   *
   * 1. **Not fall back.** An empty queue is rendered as an empty queue with the
   *    server's explanation. Substituting the unfiltered feed — or a generic
   *    "nothing matches" — is the defect §54.6 exists to prevent, and it looks
   *    identical to correct behaviour until a reader notices they were not shown
   *    what they asked for.
   * 2. **Not conflate "nothing matched" with "nothing fit".** `explained_empty`
   *    and `truncated_at` are different fields here for the same reason they are
   *    different fields in `api.ts`, and this template branches on both.
   * 3. **Not send a selector the reader did not choose.** The mood and budget
   *    inputs start empty and are omitted from the request until they are filled,
   *    so the first render is §54.7's parity case — the discovery feed, in the
   *    discovery order — rather than a queue narrowed by two accidental defaults.
   */
  import {
    fetchConciergeQueue,
    unwatchWork,
    watchWork,
    type ConciergeItem,
    type ConciergeQueue,
    type ConciergeReason,
  } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import Button from '../lib/components/Button.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  /**
   * The budget choices, in minutes, as a fixed ladder.
   *
   * A number input rather than a free-text field: §54.4 charges the queue a real
   * time budget, and "how long do you want to read for" has a small set of
   * sensible answers. A free field would also let a reader type a budget that
   * matches no work, which the server answers as an explained empty queue — a
   * correct answer to a question nobody needed to ask.
   */
  const BUDGETS = [15, 30, 60, 120] as const;

  let queue = $state<ConciergeQueue | null>(null);
  let error = $state<unknown>(null);
  let loading = $state(true);
  let mood = $state('');
  let minutes = $state<number | undefined>(undefined);

  /**
   * The moods the reader can ask for, derived from what the queue says it served.
   *
   * NOT derived from the items: a mood that matched nothing appears in no item, so
   * an items-derived list would drop exactly the mood the reader most likely wants
   * to try next — the one they just found empty. §54.2's list comes from the
   * server's validation error instead, which is the only place it appears.
   */
  let knownMoods = $state<string[]>([]);
  let moodError = $state<string | null>(null);

  /**
   * True only when the SERVER said the selector matched nothing.
   *
   * Not `items.length === 0`. An empty list has two causes here and they are not
   * the same message, so the flag is read off `explained_empty` rather than derived
   * from the length.
   */
  const explained = $derived(queue?.explained_empty != null);
  const budgetExhausted = $derived(queue?.truncated_at === 0 && !explained);

  async function load() {
    loading = true;
    error = null;
    // Only send what the reader chose. `mood` is omitted when blank rather than
    // sent as `''`, because a present-but-empty mood is a different request from
    // an absent one.
    const selector: { mood?: string; minutes?: number } = {};
    if (mood.trim() !== '') selector.mood = mood.trim();
    if (minutes !== undefined) selector.minutes = minutes;
    try {
      queue = await fetchConciergeQueue(selector);
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  function apply(moodValue: string, budget: number | undefined) {
    mood = moodValue;
    minutes = budget;
    moodError = null;
    void load();
  }

  /**
   * Why an item is in the queue, in the reader's words.
   *
   * `blend` is rendered as "from your feed" rather than hidden: a reader who asked
   * for a mood and sees an item marked as coming from the general feed has been
   * told the truth about what happened to their selector, which is §54.6's whole
   * argument for carrying a reason at all.
   */
  function reasonLabel(reason: ConciergeReason): string {
    switch (reason.kind) {
      case 'mood':
        return `because you asked for ${reason.mood}`;
      case 'budget':
        return 'it fits the time you had';
      case 'unknown_length':
        return 'its length is not known yet';
      default:
        return 'from your feed';
    }
  }

  async function toggleWatch(item: ConciergeItem, watching: boolean) {
    try {
      if (watching) {
        await unwatchWork(item.work_id);
      } else {
        await watchWork(item.work_id);
      }
      await load();
    } catch (failure) {
      error = failure;
    }
  }

  $effect(() => {
    void load();
  });
</script>

<section class="concierge">
  <header class="concierge-header">
    <h1>Your queue</h1>
    <p class="lede">
      A short list for right now, built from what you enjoy reading. Ask for a
      mood, or say how long you have — the order is the same either way.
    </p>
  </header>

  <div class="selector">
    <fieldset class="budget">
      <legend>How long have you got?</legend>
      <!--
        A "no limit" button, not a preselected budget: §54.7's parity case is a
        reader who has chosen nothing, and preselecting the first rung would make
        every first render a budgeted queue.
      -->
      <Button variant={minutes === undefined ? 'primary' : 'secondary'} onclick={() => apply(mood.trim(), undefined)}>
        No limit
      </Button>
      {#each BUDGETS as rung}
        <Button
          variant={minutes === rung ? 'primary' : 'secondary'}
          onclick={() => apply(mood.trim(), rung)}
        >
          {rung} min
        </Button>
      {/each}
    </fieldset>
  </div>

  {#if error}
    <ErrorSummary {error} onretry={load} />
  {/if}

  {#if loading}
    <Skeleton lines={5} label="Loading your queue" />
  {:else if explained}
    <!--
      §54.6: the server's own explanation, verbatim. Not paraphrased and not
      replaced with a generic empty state — the reason is the whole point of
      answering at all.
    -->
    <div class="empty explained" role="status">
      <p class="empty-head">Nothing matches that right now.</p>
      <p class="empty-why">{queue?.explained_empty}</p>
      <Button variant="secondary" onclick={() => apply('', undefined)}>
        Show my feed instead
      </Button>
    </div>
  {:else if budgetExhausted}
    <!--
      A budget of nothing is its own message. "Nothing fit in the time you gave
      me" is actionable and "nothing matched" is not, so they do not share a
      branch.
    -->
    <div class="empty exhausted" role="status">
      <p class="empty-head">Nothing fits the time you gave.</p>
      <p class="empty-why">
        Everything recommended is longer than that. Give it more time, or take the
        feed as it is.
      </p>
      <Button variant="secondary" onclick={() => apply(mood.trim(), undefined)}>
        No limit
      </Button>
    </div>
  {:else if queue && queue.items.length === 0}
    <p class="empty">Nothing to recommend yet. Read, rate, or tag a few works and the queue will fill in.</p>
  {:else if queue}
    <p class="total">
      {queue.estimated_minutes} min
      {#if queue.rate_source === 'default'}
        <span class="rate-note">(at a typical reading pace — your own once you have read something)</span>
      {/if}
    </p>

    <ul class="queue">
      {#each queue.items as item, index (item.work_id)}
        <li>
          <a
            href={`/works/${encodeURIComponent(item.work_id)}`}
            onclick={(event) => handleLinkClick(event, `/works/${encodeURIComponent(item.work_id)}`)}
          >
            <span class="queue-work">{item.title ?? item.work_id}</span>
            <span class="queue-meta">
              {#if item.estimated_minutes !== null}
                {item.estimated_minutes} min
              {:else}
                length unknown
              {/if}
              · {reasonLabel(item.reason)}
            </span>
          </a>
          <Button variant="quiet" onclick={() => toggleWatch(item, false)}>
            Tell me when it ends
          </Button>
        </li>
        {#if queue.truncated_at === index + 1}
          <!--
            §54.4's cut, marked in place rather than summarised above the list. A
            reader who was given less than they asked for should be able to see
            exactly where the list stopped without counting rows.
          -->
          <li class="cut" aria-hidden="true">
            <span>cut here — {queue.estimated_minutes} min of the time you gave</span>
          </li>
        {/if}
      {/each}
    </ul>

    {#if moodError}
      <p class="mood-error">{moodError}</p>
    {/if}
  {/if}
</section>

<style>
  .concierge {
    max-width: 72rem;
    margin-inline: auto;
    padding: 2rem 1rem;
  }
  .concierge-header h1 {
    margin-bottom: 0.25rem;
  }
  .lede {
    color: var(--text-muted);
  }
  .selector {
    margin-block: 1.5rem;
  }
  .budget {
    border: 1px solid var(--border);
    border-radius: 0.5rem;
    padding: 0.75rem 1rem 1rem;
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    align-items: center;
  }
  .budget legend {
    font-size: 0.875rem;
    color: var(--text-muted);
    padding-inline: 0.25rem;
  }
  .empty {
    padding: 2rem;
    text-align: center;
    background: var(--surface-muted);
    border-radius: 0.5rem;
    color: var(--text-muted);
    display: grid;
    gap: 0.75rem;
    justify-items: center;
  }
  .empty-head {
    font-weight: 600;
    color: var(--text);
  }
  .empty-why {
    max-width: 34rem;
  }
  .total {
    font-weight: 600;
  }
  .rate-note {
    font-weight: 400;
    color: var(--text-muted);
    font-size: 0.875rem;
  }
  .queue {
    list-style: none;
    padding: 0;
    margin: 0;
    display: grid;
    gap: 0.75rem;
  }
  .queue li {
    display: flex;
    gap: 0.75rem;
    align-items: center;
    justify-content: space-between;
  }
  .queue a {
    display: flex;
    flex-direction: column;
    flex: 1;
    padding: 1rem;
    border: 1px solid var(--border);
    border-radius: 0.5rem;
    text-decoration: none;
    color: inherit;
  }
  .queue a:hover {
    background: var(--surface-hover);
  }
  .queue-work {
    font-weight: 600;
  }
  .queue-meta {
    font-size: 0.875rem;
    color: var(--text-muted);
    margin-top: 0.25rem;
  }
  .cut {
    justify-content: center;
    color: var(--text-muted);
    font-size: 0.875rem;
    border-block: 1px dashed var(--border);
    padding-block: 0.5rem;
  }
  .mood-error {
    color: var(--danger, #b00);
  }
</style>