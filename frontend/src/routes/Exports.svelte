<script lang="ts">
  /**
   * Exports and offline copies (spec §13).
   *
   * Three things live here, and they are the three a reader needs in one place:
   *
   *  * **What this instance can produce**, including the formats it cannot, and
   *    what an operator would have to install. A format picker that silently
   *    omits PDF is a picker that makes the reader think they missed it.
   *  * **Their exports**, with the state of each, so a queued EPUB is a thing
   *    they can see rather than a button that appeared to do nothing.
   *  * **The copies this browser is holding**, with how much space they take and
   *    one way to let go of them.
   *
   * A subject arrives in the query string, because that is how a reader starts:
   * from a work in their library or from a work they wrote, pressing Export. The
   * format and the acknowledgement are chosen *here*, which is the only place
   * the notice is shown — the server refuses an export whose reader has not been
   * told, so the notice is a step rather than a paragraph.
   */
  import {
    deleteExport,
    exportDownloadUrl,
    fetchExport,
    fetchExportFormats,
    fetchExports,
    requestExport,
    type ExportFormatCatalogue,
    type ExportJob,
  } from '../lib/api';
  import {
    clearCopies,
    copiesUsage,
    listCopies,
    removeCopy,
    requestPersistence,
    saveCopy,
    type OfflineCopy,
  } from '../lib/offline';
  import { session } from '../lib/session.svelte.ts';
  import { formatBytes } from '../lib/labels';
  import { formatTimestamp } from '../lib/time';
  import Button from '../lib/components/Button.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Select from '../lib/components/Select.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  /** The subject the reader arrived with, if they arrived from a work. */
  const params = new URLSearchParams(typeof window === 'undefined' ? '' : window.location.search);
  const subjectType = params.get('subject_type');
  const subjectId = params.get('subject_id');
  const subjectTitle = params.get('title') ?? 'this work';

  let catalogue = $state<ExportFormatCatalogue | null>(null);
  let exports = $state<ExportJob[]>([]);
  let copies = $state<OfflineCopy[]>([]);
  let usage = $state({ count: 0, bytes: 0 });
  let loading = $state(true);
  let error = $state<unknown>(null);
  let busy = $state<string | null>(null);
  let notice = $state<string | null>(null);

  /** What the reader picked, and whether they have read the notice. */
  let format = $state('epub');
  let acknowledged = $state(false);
  let requested = $state(false);

  $effect(() => {
    void session.activePseud?.id;
    void load();
  });

  async function load() {
    loading = true;
    error = null;
    try {
      const [formats, listing, local] = await Promise.all([
        fetchExportFormats(),
        fetchExports(),
        listCopies(),
      ]);
      catalogue = formats;
      exports = listing.exports;
      copies = local;
      usage = await copiesUsage();
      // Default to the first format this instance can actually make, so the
      // picker never opens on one that would be refused.
      if (!formats.formats.some((entry) => entry.format === format && entry.available)) {
        format = formats.formats.find((entry) => entry.available)?.format ?? '';
      }
    } catch (failure) {
      error = failure;
      exports = [];
      copies = [];
    } finally {
      loading = false;
    }
  }

  async function start() {
    if (!subjectType || !subjectId) return;
    busy = 'start';
    error = null;
    try {
      const job = await requestExport({
        subjectType: subjectType as 'work' | 'library_item',
        subjectId,
        format,
        acknowledgePrivacy: acknowledged,
      });
      exports = [job, ...exports];
      requested = true;
      notice = 'Queued. It will appear here when it is ready.';
      // The job runs elsewhere; poll until it is no longer queued so the reader
      // does not have to reload to find out.
      void watch(job.id);
    } catch (failure) {
      error = failure;
    } finally {
      busy = null;
    }
  }

  async function watch(id: string) {
    for (let attempt = 0; attempt < 40; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, attempt < 5 ? 500 : 2000));
      try {
        const job = await fetchExport(id);
        // Race: load() may overwrite exports after start() appended the job.
        // Append if missing rather than only mapping over existing entries.
        if (exports.some((entry) => entry.id === job.id)) {
          exports = exports.map((entry) => (entry.id === job.id ? job : entry));
        } else {
          exports = [job, ...exports];
        }
        if (job.state === 'ready' || job.state === 'failed' || job.state === 'cancelled') return;
      } catch {
        // Transient error — keep polling rather than giving up.
        continue;
      }
    }
  }

  async function forget(job: ExportJob) {
    busy = job.id;
    error = null;
    try {
      await deleteExport(job.id);
      exports = exports.filter((entry) => entry.id !== job.id);
      await removeCopy(job.id);
      copies = await listCopies();
      usage = await copiesUsage();
    } catch (failure) {
      error = failure;
    } finally {
      busy = null;
    }
  }

  /** Keep a copy in this browser, which is what makes it readable offline. */
  async function keep(job: ExportJob) {
    busy = `keep-${job.id}`;
    error = null;
    notice = null;
    try {
      const response = await fetch(exportDownloadUrl(job.id), { credentials: 'same-origin' });
      if (!response.ok) throw new Error(`the download failed (${response.status})`);
      const blob = await response.blob();
      const evicted = await saveCopy({
        exportId: job.id,
        subjectId: job.subject_id,
        title: subjectTitle,
        format: job.format,
        mediaType: response.headers.get('content-type') ?? 'application/octet-stream',
        blob,
        sizeBytes: blob.size,
        savedAt: new Date().toISOString(),
      });
      await requestPersistence();
      copies = await listCopies();
      usage = await copiesUsage();
      notice =
        evicted.length > 0
          ? `Saved. To make room, ${evicted.length} older offline ${evicted.length === 1 ? 'copy was' : 'copies were'} removed.`
          : 'Saved for offline reading.';
    } catch (failure) {
      error = failure;
    } finally {
      busy = null;
    }
  }

  async function forgetCopies() {
    busy = 'copies';
    try {
      await clearCopies();
      copies = [];
      usage = await copiesUsage();
      notice = 'Offline copies removed. The exports themselves are untouched.';
    } finally {
      busy = null;
    }
  }

  /** Open a copy this browser is holding, from the bytes it holds. */
  function openCopy(copy: OfflineCopy) {
    const url = URL.createObjectURL(copy.blob);
    window.open(url, '_blank', 'noopener');
    // Revoked on the next turn: the new tab has already claimed the bytes, and
    // holding the URL alive would keep the whole file in memory.
    setTimeout(() => URL.revokeObjectURL(url), 10_000);
  }

  const stateWords: Record<string, string> = {
    queued: 'Waiting',
    running: 'Making it',
    ready: 'Ready',
    failed: 'Failed',
    cancelled: 'Cancelled',
  };
</script>

<main class="page">
  <h1>Exports</h1>

  <p class="note">
    An export is your own copy of a work, in a file you keep. Files are removed from this
    instance after {catalogue?.retention_days ?? 7} days, so download what you want to keep.
  </p>

  {#if !session.isSignedIn}
    <EmptyState
      title="Sign in to export"
      description="An export is tied to your account, so you need to be signed in to make or fetch one."
    />
  {:else if loading}
    <Skeleton lines={3} />
  {:else}
    {#if error}<ErrorSummary error={error} />{/if}
    {#if notice}<p class="notice" role="status">{notice}</p>{/if}

    {#if subjectType && subjectId}
      <section class="card" aria-labelledby="new-export">
        <h2 id="new-export">Export {subjectTitle}</h2>

        <div class="field">
          <!-- The component renders its own label, so there is one label and not
               two competing ones for the same control. -->
          <Select
            id="format"
            label="Format"
            bind:value={format}
            options={(catalogue?.formats ?? []).map((entry) => ({
              value: entry.format,
              label: entry.available ? entry.label : `${entry.label} — unavailable`,
              disabled: !entry.available,
            }))}
          />
        </div>

        {#if catalogue}
          <!-- What is missing is stated, not hidden: a reader who wanted a PDF
               learns what the operator would have to install. -->
          {#each catalogue.formats.filter((entry) => !entry.available) as entry (entry.format)}
            <p class="meta">{entry.label} is not available on this instance. {entry.requires}</p>
          {/each}
        {/if}

        <label class="ack">
          <input type="checkbox" bind:checked={acknowledged} />
          <span>{catalogue?.privacy_notice}</span>
        </label>

        <div class="actions">
          <Button
            onclick={start}
            disabled={!acknowledged || busy === 'start' || requested || !format}
          >
            {busy === 'start' ? 'Queueing…' : 'Make the export'}
          </Button>
        </div>
      </section>
    {:else}
      <p class="note">
        Open a work and choose Export to make one. Your finished exports are below.
      </p>
    {/if}

    <section aria-labelledby="my-exports">
      <h2 id="my-exports">Your exports</h2>
      {#if exports.length === 0}
        <EmptyState
          title="No exports yet"
          description="Files you ask for are listed here while they are being made, and after."
        />
      {:else}
        <ul class="items">
          {#each exports as job (job.id)}
            <li class="item">
              <div class="head">
                <h3>{job.label || job.format}</h3>
                <span class="state state-{job.state}">{stateWords[job.state] ?? job.state}</span>
              </div>
              <ul class="facts">
                <li>{formatTimestamp(job.created_at)}</li>
                {#if job.output_bytes}<li>{formatBytes(job.output_bytes)}</li>{/if}
              </ul>
              {#if job.error}
                <p class="failure">{job.error.message}</p>
              {/if}
              <div class="actions">
                {#if job.downloadable}
                  <a class="download" href={exportDownloadUrl(job.id)} download>Download</a>
                  <Button
                    variant="quiet"
                    size="sm"
                    onclick={() => keep(job)}
                    disabled={busy === `keep-${job.id}`}
                  >
                    {busy === `keep-${job.id}` ? 'Saving…' : 'Keep offline'}
                  </Button>
                {/if}
                <Button variant="quiet" size="sm" onclick={() => forget(job)} disabled={busy === job.id}>
                  Delete
                </Button>
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section aria-labelledby="offline">
      <h2 id="offline">Kept in this browser</h2>
      <p class="note">
        {#if usage.count === 0}
          Nothing is kept in this browser yet. Kept files stay on this device and are readable
          without a connection.
        {:else}
          {usage.count} {usage.count === 1 ? 'file' : 'files'}, {formatBytes(usage.bytes)}. These
          are on this device, not on the server: deleting an export here does not delete the
          copy, and deleting the copy does not delete the export.
        {/if}
      </p>
      {#if copies.length > 0}
        <ul class="items">
          {#each copies as copy (copy.exportId)}
            <li class="item">
              <div class="head">
                <h3>{copy.title}</h3>
                <span class="state">{copy.format}</span>
              </div>
              <ul class="facts">
                <li>{formatBytes(copy.sizeBytes)}</li>
                <li>saved {formatTimestamp(copy.savedAt)}</li>
              </ul>
              <div class="actions">
                <Button variant="quiet" size="sm" onclick={() => openCopy(copy)}>Open</Button>
                <Button
                  variant="quiet"
                  size="sm"
                  onclick={async () => {
                    await removeCopy(copy.exportId);
                    copies = await listCopies();
                    usage = await copiesUsage();
                  }}
                >
                  Remove
                </Button>
              </div>
            </li>
          {/each}
        </ul>
        <div class="actions">
          <Button variant="quiet" size="sm" onclick={forgetCopies} disabled={busy === 'copies'}>
            Remove all offline copies
          </Button>
        </div>
      {/if}
    </section>
  {/if}
</main>

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    max-width: 60rem;
  }

  .note,
  .meta,
  .facts {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .card {
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-width: 20rem;
  }

  .ack {
    display: flex;
    gap: var(--space-2);
    align-items: flex-start;
    font-size: var(--text-sm);
  }

  .items {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .item {
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    padding: var(--space-3);
  }

  .head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    list-style: none;
    padding: 0;
    margin: var(--space-2) 0;
  }

  .state {
    font-size: var(--text-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .state-ready {
    color: var(--color-success);
  }

  .state-failed {
    color: var(--color-danger);
  }

  .actions {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }

  .failure {
    color: var(--color-danger);
    font-size: var(--text-sm);
  }

  .notice {
    font-size: var(--text-sm);
  }
</style>
