<script lang="ts">
  /**
   * Sort control for a browse surface (spec §43.2, §43.4).
   *
   * The server already had all of this: `Sort` in the domain, `?sort=` on the
   * routes, and a sticky per-pseud preference at `/browse/sort/{surface}`.
   * What was missing was the reader-facing half -- nothing in the UI ever sent
   * `?sort=`, so the vocabulary existed only in an API surface no person could
   * reach. A requirement that says "for-you default on every browse surface"
   * is not met by a default no surface can change.
   *
   * A select, not a row of buttons: seven mutually exclusive values where only
   * the current one matters, and a select says "this is a setting" where seven
   * buttons would say "these are seven actions".
   *
   * Choosing an option does two things, and both are wanted. It moves the list
   * immediately, because a reader who picked "Trending" is waiting for
   * trending. And it stores the choice, because §43.4 asks for stickiness. The
   * store is a *follow-through*, so a failure to store is reported but does not
   * undo the list the reader is already looking at.
   */
  import {
    clearSort,
    fetchSort,
    setSort,
    SORT_LABELS,
    SORT_VALUES,
    type SortValue,
  } from '../api';
  import { session } from '../session.svelte.ts';

  interface Props {
    /** The surface key the server uses, e.g. `discover`, `people`. */
    surface: string;
    /** Called when the reader picks a value, so the page can refetch. */
    onchange?: (sort: SortValue) => void;
  }

  let { surface, onchange }: Props = $props();

  let current = $state<SortValue>('new');
  /** Whether the value came from the reader's stored preference. */
  let stored = $state(false);
  let loaded = $state(false);
  let busy = $state(false);
  /** Storage failed, the list still moved. Said plainly rather than swallowed. */
  let storeFailed = $state(false);

  // Defaults per surface, mirroring `default_for` in the server's browse route.
  // The fetch is authoritative; this only decides what the control shows before
  // the answer lands, so it must not disagree with `default_for` for long.
  const DEFAULTS: Record<string, SortValue> = {
    discover: 'for-you',
    people: 'az',
    tags: 'az',
    fandoms: 'az',
    authors: 'az',
    moods: 'az',
  };

  $effect(() => {
    let cancelled = false;
    // Re-runs when the session resolves: a signed-out reader gets the surface
    // default, and a signed-in one may have a stored preference. Treating the
    // not-yet-known session as "anonymous" would show the default and then never
    // correct it, which is the same class of bug the reading-status control had.
    void session.isSignedIn;
    void (async () => {
      try {
        const state = await fetchSort(surface);
        if (cancelled) return;
        if (SORT_VALUES.includes(state.sort as SortValue)) {
          current = state.sort as SortValue;
        }
        stored = state.source === 'preference';
      } catch {
        // A control that cannot read its own state is still usable: the surface
        // default stands and the reader can still choose. No error banner --
        // this is not a failure they can act on.
        if (!cancelled) {
          current = DEFAULTS[surface] ?? 'new';
        }
      } finally {
        if (!cancelled) loaded = true;
      }
    })();
    return () => {
      cancelled = true;
    };
  });

  async function choose(value: SortValue) {
    if (busy || value === current) return;
    busy = true;
    storeFailed = false;
    // Move the list first. The reader asked for a different order and the
    // store is bookkeeping about the next visit.
    current = value;
    onchange?.(value);
    try {
      if (!session.isSignedIn) return;
      const state = await setSort(surface, value);
      stored = state.source === 'preference';
    } catch {
      storeFailed = true;
    } finally {
      busy = false;
    }
  }

  async function reset() {
    if (busy) return;
    busy = true;
    storeFailed = false;
    const fallback = DEFAULTS[surface] ?? 'new';
    current = fallback;
    onchange?.(fallback);
    try {
      if (session.isSignedIn) await clearSort(surface);
      stored = false;
    } catch {
      storeFailed = true;
    } finally {
      busy = false;
    }
  }
</script>

<div class="sort-control" data-testid="sort-control">
  <label class="label" for="sort-{surface}">Sort</label>
  <select
    id="sort-{surface}"
    disabled={!loaded || busy}
    value={current}
    onchange={(event) => choose(event.currentTarget.value as SortValue)}
  >
    {#each SORT_VALUES as value (value)}
      <option {value}>{SORT_LABELS[value]}</option>
    {/each}
  </select>

  {#if stored}
    <button type="button" class="reset" disabled={busy} onclick={reset}>
      Back to default
    </button>
  {/if}

  {#if storeFailed}
    <p class="hint warn" role="status">
      This order will not be remembered next time.
    </p>
  {/if}
</div>

<style>
  .sort-control {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .label {
    font-size: 0.8125rem;
    color: var(--text-muted, currentColor);
  }

  select {
    font: inherit;
    padding: 0.375rem 0.5rem;
    border: 1px solid var(--border, currentColor);
    border-radius: 0.25rem;
    background: transparent;
    color: inherit;
  }

  .reset {
    font: inherit;
    font-size: 0.8125rem;
    background: none;
    border: none;
    text-decoration: underline;
    color: inherit;
    cursor: pointer;
    padding: 0;
  }

  .reset:disabled,
  select:disabled {
    cursor: default;
    opacity: 0.6;
  }

  .hint {
    margin: 0;
    font-size: 0.8125rem;
  }

  .warn {
    color: var(--danger, currentColor);
  }
</style>
