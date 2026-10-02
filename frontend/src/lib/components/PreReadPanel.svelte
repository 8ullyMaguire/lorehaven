<script lang="ts">
  /**
   * The author-only pre-read panel (gap C, §32.6).
   *
   * The routes are in `crates/app/src/routes/preread.rs` and the store in
   * `crates/db/src/preread_store.rs`.
   *
   * **This is an editor component, not a route.** §32.6 says a pre-read report is shown to
   * the author and never on the public work page, so there is deliberately no `/preread`
   * path to link to and no route id in `App.svelte`. It is rendered by `WorkEditor.svelte`
   * and it is the editor that decides the caller owns the work — the panel's own 404
   * handling is the second line of defence, not the first.
   *
   * **The component has no place to put a composite score.** There is no computed total, no
   * average, and no `score` on the container — the type in `api.ts` has none either. §32.6
   * forbids displaying composite quality scores publicly and §0.3 forbids credit, payment
   * or trust level moving any ranking signal, so a single number here would be a ranking
   * signal one template expression away from existing. The per-dimension rows are the whole
   * report, and they are shown worst-first because that is the question an author asks.
   *
   * **A 404 means "not yours", and is not shown to the user.** The server answers a
   * non-owner and a non-existent work identically, because a pre-read report is an
   * assessment of a draft and any difference would confirm the draft exists. So the panel
   * renders nothing at all on a 404 rather than an error message — a message saying "not
   * found" on somebody else's work is itself the confirmation.
   */
  import { fetchPreread, forgetPrereadProvider, type PreReadResponse } from '../api';
  import ErrorSummary from './ErrorSummary.svelte';
  import Skeleton from './Skeleton.svelte';

  let { workId }: { workId: string } = $props();

  let response = $state<PreReadResponse | null>(null);
  let error = $state<unknown>(null);
  let loading = $state(true);
  /** True when the caller does not own the work — render nothing, say nothing. */
  let notMine = $state(false);
  let withdrawing = $state<string | null>(null);

  async function load() {
    loading = true;
    error = null;
    notMine = false;
    try {
      response = await fetchPreread(workId);
    } catch (caught) {
      // A 404 is the server's indistinguishable answer for "not yours", so the panel
      // renders nothing rather than an error. Anything else is a real failure and is
      // reported as one — with `response` cleared first, so a failed reload does not leave
      // a stale report on screen next to the error, which would read as current.
      response = null;
      if (isNotFound(caught)) {
        notMine = true;
      } else {
        error = caught;
      }
    } finally {
      loading = false;
    }
  }

  /**
   * Withdraw one provider's output, then reload.
   *
   * §23.7 lets an author opt out of *specific* providers, so the control is per provider
   * and never a single "delete everything" button — that would be a withdrawal from
   * providers the author never asked to withdraw from.
   */
  async function withdraw(provider: string) {
    withdrawing = provider;
    error = null;
    try {
      await forgetPrereadProvider(workId, provider);
      await load();
    } catch (caught) {
      // The report is left alone on a failed withdrawal: showing a panel without the
      // provider that is still stored would understate what the instance holds.
      error = caught;
    } finally {
      withdrawing = null;
    }
  }

  function isNotFound(caught: unknown): boolean {
    if (typeof caught !== 'object' || caught === null) return false;
    const status = (caught as { status?: number }).status;
    const code = (caught as { code?: string }).code;
    return status === 404 || code === 'NOT_FOUND';
  }

  $effect(() => {
    void workId;
    void load();
  });
</script>

{#if notMine}
  <!-- Deliberately empty. See the header: a 404 here is the server's indistinguishable
       answer for "not yours", and anything rendered would be the confirmation it exists to
       prevent. -->
{:else if loading}
  <Skeleton lines={4} />
{:else if error}
  <ErrorSummary {error} />
{:else if response}
  <section class="preread" aria-labelledby="preread-heading">
    <h3 id="preread-heading">Pre-read assessment</h3>

    {#if response.status === 'assessed'}
      {#if !response.report.complete}
        <!--
          Shown above the numbers, not below. A partial report whose limitation is only in
          a footer reads as a full one at a glance, and "half of this was not assessable"
          has to change how the reader treats everything under it.
        -->
        <p class="partial" role="status">
          Partial — {response.report.missing.length} of the configured dimensions did not
          come back. Treat the scores below as incomplete.
        </p>
      {/if}

      <!-- Worst first, straight from the server's ordering. No total, no average. -->
      <ol class="dimensions">
        {#each response.report.dimensions as entry (entry.dimension)}
          <li>
            <span class="dimension-name">{entry.dimension}</span>
            <span class="dimension-score">{entry.score.toFixed(2)}</span>
            {#if entry.note}
              <span class="dimension-note">{entry.note}</span>
            {/if}
          </li>
        {/each}
      </ol>

      {#if response.report.missing.length > 0}
        <details>
          <summary>Not assessed ({response.report.missing.length})</summary>
          <ul class="missing">
            {#each response.report.missing as entry (entry.dimension)}
              <li>
                <span class="dimension-name">{entry.dimension}</span>
                <span class="missing-reason">{entry.reason}</span>
              </li>
            {/each}
          </ul>
        </details>
      {/if}
    {:else if response.status === 'not_assessed'}
      <!--
        Distinct from the partial case above: nothing has looked at this work yet, which is
        the normal state. Saying so plainly is better than rendering an empty score list,
        which would look like a work that scored nothing on everything.
      -->
      <p class="empty">No provider has assessed this work yet.</p>
    {:else}
      <!-- Only reachable through a concurrent withdrawal, so it is worded as the odd case
           it is rather than as another way of saying "not assessed". -->
      <p class="empty">
        A provider is listed for this work but its report is no longer stored.
      </p>
    {/if}

    {#if response.providers.length > 0}
      <div class="providers">
        <h4>Assessed by</h4>
        <ul>
          {#each response.providers as provider (provider)}
            <li>
              <span class="provider-name">{provider}</span>
              <button
                type="button"
                disabled={withdrawing === provider}
                onclick={() => withdraw(provider)}
              >
                {withdrawing === provider ? 'Withdrawing…' : 'Withdraw'}
              </button>
            </li>
          {/each}
        </ul>
        <p class="hint">
          Withdrawing removes this provider's assessment of this work. Other providers' output
          is kept.
        </p>
      </div>
    {/if}
  </section>
{/if}

<style>
  .preread {
    border: 1px solid var(--border, #d4d4d8);
    border-radius: 8px;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  /* The limitation is styled as a warning and sits above the numbers, because a partial
     report read at a glance is a full one. */
  .partial {
    color: var(--warning-fg, #92400e);
    background: var(--warning-bg, #fef3c7);
    border-radius: 6px;
    padding: 0.5rem 0.75rem;
    margin: 0;
  }

  .dimensions,
  .missing,
  .providers ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .dimensions li,
  .missing li {
    display: grid;
    grid-template-columns: minmax(6rem, 10rem) auto 1fr;
    gap: 0.5rem;
    align-items: baseline;
  }

  .dimension-name {
    font-weight: 600;
  }

  .dimension-score {
    font-variant-numeric: tabular-nums;
  }

  .dimension-note,
  .missing-reason,
  .hint,
  .empty {
    color: var(--fg-muted, #52525b);
    font-size: 0.9em;
  }

  .empty,
  .hint {
    margin: 0;
  }
</style>