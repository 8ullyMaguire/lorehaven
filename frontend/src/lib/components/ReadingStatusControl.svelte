<script lang="ts">
  /**
   * Reading status for a work (spec §9.6).
   *
   * The reader's own record of how far they got, and the input to the
   * `own.reading.basic` capability on the analytics page. Until this existed a
   * reader had no way to record it for a work published on this instance: the
   * status door keyed on a library item, and library items are created only by
   * the import runner.
   *
   * Four states, because they are the four things a reader can honestly say
   * about a work and no more. Deliberately *not* a star rating: a rating is a
   * judgement of the work, this is a note to self.
   *
   * The states are a select rather than a row of buttons because they are
   * mutually exclusive and the current one matters: a reader who has finished
   * the work needs to see that, not infer it from four unselected buttons.
   */
  import {
    clearWorkReadingStatus,
    fetchWorkReadingStatus,
    setWorkReadingStatus,
    type ReadingStatus,
    type WorkReadingStatus,
  } from '../api';
  import { session } from '../session.svelte.ts';

  interface Props {
    workId: string;
  }

  let { workId }: Props = $props();

  // All five, in the order a reader moves through them, and in the server's own
  // vocabulary: `ReadingStatus::parse` is the authority, and it is kebab-case.
  // An earlier version of this list had four states and `on_hold`, which would
  // have sent a value the server does not parse and dropped `want-to-read`
  // entirely -- so a reader could never say they had not started.
  const CHOICES: { value: ReadingStatus; label: string; hint: string }[] = [
    { value: 'want-to-read', label: 'Want to read', hint: 'On the list, not started' },
    { value: 'reading', label: 'Reading', hint: 'Started, not finished' },
    { value: 'on-hold', label: 'On hold', hint: 'Intending to come back' },
    { value: 'dropped', label: 'Dropped', hint: 'Not finishing this one' },
    { value: 'finished', label: 'Finished', hint: 'Read to the end' },
  ];

  let current = $state<ReadingStatus | null>(null);
  let loaded = $state(false);
  let busy = $state(false);
  let error = $state<unknown>(null);

  // Loaded when the session resolves rather than passed in: the work page is
  // the only place this appears, and a prop would mean the page had to fetch it
  // for a visitor who cannot see it anyway.
  //
  // The `session.isSignedIn` read inside the effect is load-bearing. It starts
  // as `unknown` and resolves asynchronously, so an effect that ran only on
  // mount would see a not-yet-signed-in session, give up, and never fetch --
  // leaving a signed-in reader with a permanently unselected control that looks
  // like they had recorded nothing. Reading the flag here is what makes the
  // effect re-run when it changes.
  $effect(() => {
    let cancelled = false;
    const signedIn = session.isSignedIn;
    if (!signedIn) {
      // Not "loaded": the session may still be resolving, and treating
      // `unknown` as "no session" is the same bug one level up.
      loaded = session.status === 'anonymous';
      return;
    }
    void (async () => {
      try {
        const record: WorkReadingStatus | null = await fetchWorkReadingStatus(workId);
        if (!cancelled) current = record?.status ?? null;
      } catch (failure) {
        // A failed *read* is not worth an error banner: the reader has not
        // asked anything yet, and offering the four states as though nothing
        // was recorded would be a guess. Say so quietly instead.
        if (!cancelled) error = failure;
      } finally {
        if (!cancelled) loaded = true;
      }
    })();
    return () => {
      cancelled = true;
    };
  });

  async function choose(status: ReadingStatus) {
    if (busy) return;
    busy = true;
    error = null;
    // Optimistic, then reconciled: the select must not snap back while the
    // request is in flight, or a reader choosing "Finished" sees it revert to
    // "no status" and has to wonder whether the click landed.
    const previous = current;
    current = status;
    try {
      const record = await setWorkReadingStatus(workId, status);
      current = record.status;
    } catch (failure) {
      current = previous;
      error = failure;
    } finally {
      busy = false;
    }
  }

  async function clear() {
    if (busy) return;
    busy = true;
    error = null;
    const previous = current;
    current = null;
    try {
      await clearWorkReadingStatus(workId);
    } catch (failure) {
      current = previous;
      error = failure;
    } finally {
      busy = false;
    }
  }

  function messageFor(failure: unknown): string {
    if (failure instanceof Error && failure.message) return failure.message;
    return 'Could not record that. Please try again.';
  }
</script>

<div class="reading-status">
  <h3 class="label" id="reading-status-label">Where you got to</h3>

  <!--
    The `session.status === 'anonymous'` test rather than `!session.isSignedIn`:
    the session starts `unknown`, and branching on the negation shows the
    sign-in prompt to a reader who is signed in but whose session has not
    resolved yet -- asking somebody to sign in again while they are signed in.
    `unknown` gets the loading state, which is the honest one.
  -->
  {#if session.status === 'anonymous'}
    <p class="hint">
      <a href="/sign-in">Sign in</a> to keep track of what you have read.
    </p>
  {:else if !loaded}
    <p class="hint" role="status">Checking what you have recorded…</p>
  {:else}
    <div class="choices" role="group" aria-labelledby="reading-status-label">
      {#each CHOICES as choice (choice.value)}
        <button
          type="button"
          class="choice"
          class:chosen={current === choice.value}
          title={choice.hint}
          aria-pressed={current === choice.value}
          disabled={busy}
          onclick={() => choose(choice.value)}
        >
          {choice.label}
        </button>
      {/each}
    </div>

    {#if current !== null}
      <button type="button" class="forget" disabled={busy} onclick={clear}>
        Forget this
      </button>
    {/if}
  {/if}

  {#if error}
    <!-- A failed write that springs back with no explanation reads as "my
         click didn't register", which is the same problem KudosButton had. -->
    <p class="error" role="alert">{messageFor(error)}</p>
  {/if}
</div>

<style>
  .reading-status {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
  }

  .label {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--color-muted);
    margin: 0;
  }

  .choices {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .choice {
    padding: var(--space-2) var(--space-3);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-sm);
    background: var(--color-surface);
    color: var(--color-text);
    cursor: pointer;
    font-size: var(--text-sm);
  }

  .choice:hover:not(:disabled) {
    border-color: var(--color-accent);
  }

  /* The chosen state has to be more than a colour: this is the current value of
     a control, and a reader who cannot distinguish the accent is left guessing
     what they already recorded. */
  .choice.chosen {
    border-color: var(--color-accent);
    color: var(--color-accent);
    font-weight: 600;
  }

  .choice.chosen::before {
    content: '✓ ';
  }

  .choice:disabled,
  .forget:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .forget {
    background: none;
    border: none;
    padding: 0;
    color: var(--color-muted);
    font-size: var(--text-xs);
    text-decoration: underline;
    cursor: pointer;
  }

  .hint {
    font-size: var(--text-sm);
    color: var(--color-muted);
    margin: 0;
  }

  .error {
    color: var(--color-danger, #b91c1c);
    font-size: var(--text-xs);
    margin: 0;
  }
</style>
