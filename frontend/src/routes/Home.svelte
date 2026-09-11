<script lang="ts">
  import Skeleton from '../lib/components/Skeleton.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import MetadataChip from '../lib/components/MetadataChip.svelte';
  import {
    fetchInstanceMeta,
    fetchReadiness,
    type InstanceMeta,
    type ReadinessReport,
  } from '../lib/api';

  let meta = $state<InstanceMeta | null>(null);
  let readiness = $state<ReadinessReport | null>(null);
  let error = $state<unknown>(null);
  let loading = $state(true);

  /**
   * Both calls are real. Nothing on this page is placeholder text dressed up as
   * data: if the server cannot answer, the page says so (spec §1.1).
   */
  async function load() {
    loading = true;
    error = null;
    try {
      const [metaResult, readinessResult] = await Promise.all([
        fetchInstanceMeta(),
        fetchReadiness(),
      ]);
      meta = metaResult;
      readiness = readinessResult;
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void load();
  });
</script>

<section class="hero">
  <h1>Read, write, and keep what you love.</h1>
  <p class="lede">
    Lorehaven is a home for fanfiction: a place to read without an account, write
    without a publishing queue, import the library you already have, and take it
    offline when you want it.
  </p>

  <div class="search-stub">
    <!--
      Deliberately disabled rather than decorative. Search arrives with the
      structured taxonomy in Milestone 9, and a field that silently does
      nothing would be worse than one that explains itself.
    -->
    <label class="visually-hidden" for="hero-search">Search works</label>
    <input
      id="hero-search"
      type="search"
      placeholder="Search is not available yet"
      disabled
      aria-describedby="search-note"
    />
    <p id="search-note" class="note">
      Search and discovery arrive with Milestone 9 and Milestone 10. The
      catalog below is real, and it will fill in as those milestones land.
    </p>
  </div>
</section>

{#if error}
  <ErrorSummary {error} onretry={load} />
{/if}

<div class="panels">
  <section class="panel" aria-labelledby="instance-heading">
    <h2 id="instance-heading">This instance</h2>
    {#if loading && !meta}
      <Skeleton lines={4} label="Loading instance details" />
    {:else if meta}
      <dl class="facts">
        <dt>Name</dt>
        <dd>{meta.name}</dd>
        <dt>Build</dt>
        <dd><code>{meta.build}</code></dd>
        <dt>Environment</dt>
        <dd>{meta.environment}</dd>
        <dt>API</dt>
        <dd>{meta.api_version}</dd>
      </dl>

      <h3>Content policy</h3>
      <ul class="policy">
        <li>
          Anonymous reading
          <strong>{meta.policy.anonymous_reading ? 'available' : 'disabled'}</strong>
        </li>
        <li>
          Visitors may read up to
          <strong>{meta.policy.anonymous_max_rating}</strong>
        </li>
        <li>
          Accounts of unknown age may read up to
          <strong>{meta.policy.unknown_age_max_rating}</strong>
        </li>
        <li>
          Declared minors may read up to
          <strong>{meta.policy.minor_max_rating}</strong>
        </li>
        <li>
          Registration
          <strong>{meta.policy.registration_open ? 'open' : 'closed'}</strong>
        </li>
      </ul>
      <p class="note">
        These values come from <code>/api/v1/meta</code>, and the same policy is
        enforced server-side — the interface never decides eligibility.
      </p>
    {:else}
      <p>Instance details are unavailable.</p>
    {/if}
  </section>

  <section class="panel" aria-labelledby="health-heading">
    <h2 id="health-heading">Service health</h2>
    {#if loading && !readiness}
      <Skeleton lines={3} label="Checking service health" />
    {:else if readiness}
      <ul class="checks">
        {#each Object.entries(readiness.checks) as [name, check] (name)}
          <li class:ok={check.ok} class:bad={!check.ok}>
            <span class="dot" aria-hidden="true"></span>
            <span class="check-name">{name}</span>
            <span class="check-detail">{check.detail}</span>
            {#if !check.ok && check.remedy}
              <span class="remedy">{check.remedy}</span>
            {/if}
          </li>
        {/each}
      </ul>
    {:else}
      <p>Health information is unavailable.</p>
    {/if}
  </section>
</div>

<section class="panel wide" aria-labelledby="progress-heading">
  <h2 id="progress-heading">What is built so far</h2>
  <p class="note">
    Each row reflects <code>docs/verification.md</code>. Nothing is claimed as
    finished until it runs and its tests pass.
  </p>
  <ul class="milestones">
    <li>
      <MetadataChip label="Milestone 0" tone="primary" />
      <span>Running application: workspace, configuration, migrations, health checks, seeding, embedded frontend.</span>
    </li>
    <li>
      <MetadataChip label="Milestone 1" tone="accent" />
      <span>Design system and navigation shell: tokens, components, keyboard and focus behaviour.</span>
    </li>
    <li>
      <MetadataChip label="Milestone 2" />
      <span>Accounts, pseuds, privacy and age policy — next.</span>
    </li>
  </ul>
</section>

<style>
  .hero {
    padding: var(--space-7) 0 var(--space-5);
    border-bottom: var(--border-width) solid var(--color-border);
    margin-bottom: var(--space-6);
  }

  .hero h1 {
    max-width: 24ch;
  }

  .lede {
    font-size: var(--text-lg);
    color: var(--color-muted);
    max-width: 48ch;
  }

  .search-stub input {
    font: inherit;
    width: 100%;
    max-width: 34rem;
    padding: var(--space-3) var(--space-4);
    border: var(--border-width) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background: var(--color-surface);
    color: var(--color-text);
  }

  .search-stub input:disabled {
    opacity: 0.75;
    cursor: not-allowed;
  }

  .note {
    font-size: var(--text-sm);
    color: var(--color-muted);
    max-width: 48ch;
  }

  .panels {
    display: grid;
    gap: var(--space-5);
    grid-template-columns: 1fr;
  }

  @media (min-width: 52rem) {
    .panels {
      grid-template-columns: 1fr 1fr;
    }
  }

  .panel {
    background: var(--color-surface);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-lg);
    padding: var(--space-5);
  }

  .wide {
    margin-top: var(--space-5);
  }

  .panel h2 {
    font-size: var(--text-xl);
  }

  .panel h3 {
    font-size: var(--text-base);
    margin-top: var(--space-4);
    color: var(--color-muted);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-family: var(--font-interface);
  }

  .facts {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: var(--space-2) var(--space-4);
    margin: 0;
  }

  .facts dt {
    color: var(--color-muted);
  }

  .facts dd {
    margin: 0;
  }

  .policy {
    margin: 0;
    padding-left: var(--space-5);
    color: var(--color-muted);
  }

  .policy strong {
    color: var(--color-text);
  }

  .checks {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .checks li {
    display: grid;
    grid-template-columns: auto auto 1fr;
    gap: var(--space-3);
    align-items: baseline;
    font-size: var(--text-sm);
  }

  .dot {
    width: 0.5rem;
    height: 0.5rem;
    border-radius: 50%;
    background: var(--color-success);
    align-self: center;
  }

  .bad .dot {
    background: var(--color-danger);
  }

  .check-name {
    font-weight: 600;
    text-transform: capitalize;
  }

  .check-detail {
    color: var(--color-muted);
    /* The server writes this line, and it names a connection: a URL with no
       space in it. Without this the detail sets the panel's minimum width and
       pushes the whole page sideways at 320 CSS pixels. */
    overflow-wrap: anywhere;
  }

  .remedy {
    grid-column: 3;
    color: var(--color-danger);
  }

  .milestones {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .milestones li {
    display: flex;
    gap: var(--space-3);
    align-items: baseline;
    flex-wrap: wrap;
  }

  .milestones span {
    color: var(--color-muted);
    flex: 1;
    min-width: 14rem;
  }
</style>
