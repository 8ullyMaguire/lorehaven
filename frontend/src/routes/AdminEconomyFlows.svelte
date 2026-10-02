<script lang="ts">
  /**
   * The operator's faucet/sink view (spec §53, row M45-18).
   *
   * §53.2 asks for "a balance and a composition", so this renders both: the three
   * totals, and every mechanism that moved credits. The composition is the useful
   * part -- an operator told only that 4,200 credits went unclassified cannot do
   * anything about them.
   *
   * Three things this deliberately does not do:
   *
   *   - **No per-account anything.** §53.2 forbids it, and there is nothing here a
   *     reader should see. The server sends aggregate mechanism names only.
   *   - **Nothing is hidden.** An undeclared mechanism renders with a visible warning
   *     rather than being filtered out. §53.1: a dashboard that silently omits what
   *     nobody classified reports a smaller economy than exists -- and a warning the
   *     operator can act on is the whole point of the view.
   *   - **The threshold is not an action.** `over_threshold` is rendered as a note.
   *     §0.3 makes bought ranking and bought trust non-negotiable, so nothing here
   *     clamps, suspends or adjusts a balance.
   *
   * The server decides who may see this. A non-operator gets 404 -- the same answer
   * as "no such endpoint" -- and the page says so without pretending otherwise.
   */
  import {
    fetchEconomyFlows,
    type EconomyFlows,
    type FlowMechanism,
  } from '../lib/api';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  let flows = $state<EconomyFlows | null>(null);
  let loading = $state(true);
  let error = $state<unknown>(null);
  let notFound = $state(false);

  function isNotFound(failure: unknown): boolean {
    return (
      typeof failure === 'object' &&
      failure !== null &&
      'status' in failure &&
      (failure as { status?: number }).status === 404
    );
  }

  async function load() {
    loading = true;
    error = null;
    notFound = false;
    try {
      flows = await fetchEconomyFlows();
    } catch (failure) {
      notFound = isNotFound(failure);
      error = failure;
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void load();
  });

  const declared = $derived(
    (flows?.mechanisms ?? []).filter((m) => m.declared),
  );
  const undeclared = $derived(
    (flows?.mechanisms ?? []).filter((m) => !m.declared),
  );

  /** Credits, with the sign shown so a negative total is never read as a positive one. */
  function credits(value: number): string {
    return value.toLocaleString();
  }

  function sideLabel(m: FlowMechanism): string {
    switch (m.flow) {
      case 'faucet':
        return 'faucet — pays into the loop';
      case 'sink':
        return 'sink — drains it';
      case 'neutral':
        return 'neutral — moves credits without changing the total';
      default:
        return 'undeclared — nobody has said which side this is';
    }
  }
</script>

<section aria-labelledby="flows-heading">
  <h2 id="flows-heading">Faucets and sinks</h2>

  {#if loading}
    <Skeleton lines={4} label="Loading the economy" />
  {:else if notFound}
    <!--
      404, not 403, and the page says exactly that. A 403 would confirm this
      instance runs an economy dashboard, and for an operator view the existence
      is itself the disclosure.
    -->
    <p class="notice">
      No such page. If you are the operator of this instance and expected to see this,
      check that the account is configured as its operator.
    </p>
  {:else if error}
    <ErrorSummary {error} />
  {:else if flows}
    <!--
      `role="status"` rather than an alert: crossing the threshold is information
      for an operator, not an error, and an aria-live assertive region would
      interrupt whatever they were doing.
    -->
    <div class="totals" role="status">
      <div class="total">
        <span class="label">Faucets</span>
        <span class="value" data-testid="faucet-credits">{credits(flows.faucet_credits)}</span>
      </div>
      <div class="total">
        <span class="label">Sinks</span>
        <span class="value" data-testid="sink-credits">{credits(flows.sink_credits)}</span>
      </div>
      <div class="total">
        <span class="label">Net</span>
        <span class="value" data-testid="net-credits">{credits(flows.net_credits)}</span>
      </div>
    </div>

    <p class="window">
      Window: {flows.since} to {flows.until}. {flows.note}
    </p>

    {#if flows.over_threshold}
      <!--
        A note, not an action. The threshold is `net > threshold` and nothing on
        this page changes because of it -- no clamp, no suspension. Reporting it is
        the whole of its job.
      -->
      <p class="threshold" data-testid="over-threshold">
        Net credits are above the configured threshold of {credits(flows.threshold)}. Reported
        for a human to act on; nothing has been adjusted or suspended.
      </p>
    {/if}

    <h3>Declared mechanisms</h3>
    {#if declared.length === 0}
      <p>No declared mechanism moved credits in this window.</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th scope="col">Mechanism</th>
            <th scope="col">Side</th>
            <th scope="col">Net credits</th>
          </tr>
        </thead>
        <tbody>
          {#each declared as m (m.key)}
            <tr data-testid="mechanism-{m.key}">
              <td>{m.key}</td>
              <td>{sideLabel(m)}</td>
              <td>{credits(m.net_credits)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}

    <!--
      The undeclared section is rendered whenever there is one, never hidden behind a
      disclosure the operator has to go looking for. It is the number that tells them
      their classification is incomplete, which is the only reason to build the view.
    -->
    {#if undeclared.length > 0}
      <h3 data-testid="undeclared-heading">
        Undeclared ({flows.undeclared})
      </h3>
      <p class="warn" data-testid="undeclared-warning">
        These moved credits but nobody has declared which side of the loop they are on. Their
        credits are in the net above — they are not missing from the economy, they are
        unclassified in it.
      </p>
      <table>
        <thead>
          <tr>
            <th scope="col">Mechanism</th>
            <th scope="col">Net credits</th>
          </tr>
        </thead>
        <tbody>
          {#each undeclared as m (m.key)}
            <tr data-testid="undeclared-{m.key}">
              <td>{m.key}</td>
              <td>{credits(m.net_credits)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {/if}
</section>

<style>
  .totals {
    display: flex;
    gap: 2rem;
    flex-wrap: wrap;
  }
  .total {
    display: flex;
    flex-direction: column;
  }
  .label {
    font-size: 0.85rem;
    opacity: 0.8;
  }
  .value {
    font-size: 1.4rem;
    font-variant-numeric: tabular-nums;
  }
  .window {
    opacity: 0.8;
    font-size: 0.9rem;
  }
  .threshold {
    border-left: 3px solid currentColor;
    padding-left: 0.75rem;
  }
  .warn {
    border-left: 3px solid #b8860b;
    padding-left: 0.75rem;
  }
  .notice {
    opacity: 0.8;
  }
  table {
    border-collapse: collapse;
    width: 100%;
  }
  th,
  td {
    text-align: left;
    padding: 0.35rem 0.6rem;
    border-bottom: 1px solid color-mix(in srgb, currentColor 15%, transparent);
  }
  td:last-child {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
</style>
