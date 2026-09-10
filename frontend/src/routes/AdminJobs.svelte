<script lang="ts">
  /**
   * The operator's view of the queue (spec §10.1).
   *
   * Who may see this is decided by the server, not by this page:
   * `config.administration.operator_account_id` names one account, and everyone
   * else gets a `404` — so the page's job is to render that honestly rather
   * than to guess at a role. There is no staff model yet; Milestone 13 replaces
   * the setting with a trust level.
   *
   * What it shows that `/jobs` does not: every job on the instance, including
   * ones with no owner, and the two operator actions — filter, and retry a job
   * that has finished failing.
   */
  import { fetchAllJobs, retryJob, type Job } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  /*
   * The states are the server's vocabulary, listed so the filter cannot offer
   * a spelling the server would refuse.
   */
  const STATES = ['queued', 'leased', 'running', 'succeeded', 'failed', 'cancelled'] as const;
  const ACTIVE = new Set(['queued', 'leased', 'running']);

  let jobs = $state<Job[]>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);
  let forbidden = $state(false);
  let state = $state('');
  let busy = $state<string | null>(null);

  const hasActive = $derived(jobs.some((job) => ACTIVE.has(job.state)));

  $effect(() => {
    void state;
    void session.activePseud?.id;
    void load();
  });

  $effect(() => {
    if (!hasActive || forbidden || !session.isSignedIn) return;
    const timer = setInterval(() => void load({ quiet: true }), 1500);
    return () => clearInterval(timer);
  });

  async function load(options: { quiet?: boolean } = {}) {
    if (!options.quiet) loading = true;
    error = null;
    try {
      const view = await fetchAllJobs(state ? { state } : {});
      jobs = view.items;
      forbidden = false;
    } catch (failure) {
      // A 404 here is the answer for "not the operator", and it is not an
      // error the operator can act on — so it is not shown as one.
      forbidden = isNotFound(failure);
      if (!forbidden) error = failure;
      if (!options.quiet) jobs = [];
    } finally {
      loading = false;
    }
  }

  function isNotFound(failure: unknown): boolean {
    return (
      typeof failure === 'object' &&
      failure !== null &&
      'status' in failure &&
      (failure as { status?: number }).status === 404
    );
  }

  async function retry(job: Job) {
    busy = job.id;
    error = null;
    try {
      const updated = await retryJob(job.id);
      jobs = jobs.map((candidate) => (candidate.id === updated.id ? updated : candidate));
    } catch (failure) {
      error = failure;
      await load({ quiet: true });
    } finally {
      busy = null;
    }
  }

  function percent(job: Job): number {
    return Math.max(0, Math.min(1000, job.progress_permille)) / 10;
  }
</script>

<h1>Queue</h1>

{#if !session.isSignedIn}
  <p class="note">
    <a href="/sign-in" onclick={(event) => handleLinkClick(event, '/sign-in')}>Sign in</a> to see the
    queue.
  </p>
{:else if loading}
  <Skeleton lines={6} />
{:else if forbidden}
  <p class="note">
    This page belongs to the instance's operator. No account is configured as one, so nobody can
    open it: set <code>LOREHAVEN_OPERATOR_ACCOUNT_ID</code> to an account's id. Milestone 13
    replaces that single-account gate with a trust level.
  </p>
{:else}
  {#if error}
    <ErrorSummary error={error} />
  {/if}

  <div class="filters">
    <label for="state-filter">State</label>
    <select id="state-filter" bind:value={state}>
      <option value="">all</option>
      {#each STATES as candidate (candidate)}
        <option value={candidate}>{candidate}</option>
      {/each}
    </select>
    <button type="button" class="quiet" onclick={() => void load()} disabled={loading}>
      Refresh
    </button>
  </div>

  {#if jobs.length === 0}
    <p class="note">Nothing matches. The queue is empty or the filter excludes everything.</p>
  {:else}
    <table>
      <caption>{jobs.length} {jobs.length === 1 ? 'job' : 'jobs'}</caption>
      <thead>
        <tr>
          <th scope="col">Kind</th>
          <th scope="col">State</th>
          <th scope="col">Progress</th>
          <th scope="col">Attempts</th>
          <th scope="col">Owner</th>
          <th scope="col">Created</th>
          <th scope="col">Action</th>
        </tr>
      </thead>
      <tbody>
        {#each jobs as job (job.id)}
          <tr>
            <td>{job.kind}</td>
            <td>
              <span class="state" data-state={job.state}>{job.state}</span>
              {#if job.last_error}
                <span class="failure" title={job.last_error}>{job.last_error}</span>
              {/if}
            </td>
            <td>
              {Math.floor(percent(job))}%{job.checkpoint ? ` — ${job.checkpoint}` : ''}
            </td>
            <td>{job.attempts} / {job.max_attempts}</td>
            <td class="meta">{job.requested_by ?? '—'}</td>
            <td class="meta">{job.created_at.slice(0, 19).replace('T', ' ')}</td>
            <td>
              {#if ACTIVE.has(job.state)}
                <span class="meta">running</span>
              {:else}
                <button
                  type="button"
                  class="quiet"
                  onclick={() => retry(job)}
                  disabled={busy === job.id}
                  aria-label="Retry the {job.kind} job created at {job.created_at}"
                >
                  {busy === job.id ? 'Retrying…' : 'Retry'}
                </button>
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
{/if}

<style>
  .note,
  .meta {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .filters {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin: var(--space-4) 0;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-sm);
  }

  caption {
    text-align: left;
    color: var(--color-muted);
    padding-bottom: var(--space-2);
  }

  th,
  td {
    text-align: left;
    padding: var(--space-2);
    border-bottom: var(--border-width) solid var(--color-border);
    vertical-align: top;
  }

  .state {
    padding: 0 var(--space-2);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-sm);
  }

  .state[data-state='failed'] {
    border-color: var(--color-danger, #b3261e);
  }

  .failure {
    display: block;
    margin-top: var(--space-1);
    color: var(--color-danger, #b3261e);
    max-width: 32ch;
    overflow-wrap: anywhere;
  }

  .quiet {
    background: none;
    border: var(--border-width) solid var(--color-border);
    color: var(--color-muted);
    font-size: var(--text-sm);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
  }
</style>
