<script lang="ts">
  /**
   * Admin media health dashboard (spec §32.7.11).
   *
   * Six read-only metric panels for the operator: overall health, link rot,
   * curator leaderboard, bounty status, storage, and provider reliability.
   *
   * The server decides who may see this, and it now says so the way every other
   * operator surface in the tree does: **404**, because for an operator view the
   * existence is the disclosure (`flows`, `admin_discovery`, `decision_service`).
   * This page used to read only 401, so a signed-in non-operator got six 403s and
   * a red "That did not work / access denied" — six failures for a page they were
   * never going to be allowed to see. It reads 403 and 404 alike now, and the gate
   * is checked before any of the six is called.
   */
  import {
    fetchMediaHealthOverview,
    fetchLinkRotReport,
    fetchCuratorLeaderboard,
    fetchBountyStatus,
    fetchStorageStatus,
    fetchProviderReliability,
    type MediaHealthOverview,
    type LinkRotReport,
    type CuratorLeader,
    type BountyStatus,
    type StorageStatus,
    type ProviderReliability,
  } from '../lib/api';
  import { session } from '../lib/session.svelte.ts';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import SignInGate from '../lib/components/SignInGate.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  let overview = $state<MediaHealthOverview | null>(null);
  let linkRot = $state<LinkRotReport | null>(null);
  let leaders = $state<CuratorLeader[] | null>(null);
  let bounties = $state<BountyStatus | null>(null);
  let storage = $state<StorageStatus | null>(null);
  let providers = $state<ProviderReliability[] | null>(null);

  let loading = $state(true);
  let error = $state<unknown>(null);
  /**
   * Set when the server refused: 401 (nobody signed in) or 404 (signed in, not the
   * operator — this instance's deliberate answer rather than a 403).
   *
   * One flag for both, because the page's response to either is the same honest
   * sentence about who may see this, and keeping them apart would invite a
   * third branch that renders a 404 as an error.
   */
  let forbidden = $state(false);

  function isRefusal(failure: unknown): boolean {
    if (typeof failure !== 'object' || failure === null || !('status' in failure)) {
      return false;
    }
    const status = (failure as { status?: number }).status;
    return status === 401 || status === 403 || status === 404;
  }

  async function load() {
    loading = true;
    error = null;
    forbidden = false;
    try {
      const [o, lr, cl, bs, st, pr] = await Promise.all([
        fetchMediaHealthOverview(),
        fetchLinkRotReport(),
        fetchCuratorLeaderboard(),
        fetchBountyStatus(),
        fetchStorageStatus(),
        fetchProviderReliability(),
      ]);
      overview = o;
      linkRot = lr;
      leaders = cl.curators;
      bounties = bs;
      storage = st;
      providers = pr.providers;
    } catch (failure) {
      if (isRefusal(failure)) {
        forbidden = true;
      } else {
        error = failure;
      }
    } finally {
      loading = false;
    }
  }

  function fmtBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
    return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }

  $effect(() => {
    void session.activePseud?.id;
    if (session.isSignedIn) void load();
  });
</script>

<svelte:head><title>Media health · Lorehaven</title></svelte:head>

<h1>Media health</h1>

<SignInGate purpose="see this instance's media health metrics" skeletonLines={10}>
{#if loading}
  <Skeleton lines={10} label="Loading media health metrics" />
{:else if forbidden}
  <!--
    The instance operator, named by configuration — not a trust tier, which is what
    the server actually checks (`media_health::require_operator` reads
    `administration.operator_account_id`). A non-operator is answered 404 rather
    than 403 because for an operator view the existence is the disclosure; this note
    says so plainly instead of rendering the red panel that six 403s used to produce.
  -->
  <p class="note" data-testid="media-health-refused">
    This page belongs to the instance's operator. If that is you and you cannot see it, check that
    the account is configured as this instance's operator.
  </p>
{:else if error}
  <ErrorSummary error={error} onretry={() => void load()} />
{:else if overview}
  <section class="panel" aria-label="Overall media health">
    <h2>Overall health</h2>
    <div class="metrics-grid">
      <div class="metric">
        <span class="metric-value">{overview.health_pct}%</span>
        <span class="metric-label">well-mirrored (≥ 3 links)</span>
      </div>
      <div class="metric">
        <span class="metric-value">{overview.total_references}</span>
        <span class="metric-label">total references</span>
      </div>
      <div class="metric">
        <span class="metric-value">{overview.below_threshold}</span>
        <span class="metric-label">below threshold</span>
      </div>
      <div class="metric">
        <span class="metric-value">{overview.well_mirrored}</span>
        <span class="metric-label">healthy references</span>
      </div>
    </div>
  </section>

  {#if linkRot}
    <section class="panel" aria-label="Link rot by provider">
      <h2>Link rot</h2>
      <p class="note">Since {linkRot.since.slice(0, 10)}. Total dead: {linkRot.total_rot}</p>
      {#if linkRot.by_provider.length > 0}
        <table>
          <thead>
            <tr><th scope="col">Provider</th><th scope="col">Dead links</th></tr>
          </thead>
          <tbody>
            {#each linkRot.by_provider as row (row.provider)}
              <tr>
                <td>{row.provider}</td>
                <td>{row.dead}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else}
        <p class="note">No link rot recorded in this window.</p>
      {/if}
    </section>
  {/if}

  {#if bounties}
    <section class="panel" aria-label="Bounty status">
      <h2>Standing bounties</h2>
      <div class="metrics-grid">
        <div class="metric">
          <span class="metric-value">{bounties.active_bounties}</span>
          <span class="metric-label">active bounties</span>
        </div>
        <div class="metric">
          <span class="metric-value">{bounties.total_available}</span>
          <span class="metric-label">credits available</span>
        </div>
      </div>
    </section>
  {/if}

  {#if storage}
    <section class="panel" aria-label="Storage usage">
      <h2>Storage</h2>
      <div class="metrics-grid">
        <div class="metric">
          <span class="metric-value">{storage.local_mirrors}</span>
          <span class="metric-label">local mirrors</span>
        </div>
        <div class="metric">
          <span class="metric-value">{fmtBytes(storage.total_bytes)}</span>
          <span class="metric-label">mirror storage used</span>
        </div>
        <div class="metric">
          <span class="metric-value">{storage.ipfs_pins}</span>
          <span class="metric-label">active IPFS pins</span>
        </div>
      </div>
    </section>
  {/if}

  {#if providers && providers.length > 0}
    <section class="panel" aria-label="Provider reliability ranking">
      <h2>Provider reliability</h2>
      <table>
        <thead>
          <tr>
            <th scope="col">Provider</th>
            <th scope="col">Healthy</th>
            <th scope="col">Total</th>
            <th scope="col">Health rate</th>
          </tr>
        </thead>
        <tbody>
          {#each providers as p (p.provider)}
            <tr>
              <td>{p.provider}</td>
              <td>{p.healthy}</td>
              <td>{p.total}</td>
              <td>{(p.health_rate * 100).toFixed(1)}%</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>
  {/if}

  {#if leaders && leaders.length > 0}
    <section class="panel" aria-label="Top curators">
      <h2>Curator leaderboard</h2>
      <table>
        <thead>
          <tr>
            <th scope="col">Account</th>
            <th scope="col">Rewards</th>
            <th scope="col">Actions</th>
          </tr>
        </thead>
        <tbody>
          {#each leaders as l (l.account_id)}
            <tr>
              <td class="meta">{l.account_id}</td>
              <td>{l.total_rewards}</td>
              <td>{l.actions}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>
  {/if}
{/if}
</SignInGate>

<style>
  h1 {
    margin-block: var(--space-4) var(--space-6);
  }
  h2 {
    font-size: var(--text-lg);
    margin-bottom: var(--space-3);
  }
  .panel {
    border: 1px solid var(--color-border);
    border-radius: 8px;
    padding: var(--space-4);
    margin-bottom: var(--space-6);
  }
  .metrics-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
    gap: var(--space-4);
  }
  .metric {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .metric-value {
    font-size: var(--text-2xl);
    font-weight: 700;
  }
  .metric-label {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }
  .note,
  .meta {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th,
  td {
    padding: var(--space-2) var(--space-3);
    text-align: left;
    border-bottom: 1px solid var(--color-border);
    font-size: var(--text-sm);
  }
  th {
    font-weight: 600;
  }
  .note {
    margin-bottom: var(--space-3);
  }
</style>
