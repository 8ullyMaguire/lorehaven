<script module lang="ts">
  /**
   * Exported from the module context so callers and tests can type their fixtures
   * against the real shape. An `export interface` in the instance `<script>` is not
   * importable in Svelte 5.
   */
  import type { DnfReason, DnfRecord, DnfReasonCount } from '../api';
  export type { DnfReason, DnfRecord, DnfReasonCount };
  /**
   * The props, exported so `DnfPanel.test.ts` can type its fixture helper.
   *
   * `ComponentProps<DnfPanel>` is NOT a substitute: it resolves to `undefined` for a
   * component whose props are declared in the instance script, so a test helper typed with
   * it accepts nothing and every override becomes
   * `not assignable to parameter of type 'undefined'`. An exported `Props` is explicit and
   * does not depend on how the component is declared.
   */
  export type DnfPanelProps = {
    workId: string;
    signedIn?: boolean;
    loadMine?: typeof fetchMyDnf;
    loadReasons?: typeof fetchDnfReasons;
    save?: typeof setMyDnf;
    clear?: typeof clearMyDnf;
  };
</script>

<script lang="ts">
  /**
   * "Mark as DNF" — item 11 of the 100-idea audit.
   *
   * ## What was already here, and what this adds
   *
   * The entire server side shipped in M45-21: `did_not_finish` (0073) with six structured
   * reasons, a `note`, `is_public`, `works.allow_dnf_feedback` as the author's consent
   * switch, seven store functions, three routes and eight acceptance tests. `grep -rl
   * dnf frontend/src` returned **nothing**. So this component is the missing half of a
   * finished feature, and it is deliberately not a new data model.
   *
   * (An earlier plan proposed a `reader_work_status` table for this. It would have been a
   * second, weaker implementation of the thing 0073 already does — see
   * `docs/plans/100-ideas-remaining.md` §1.)
   *
   * ## Why the reason is a menu and not free text
   *
   * Six structured reasons can be counted; a free-text note cannot. The reader gets both,
   * and only the reason is ever eligible for an aggregate — and only when the author has
   * turned `allow_dnf_feedback` on. That split is the feature: it is what lets a reader
   * record something private AND let it help an author, without the author ever seeing a
   * reader's words.
   *
   * ## Why "private" is the default and the checkbox is opt-in
   *
   * A DNF mark is a reader's disposition toward a work. Publishing it by default would
   * be the same mistake as an unscoped "most bookmarked" leaderboard (item 27): an
   * aggregate that readers did not choose to contribute. So `isPublic` starts false and
   * the reader has to move it.
   *
   * ## The signed-out case
   *
   * Renders nothing. A DNF mark belongs to a pseud, so a signed-out reader has no row to
   * write and no business being offered one.
   */
  import {
    DNF_REASONS,
    fetchDnfReasons,
    fetchMyDnf,
    clearMyDnf,
    setMyDnf,
  } from '../api';

  interface Props {
    workId: string;
    /** False signed-out: the whole panel is omitted rather than shown disabled. */
    signedIn?: boolean;
    /** Injected in tests; defaults to the real client. */
    loadMine?: typeof fetchMyDnf;
    loadReasons?: typeof fetchDnfReasons;
    save?: typeof setMyDnf;
    clear?: typeof clearMyDnf;
  }

  let {
    workId,
    signedIn = false,
    loadMine = fetchMyDnf,
    loadReasons = fetchDnfReasons,
    save = setMyDnf,
    clear = clearMyDnf,
  }: Props = $props();

  type Stage = 'idle' | 'open' | 'saving' | 'saved' | 'clearing' | 'cleared' | 'error';

  let stage = $state<Stage>('idle');
  let mine = $state<DnfRecord | null>(null);
  let aggregate = $state<DnfReasonCount[]>([]);
  let reason = $state<DnfReason>('not_my_taste');
  let note = $state('');
  let isPublic = $state(false);
  let message = $state('');
  let busy = $state(false);
  /**
   * The failure from the initial load, kept separate from `message` because a load
   * failure is not a save receipt: reusing one field let a stale "Saved." survive a
   * subsequent reload and claim a mark that was never written.
   */
  let loadError = $state('');

  const reasonLabel = (value: DnfReason): string =>
    DNF_REASONS.find((r) => r.value === value)?.label ?? value;

  const totalPublic = $derived(aggregate.reduce((sum, row) => sum + row.count, 0));

  const shownMessage = $derived(loadError || message);

  async function load(): Promise<void> {
    if (!signedIn) return;
    loadError = '';
    // The two requests are independent, and neither may swallow the other. They are also
    // caught SEPARATELY on purpose: the aggregate is the author's disclosure while the
    // reader's own row is the reader's business, so a failure to load the first must not
    // cost the second. `Promise.all` would reject both on one failure.
    //
    // The catch here is load-bearing, not defensive: without it one network blip raises
    // an unhandled rejection, which fails a whole vitest run rather than this panel.
    const [own, counts] = await Promise.allSettled([loadMine(workId), loadReasons(workId)]);

    if (own.status === 'fulfilled') {
      mine = own.value;
      if (own.value) {
        reason = own.value.reason;
        note = own.value.note ?? '';
        isPublic = own.value.is_public;
      }
    } else {
      loadError =
        own.reason instanceof Error ? own.reason.message : 'Could not load your existing mark.';
    }

    if (counts.status === 'fulfilled') {
      aggregate = counts.value;
    } else if (!loadError) {
      loadError =
        counts.reason instanceof Error
          ? counts.reason.message
          : 'Could not load the reader feedback for this work.';
    }
  }

  /**
   * Re-read the public counts after a write, WITHOUT touching `loadError`.
   *
   * A failed refresh here must not blank the panel or replace a save receipt with an error
   * about a read the reader did not ask for: the mark was written, so saying "could not
   * load" would make a successful save look lost. The stale count is the lesser wrongness,
   * and a page reload fixes it.
   */
  async function refreshAggregate(): Promise<void> {
    try {
      aggregate = await loadReasons(workId);
    } catch {
      // Deliberately silent, and deliberately not loadError — see the doc comment.
    }
  }

  $effect(() => {
    void workId;
    void signedIn;
    stage = 'idle';
    mine = null;
    aggregate = [];
    note = '';
    isPublic = false;
    message = '';
    loadError = '';
    void load();
  });

  async function openForm(): Promise<void> {
    stage = 'open';
    message = '';
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy) return;
    busy = true;
    stage = 'saving';
    try {
      mine = await save(workId, reason, { note: note.trim() || null, isPublic });
      stage = 'saved';
      // Never "Saved." unconditionally: a private mark and a shared one behave
      // differently, and telling a reader their reason went to the author when it did not
      // (or the reverse) is the one message here that cannot be taken back.
      message = isPublic
        ? 'Saved. The author will see which reason, never your note.'
        : 'Saved. This stays off the work page unless you make it public.';
      // Refresh the aggregate. This was a REAL defect caught by the journey that asserts
      // "1 reader stopped here": the panel said "shared with the author" and showed no
      // aggregate, because the counts had been fetched once at mount — before the write
      // that changed them. The reader's own mark and the author's disclosure of it come
      // from different queries and only one of them was being re-read.
      await refreshAggregate();
    } catch (err) {
      stage = 'error';
      message = err instanceof Error ? err.message : 'Could not save that.';
    } finally {
      busy = false;
    }
  }

  async function undo(): Promise<void> {
    if (busy) return;
    busy = true;
    stage = 'clearing';
    try {
      await clear(workId);
      mine = null;
      note = '';
      isPublic = false;
      stage = 'cleared';
      message = 'Cleared. You can mark it again at any time.';
      // Same reason as after a save: the counts are now stale by one row, and a summary
      // that still counts a mark this reader just removed is the kind of small wrongness
      // that makes an honest page untrustworthy.
      await refreshAggregate();
    } catch (err) {
      stage = 'error';
      message = err instanceof Error ? err.message : 'Could not clear that.';
    } finally {
      busy = false;
    }
  }
</script>

{#if signedIn}
  <section class="dnf" aria-labelledby="dnf-heading">
    <h2 id="dnf-heading">Did not finish</h2>

    <!--
      The reader's own mark. A work you have marked is a work you decided not to finish,
      and the panel says so in the reader's terms rather than in the schema's: the stored
      value is `not_my_taste`, the reader reads "Not my taste".
    -->
    {#if mine}
      <p class="marked" data-testid="dnf-current">
        You marked this <strong>{reasonLabel(mine.reason)}</strong>
        {#if mine.is_public}
          <span class="badge">shared with the author</span>
        {:else}
          <span class="badge private">private</span>
        {/if}
      </p>
      <button type="button" onclick={undo} disabled={busy}>
        {stage === 'clearing' ? 'Clearing…' : 'Clear this mark'}
      </button>
    {:else if stage === 'open' || stage === 'saving' || stage === 'saved' || stage === 'error'}
      <form onsubmit={submit}>
        <fieldset>
          <legend>Why did you stop?</legend>
          {#each DNF_REASONS as option (option.value)}
            <label class="reason">
              <input
                type="radio"
                name="dnf-reason"
                value={option.value}
                bind:group={reason}
                disabled={busy}
              />
              <span class="reason-label">{option.label}</span>
              <span class="reason-hint">{option.hint}</span>
            </label>
          {/each}
        </fieldset>

        <label class="note-field">
          Note to yourself
          <!--
            Private ALWAYS, and not merely private by default: there is no control that
            can make this public. The structured reason is what may be shared; a reader's
            own words are theirs.
          -->
          <textarea bind:value={note} rows="2" maxlength="500" disabled={busy}></textarea>
          <span class="hint">Only you see this. It is never sent to the author.</span>
        </label>

        <label class="public-field">
          <input type="checkbox" bind:checked={isPublic} disabled={busy} />
          <span>
            Share the reason with the author
            <span class="hint">
              They will see which of the six reasons, never your note. Authors who have not
              turned on feedback see nothing at all.
            </span>
          </span>
        </label>

        <div class="actions">
          <button type="submit" disabled={busy}>
            {stage === 'saving' ? 'Saving…' : 'Save'}
          </button>
          <button type="button" onclick={() => (stage = 'idle')} disabled={busy}>Cancel</button>
        </div>
      </form>
    {:else}
      <p class="offer">
        Stopped reading it? Marking it keeps it out of your recommendations and lets the
        author know — if they ask for feedback.
      </p>
      <button type="button" onclick={openForm} data-testid="dnf-open">Mark as DNF</button>
    {/if}

    {#if shownMessage}
      <p
        class="note"
        role="status"
        data-testid="dnf-message"
        class:error={Boolean(loadError)}
      >
        {shownMessage}
      </p>
    {/if}

    <!--
      The author's aggregate. Only PUBLIC marks, and only when the author has enabled
      feedback — `aggregate_dnf_counts` already enforces both, and an empty list here is
      the correct answer for a fresh work.
    -->
    {#if totalPublic > 0}
      <details class="aggregate">
        <!--
        The count is written out rather than pluralised by a helper: `plural` does not
        exist in this codebase, and "1 reader stopped here" next to "2 readers stopped
        here" is the whole honesty requirement of this line. A header that always reads
        "readers" is wrong in the one case a reader is most likely to check.
      -->
      <summary>
        {totalPublic}
        {totalPublic === 1 ? 'reader stopped' : 'readers stopped'} here
      </summary>
        <ul>
          {#each aggregate as row (row.reason)}
            <li>
              <span>{reasonLabel(row.reason)}</span>
              <span class="count">{row.count}</span>
            </li>
          {/each}
        </ul>
      </details>
    {/if}
  </section>
{/if}

<style>
  .dnf {
    margin-block: var(--space-6);
    padding: var(--space-4);
    border: 1px solid var(--color-border, currentColor);
    border-radius: var(--radius-md, 6px);
  }

  h2 {
    font-size: var(--text-lg);
    margin-block-end: var(--space-2);
  }

  .offer,
  .marked {
    margin-block-end: var(--space-3);
  }

  fieldset {
    border: none;
    padding: 0;
    margin-block-end: var(--space-3);
  }

  legend {
    font-weight: 600;
    padding-block-end: var(--space-1);
  }

  .reason {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: var(--space-1) var(--space-2);
    align-items: baseline;
    padding-block: var(--space-1);
  }

  .reason-hint,
  .hint {
    grid-column: 2;
    font-size: var(--text-sm);
    color: var(--color-muted);
  }

  .note-field,
  .public-field {
    display: block;
    margin-block-end: var(--space-3);
  }

  textarea {
    display: block;
    inline-size: 100%;
    font: inherit;
  }

  .badge {
    font-size: var(--text-sm);
    margin-inline-start: var(--space-2);
  }

  .badge.private {
    color: var(--color-muted);
  }

  .actions {
    display: flex;
    gap: var(--space-2);
  }

  .aggregate {
    margin-block-start: var(--space-3);
  }

  .aggregate li {
    display: flex;
    justify-content: space-between;
  }
</style>