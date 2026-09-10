<script lang="ts">
  /**
   * The caller's own queue (spec §10.1).
   *
   * What this page is for: a request that hands work to the queue answers `202`
   * with a job id, and the reader of the page wants to know *whether it
   * finished*. So the page shows state, progress and — when something went
   * wrong — the reason, and offers the one action the server allows: cancel.
   *
   * Three deliberate choices:
   *
   *  * **Polling stops when nothing is running.** A queue of finished jobs is
   *    not polled for ever; the timer runs only while a job can still change.
   *  * **Cancel is offered only where the server says it is allowed.** The
   *    `cancellable` flag comes from the response, which is the same function
   *    the route checks — a button that always fails is worse than no button.
   *  * **A progress bar carries its number in text as well.** A bar alone is
   *    unreadable to anybody not looking at it.
   */
  import { cancelJob, fetchJobs, startJob, type Job } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  let jobs = $state<Job[]>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);
  let starting = $state(false);
  let busy = $state<string | null>(null);

  /** A job that can still change: queued, leased or running. */
  const ACTIVE = new Set(['queued', 'leased', 'running']);
  const hasActive = $derived(jobs.some((job) => ACTIVE.has(job.state)));

  $effect(() => {
    void session.activePseud?.id;
    void load();
  });

  /*
   * Poll only while there is something to watch. The effect re-runs when
   * `hasActive` flips, and clears its timer on the way out, so a page of
   * finished jobs makes no further requests.
   */
  $effect(() => {
    if (!hasActive || !session.isSignedIn) return;
    const timer = setInterval(() => void load({ quiet: true }), 1500);
    return () => clearInterval(timer);
  });

  async function load(options: { quiet?: boolean } = {}) {
    if (!options.quiet) loading = true;
    error = null;
    try {
      const view = await fetchJobs();
      jobs = view.items;
    } catch (failure) {
      error = failure;
      if (!options.quiet) jobs = [];
    } finally {
      loading = false;
    }
  }

  async function start() {
    starting = true;
    error = null;
    try {
      const job = await startJob({ task: 'probe', steps: 10, delay_ms: 300 });
      jobs = [job, ...jobs];
    } catch (failure) {
      error = failure;
    } finally {
      starting = false;
    }
  }

  async function cancel(job: Job) {
    busy = job.id;
    error = null;
    try {
      const updated = await cancelJob(job.id);
      jobs = jobs.map((candidate) => (candidate.id === updated.id ? updated : candidate));
    } catch (failure) {
      error = failure;
      // The row may have been cancelled by another tab; re-read rather than
      // leaving the page showing a stale state.
      await load({ quiet: true });
    } finally {
      busy = null;
    }
  }

  function percent(job: Job): number {
    return Math.max(0, Math.min(1000, job.progress_permille)) / 10;
  }
</script>

<h1>Jobs</h1>

{#if !session.isSignedIn}
  <p class="note">
    <a href="/sign-in" onclick={(event) => handleLinkClick(event, '/sign-in')}>Sign in</a> to see the
    work your requests started.
  </p>
{:else if loading}
  <Skeleton lines={4} />
{:else}
  <p class="note">
    Work that a request handed to the background, oldest request first. Nothing here happens while
    you wait on another page.
  </p>

  {#if error}
    <ErrorSummary error={error} />
  {/if}

  <div class="actions">
    <button type="button" onclick={start} disabled={starting}>
      {starting ? 'Starting…' : 'Start a diagnostic job'}
    </button>
    <button type="button" class="quiet" onclick={() => void load()} disabled={loading}>
      Refresh
    </button>
  </div>

  {#if jobs.length === 0}
    <p class="note">Nothing in the queue. Jobs you start appear here.</p>
  {:else}
    <ul class="jobs">
      {#each jobs as job (job.id)}
        <li class="job">
          <div class="row">
            <span class="kind">{job.kind}</span>
            <span class="state" data-state={job.state}>{job.state}</span>
            <span class="date">{job.created_at.slice(0, 19).replace('T', ' ')}</span>
            {#if job.cancellable}
              <button
                type="button"
                class="quiet"
                onclick={() => cancel(job)}
                disabled={busy === job.id}
                aria-label="Cancel the {job.kind} job created at {job.created_at}"
              >
                {busy === job.id ? 'Cancelling…' : 'Cancel'}
              </button>
            {/if}
          </div>

          {#if job.state === 'running' || job.state === 'leased' || job.progress_permille > 0}
            <div class="progress">
              <progress max="100" value={percent(job)}></progress>
              <span class="meta">
                {Math.floor(percent(job))}%{job.checkpoint ? ` — ${job.checkpoint}` : ''}
              </span>
            </div>
          {/if}

          {#if job.state === 'queued' && job.attempts > 0}
            <p class="meta">
              Waiting to retry (attempt {job.attempts} of {job.max_attempts}), next at
              {job.available_at.slice(0, 19).replace('T', ' ')}.
            </p>
          {/if}

          {#if job.last_error}
            <p class="failure">{job.last_error}</p>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
{/if}

<style>
  .note,
  .meta {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .actions {
    display: flex;
    gap: var(--space-3);
    margin: var(--space-4) 0;
  }

  .jobs {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .job {
    padding: var(--space-2) var(--space-3);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
  }

  .row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .kind {
    font-weight: 600;
    flex: 1;
    min-width: 10ch;
  }

  .state {
    font-size: var(--text-sm);
    text-transform: lowercase;
    padding: 0 var(--space-2);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-sm);
  }

  .state[data-state='failed'] {
    border-color: var(--color-danger, #b3261e);
  }

  .date {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .progress {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-top: var(--space-2);
  }

  .progress progress {
    flex: 1;
    max-width: 24rem;
  }

  .failure {
    margin: var(--space-2) 0 0;
    color: var(--color-danger, #b3261e);
    font-size: var(--text-sm);
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
