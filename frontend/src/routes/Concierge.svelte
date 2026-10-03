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
    ApiError,
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
   * Whether `knownMoods` was learned from a refusal rather than being real state.
   *
   * §54.2's list is not a field on the queue, so it only exists after a 422. That
   * makes it derived-from-an-error rather than loaded, and the flag is what lets a
   * later SUCCESS drop it: without this the instance's moods would outlive the
   * question that produced them and sit under the selector indefinitely.
   */
  let moodErrorLearned = $state(false);

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
      // A successful queue clears the refusal and, if it arrived from one, the
      // learned mood list with it: the list was learned from a 422 about a mood
      // the reader no longer has selected.
      moodError = null;
      if (moodErrorLearned) knownMoods = [];
    } catch (failure) {
      error = failure;
      // A refused MOOD is not a failure of the page. The server answered 422
      // naming the moods that DO exist, and that list is the only place
      // §54.2's vocabulary appears -- so it is read out of the refusal and
      // offered back, rather than shown as a red banner the reader can do
      // nothing with.
      //
      // Everything else stays an error. A 500 on the same request is a real
      // failure and gets ErrorSummary.
      const refusal = moodRefusal(failure);
      if (refusal) {
        moodError = refusal.message;
        if (refusal.available.length > 0) {
          knownMoods = refusal.available;
          moodErrorLearned = true;
        }
        // Swallowed deliberately: the reader gets the vocabulary inline, which
        // is more use than the generic summary for the same event.
        error = null;
      }
    } finally {
      loading = false;
    }
  }

  /**
   * The mood refusal out of an `ApiError`, or null when this is some other error.
   *
   * Read off `fieldErrors.mood` because that is where the route puts it: the
   * server refuses with `AppError::field("mood", ...)` so the message is scoped to
   * the field rather than being a page-level complaint. A 422 on some OTHER field,
   * or any other status, returns null and is handled as an error.
   */
  function moodRefusal(failure: unknown): { message: string; available: string[] } | null {
    if (!(failure instanceof ApiError)) return null;
    // 422 EXACTLY, not "any 4xx" and not "any error carrying fieldErrors.mood".
    //
    // Widening the gate was tried and it is wrong: a 500 that happens to carry a
    // `mood` field error is a real failure, and treating it as a refused mood
    // would swallow it into an inline sentence with no retry -- the mutation
    // (>= 400) left all 16 tests green, because the 500 these tests use carries no
    // field errors at all and so cannot tell the two rules apart.
    if (failure.status !== 422) return null;
    const message = failure.fieldErrors.mood;
    if (message === undefined) return null;
    // The server formats the list into the sentence ("the moods on this instance
    // are: comfort, longing"), so the words are recovered from the message
    // rather than from a structured field. The response has no such field, and
    // inventing one here would mean the client parsing English -- so this parse
    // is deliberately narrow and falls back to no list when the shape differs.
    const marker = 'the moods on this instance are:';
    const at = message.indexOf(marker);
    const available =
      at === -1
        ? []
        : message
            .slice(at + marker.length)
            .split(',')
            .map((m) => m.trim())
            .filter((m) => m.length > 0);
    return { message, available };
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
    <fieldset class="mood">
      <legend>What kind of sitting?</legend>
      <!--
        A free-text field, not a <select>: §15.8's moods are author-assigned, so
        the vocabulary is whatever this instance's writers have used and a closed
        list cannot know it. The server validates against the moods a PUBLISHED
        work actually carries and refuses anything else by name.
      -->
      <label class="mood-label" for="concierge-mood">A mood, if you feel like naming one</label>
      <input
        id="concierge-mood"
        class="mood-input"
        type="text"
        autocomplete="off"
        placeholder="comfort, longing, ..."
        bind:value={mood}
        onkeydown={(event) => {
          if (event.key === 'Enter') {
            event.preventDefault();
            apply(mood.trim(), minutes);
          }
        }}
      />
      {#if moodError}
        <!--
          The server's own sentence, not a paraphrase. It names what the reader
          typed AND what exists, and the second half is the only statement of
          §54.2's vocabulary this surface can make.
        -->
        <p class="mood-error" role="status">{moodError}</p>
      {/if}
      {#if knownMoods.length > 0}
        <!--
          Offered as buttons because the list arrived from a refusal: the reader
          has already been told they cannot ask for what they asked for, so the
          useful next move is one click rather than retyping.
        -->
        <ul class="mood-suggestions">
          {#each knownMoods as suggestion (suggestion)}
            <li>
              <Button variant="secondary" onclick={() => apply(suggestion, minutes)}>
                {suggestion}
              </Button>
            </li>
          {/each}
        </ul>
      {/if}
    </fieldset>

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

  {/if}
</section>

<style>
  .concierge {
    max-width: 72rem;
    margin-inline: auto;
    padding: 2rem 1rem;
  }
  /*
    The mood fieldset sits beside the budget one, so the two are laid out here
    rather than in two rules. The file's other selector rules use raw values and
    two custom properties that are NOT DEFINED anywhere in tokens.css
    (`--text-muted`, `--border`), which resolve to nothing at computed-value time --
    so the real tokens are used here instead, and those dead references are noted
    rather than copied.
  */
  .selector {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem 1.5rem;
    align-items: flex-start;
  }
  .mood {
    flex: 1 1 22rem;
    border: var(--border-width) solid var(--color-border);
    border-radius: 0.5rem;
    padding: 0.75rem 1rem 1rem;
  }
  .mood legend {
    font-size: 0.875rem;
  }
  .mood-label {
    display: block;
    font-size: 0.875rem;
    color: var(--color-muted);
    margin-bottom: 0.5rem;
  }
  .mood-input {
    width: 100%;
    box-sizing: border-box;
    padding: 0.4rem 0.6rem;
    border: var(--border-width) solid var(--color-border-strong);
    border-radius: 0.25rem;
    background: var(--color-surface);
    color: var(--color-text);
    font: inherit;
  }
  .mood-error {
    margin: 0.6rem 0 0;
    color: var(--color-muted);
    font-size: 0.875rem;
  }
  .mood-suggestions {
    list-style: none;
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    margin: 0.6rem 0 0;
    padding: 0;
  }

  .concierge-header h1 {
    margin-bottom: 0.25rem;
  }
  .lede {
    color: var(--color-muted);
  }
  .budget {
    margin-bottom: 1.5rem;
    border: 1px solid var(--color-border);
    border-radius: 0.5rem;
    padding: 0.75rem 1rem 1rem;
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    align-items: center;
  }
  .budget legend {
    font-size: 0.875rem;
    color: var(--color-muted);
    padding-inline: 0.25rem;
  }
  .empty {
    padding: 2rem;
    text-align: center;
    background: var(--color-surface-raised);
    border-radius: 0.5rem;
    color: var(--color-muted);
    display: grid;
    gap: 0.75rem;
    justify-items: center;
  }
  .empty-head {
    font-weight: 600;
    color: var(--color-text);
  }
  .empty-why {
    max-width: 34rem;
  }
  .total {
    font-weight: 600;
  }
  .rate-note {
    font-weight: 400;
    color: var(--color-muted);
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
    border: 1px solid var(--color-border);
    border-radius: 0.5rem;
    text-decoration: none;
    color: inherit;
  }
  .queue a:hover {
    background: var(--color-surface-raised);
  }
  .queue-work {
    font-weight: 600;
  }
  .queue-meta {
    font-size: 0.875rem;
    color: var(--color-muted);
    margin-top: 0.25rem;
  }
  .cut {
    justify-content: center;
    color: var(--color-muted);
    font-size: 0.875rem;
    border-block: 1px dashed var(--color-border);
    padding-block: 0.5rem;
  }
  .mood-error {
    color: var(--color-danger);
  }
</style>