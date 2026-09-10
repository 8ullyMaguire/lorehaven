<script lang="ts">
  export interface JobState {
    /** What the job is doing, e.g. "Importing chapters". */
    label: string;
    status: 'queued' | 'running' | 'succeeded' | 'failed' | 'canceled' | 'retry_wait';
    /** 0–100, when known. */
    percent?: number;
    /** Human-readable summary, e.g. "12 of 40 chapters". */
    detail?: string;
    /** Stable error code, shown without internal detail (spec §5). */
    errorCode?: string;
  }

  interface Props {
    job: JobState;
  }

  let { job }: Props = $props();

  const STATUS_LABEL: Record<JobState['status'], string> = {
    queued: 'Queued',
    running: 'In progress',
    succeeded: 'Done',
    failed: 'Failed',
    canceled: 'Cancelled',
    retry_wait: 'Waiting to retry',
  };

  let indeterminate = $derived(job.status === 'running' && job.percent === undefined);
</script>

<div class="job" data-status={job.status}>
  <div class="head">
    <span class="label">{job.label}</span>
    <span class="status">{STATUS_LABEL[job.status]}</span>
  </div>

  <div
    class="track"
    role="progressbar"
    aria-valuemin="0"
    aria-valuemax="100"
    aria-valuenow={job.percent ?? undefined}
    aria-valuetext={job.percent === undefined ? STATUS_LABEL[job.status] : `${job.percent}%`}
    aria-label={job.label}
  >
    <div
      class="fill"
      class:indeterminate
      style={job.percent === undefined ? undefined : `width:${Math.max(0, Math.min(100, job.percent))}%`}
    ></div>
  </div>

  {#if job.detail}<p class="detail">{job.detail}</p>{/if}

  {#if job.status === 'failed' && job.errorCode}
    <!--
      Job errors are redacted before display (spec §5): the user gets a stable
      code and the request id, never a stack trace or a URL with credentials.
    -->
    <p class="error" role="alert">Failed with code <code>{job.errorCode}</code>.</p>
  {/if}
</div>

<style>
  .job {
    padding: var(--space-3) var(--space-4);
    background: var(--color-surface);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
  }

  .head {
    display: flex;
    justify-content: space-between;
    gap: var(--space-3);
    font-size: var(--text-sm);
    margin-bottom: var(--space-2);
  }

  .label {
    font-weight: 600;
  }

  .status {
    color: var(--color-muted);
  }

  .track {
    height: 0.4rem;
    background: var(--color-border);
    border-radius: var(--radius-pill);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--color-primary);
    transition: width var(--duration-base) ease;
  }

  .job[data-status='failed'] .fill {
    background: var(--color-danger);
  }

  .indeterminate {
    width: 40%;
    animation: slide 1.4s ease-in-out infinite;
  }

  @keyframes slide {
    0% {
      margin-left: 0;
    }
    50% {
      margin-left: 60%;
    }
    100% {
      margin-left: 0;
    }
  }

  .detail {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
    color: var(--color-muted);
  }

  .error {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
    color: var(--color-danger);
  }
</style>
