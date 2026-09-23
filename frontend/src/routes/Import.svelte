<script lang="ts">
  /**
   * Import a work from another site (spec §11.2, §11.4).
   *
   * The page is built around one promise: **nothing is stored until the reader
   * has seen what would be stored.** So it is two steps rather than one. The
   * first asks the source what it holds and the server what it would do with it;
   * the second confirms. The confirmation carries the plan the reader was
   * shown, and the server re-derives it and refuses if it has moved, which is
   * what stops a consent from being applied to something it did not describe.
   *
   * Three deliberate choices:
   *
   *  * **The plan is shown as actions, not as a diff of counts.** "Adds 12
   *    chapters" is a number; the reader confirming an import wants to know
   *    what will change.
   *  * **A dry run is offered rather than implied.** It records the plan and
   *    stores no chapter, which is the honest way to answer "what would this
   *    do" for a work somebody is unsure about.
   *  * **Capability absence is stated.** A source this build cannot read is not
   *    offered silently; the catalogue says which sources exist and what each
   *    can do (spec §11.1).
   */
  import {
    cancelImport,
    fetchImportSources,
    fetchImports,
    previewImport,
    retryFailedChapters,
    startImport,
    type ImportJobView,
    type ImportSource,
    type PreviewView,
  } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte.ts';
  import Button from '../lib/components/Button.svelte';
  import ClampedText from '../lib/components/ClampedText.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Select from '../lib/components/Select.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import TextField from '../lib/components/TextField.svelte';

  let url = $state('');
  let previewing = $state(false);
  let preview = $state<PreviewView | null>(null);
  let previewError = $state<unknown>(null);

  let destination = $state('library');
  let dryRun = $state(false);
  let starting = $state(false);
  let startError = $state<unknown>(null);
  let started = $state<{ import_id: string; state: string } | null>(null);

  let sources = $state<ImportSource[]>([]);
  let imports = $state<ImportJobView[]>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);
  let busy = $state<string | null>(null);

  /** A job that can still change: worth polling for. */
  const ACTIVE = new Set(['queued', 'pending', 'running', 'leased']);
  const hasActive = $derived(imports.some((row) => ACTIVE.has(row.state)));

  /*
   * The destination list is fixed and mostly disabled, and that is the honest
   * shape. Spec §11.2 requires an explicit choice among four destinations; this
   * build can carry out one of them, so the others are shown as unavailable
   * rather than omitted — a reader who expects to import into a draft should
   * learn that here, not from a refusal after confirming.
   */
  const destinations = [
    { value: 'library', label: 'Into my library' },
    { value: 'draft', label: 'Into a new draft of mine (not yet available)', disabled: true },
    { value: 'republication', label: 'Republish with the author’s permission (not yet available)', disabled: true },
    { value: 'preservation', label: 'Into the preservation archive (operator only, not yet available)', disabled: true },
  ];

  $effect(() => {
    void session.activePseud?.id;
    void load();
  });

  $effect(() => {
    if (!hasActive || !session.isSignedIn) return;
    const timer = setInterval(() => void load({ quiet: true }), 2000);
    return () => clearInterval(timer);
  });

  async function load(options: { quiet?: boolean } = {}) {
    if (!options.quiet) loading = true;
    error = null;
    try {
      const [catalogue, listing] = await Promise.all([fetchImportSources(), fetchImports()]);
      sources = catalogue.items;
      imports = listing.items;
    } catch (failure) {
      error = failure;
      if (!options.quiet) imports = [];
    } finally {
      loading = false;
    }
  }

  async function check() {
    previewing = true;
    previewError = null;
    preview = null;
    started = null;
    try {
      preview = await previewImport(url.trim(), session.activePseud?.id);
    } catch (failure) {
      previewError = failure;
    } finally {
      previewing = false;
    }
  }

  async function confirm() {
    if (!preview) return;
    starting = true;
    startError = null;
    try {
      started = await startImport({
        url: preview.source_url,
        destination,
        dry_run: dryRun,
        confirmed_plan: preview.plan.plan,
        pseud_id: session.activePseud?.id,
      });
      preview = null;
      await load({ quiet: true });
    } catch (failure) {
      startError = failure;
    } finally {
      starting = false;
    }
  }

  async function cancel(row: ImportJobView) {
    busy = row.id;
    error = null;
    try {
      await cancelImport(row.id);
      await load({ quiet: true });
    } catch (failure) {
      error = failure;
      await load({ quiet: true });
    } finally {
      busy = null;
    }
  }

  async function retry(row: ImportJobView) {
    busy = row.id;
    error = null;
    try {
      await retryFailedChapters(row.id);
      await load({ quiet: true });
    } catch (failure) {
      error = failure;
    } finally {
      busy = null;
    }
  }

  /**
   * Queue the same address again.
   *
   * This is the update check: a preview of a work already held answers
   * `update` or `no_change` with the chapters that moved, so asking again is
   * how the reader learns a source has published more.
   */
  function checkForUpdates(row: ImportJobView) {
    url = row.source_url;
    void check();
  }

  /** One line of plain language for what a plan would do. */
  function describe(plan: PreviewView['plan']): string {
    if (plan.plan === 'create') return 'This would be imported as a new work.';
    if (plan.plan === 'no_change') return 'Nothing has changed since this was imported.';
    const parts: string[] = [];
    if (plan.added) parts.push(`${plan.added} added`);
    if (plan.removed) parts.push(`${plan.removed} removed`);
    if (plan.reordered) parts.push(`${plan.reordered} reordered`);
    if (plan.retitled) parts.push(`${plan.retitled} retitled`);
    return parts.length
      ? `This would update the stored copy: ${parts.join(', ')}.`
      : 'This would update the stored copy.';
  }

  /**
   * The last sentence the worker wrote about an import, if it wrote one.
   *
   * `report_json` is a free-form object, so this pulls the fields that are
   * known to hold prose and shows nothing rather than stringifying the whole
   * object into a reader's page.
   */
  function activeReport(row: ImportJobView): string {
    const report = row.report;
    if (!report) return '';
    const summary = report.summary ?? report.message ?? report.detail;
    if (typeof summary === 'string') return summary;
    // §32.7.9: surface media rescue stats when present.
    const media = report.media_rescue;
    if (media && typeof media === 'object') {
      const m = media as Record<string, unknown>;
      const total = typeof m.total_urls === 'number' ? m.total_urls : 0;
      const held = typeof m.already_held === 'number' ? m.already_held : 0;
      const created = typeof m.new_references === 'number' ? m.new_references : 0;
      const bad = typeof m.unparseable === 'number' ? m.unparseable : 0;
      if (total > 0) {
        const parts = [`${total} media URLs found`];
        if (held > 0) parts.push(`${held} already held`);
        if (created > 0) parts.push(`${created} new`);
        if (bad > 0) parts.push(`${bad} broken`);
        return `Media rescue: ${parts.join(', ')}.`;
      }
    }
    return '';
  }

  function sourceLabel(key: string): string {
    return sources.find((source) => source.key === key)?.display_name ?? key;
  }

  function when(value: string | null): string {
    return value ? value.slice(0, 19).replace('T', ' ') : '—';
  }

  let enabledSources = $derived(sources.filter((source) => source.enabled));
  let unavailableSources = $derived(sources.filter((source) => !source.enabled));
  // The terms are instance-wide, so any entry answers for all of them; the
  // catalogue repeats them per source so that the source that refuses and the
  // rule that refused it are on the same line of the page.
  let robots = $derived(sources.find((source) => source.robots)?.robots ?? null);
</script>

<h1>Import</h1>

{#if !session.isSignedIn}
  <p class="note">
    <a href="/sign-in" onclick={(event) => handleLinkClick(event, '/sign-in')}>Sign in</a> to import a
    work from another site into your library.
  </p>
{:else}
  <p class="note">
    Paste the address of a work on a site this instance can read. Nothing is fetched into your
    library until you have seen what would be fetched and confirmed it.
  </p>

  {#if error}
    <ErrorSummary error={error} />
  {/if}

  <form
    class="checker"
    onsubmit={(event) => {
      event.preventDefault();
      void check();
    }}
  >
    <TextField
      label="Address of the work"
      bind:value={url}
      placeholder="https://www.royalroad.com/fiction/21220/…"
      hint="The page of the work itself, not a chapter of it."
      required
    />
    <div class="actions">
      <Button type="submit" loading={previewing} disabled={!url.trim()}>
        {previewing ? 'Reading the source…' : 'Check what would happen'}
      </Button>
    </div>
  </form>

  {#if previewError}
    <ErrorSummary error={previewError} />
  {/if}

  {#if preview}
    <section class="preview" aria-labelledby="preview-heading">
      <h2 id="preview-heading">What confirming would do</h2>

      <div class="work">
        <h3>{preview.title}</h3>
        <p class="byline">
          by
          {#if preview.author_url}
            <a href={preview.author_url} rel="noreferrer noopener" target="_blank">{preview.author_text}</a>
          {:else}
            {preview.author_text || 'an unknown author'}
          {/if}
          on {sourceLabel(preview.source_key)}
        </p>
        <ul class="facts">
          <li>{preview.chapter_count} {preview.chapter_count === 1 ? 'chapter' : 'chapters'}</li>
          {#if preview.word_count !== null}
            <li>{preview.word_count.toLocaleString()} words</li>
          {/if}
          <li>{preview.status}</li>
          {#if preview.language}<li>{preview.language}</li>{/if}
        </ul>
        {#if preview.summary}
          <!--
            The same treatment the library gives a work's summary, because it is
            the same text and could be the same mistake. Six lines rather than
            four: the reader is here to decide on this one work, so the summary
            carries more of the decision than it does in a list.
          -->
          <ClampedText
            text={preview.summary}
            lines={6}
            moreLabel="Show the whole summary"
          />
        {/if}
        <p class="meta">
          Source address:
          <a href={preview.source_url} rel="noreferrer noopener" target="_blank"
            >{preview.source_url}</a
          >
        </p>
      </div>

      {#if preview.duplicate_warning}
        <p class="warning" role="status">{preview.duplicate_warning}</p>
      {/if}

      <p class="plan" data-plan={preview.plan.plan}>{describe(preview.plan)}</p>

      {#if preview.chapters.length > 0}
        <details class="chapters">
          <summary>{preview.chapter_count} chapters, as the source lists them</summary>
          <ol>
            {#each preview.chapters.slice(0, 200) as chapter (chapter.ordinal)}
              <li>{chapter.title || `Chapter ${chapter.ordinal}`}</li>
            {/each}
          </ol>
          {#if preview.chapters.length > 200}
            <p class="note">Showing the first 200. All of them would be imported.</p>
          {/if}
        </details>
      {/if}

      <div class="confirm">
        <Select label="Destination" bind:value={destination} options={destinations} />
        <label class="dry-run">
          <input type="checkbox" bind:checked={dryRun} />
          Dry run — record what would happen and store no chapters
        </label>
        <div class="actions">
          <Button onclick={confirm} loading={starting} disabled={destination !== 'library'}>
            {dryRun ? 'Run without storing' : `Import ${preview.chapter_count} chapters`}
          </Button>
          <Button
            variant="quiet"
            onclick={() => {
              preview = null;
              previewError = null;
            }}
          >
            Cancel
          </Button>
        </div>
      </div>

      {#if startError}
        <ErrorSummary error={startError} />
      {/if}
    </section>
  {/if}

  {#if started}
    <p class="started" role="status">
      Import queued. It runs in the background —
      <a href="/jobs" onclick={(event) => handleLinkClick(event, '/jobs')}>watch it on the jobs page</a
      >. You can close this page.
    </p>
  {/if}

  <section class="history" aria-labelledby="imports-heading">
    <h2 id="imports-heading">Your imports</h2>

    {#if loading}
      <Skeleton lines={3} />
    {:else if imports.length === 0}
      <p class="note">Nothing imported yet. Addresses you import appear here with their state.</p>
    {:else}
      <ul class="imports">
        {#each imports as row (row.id)}
          <li class="import">
            <div class="row">
              <span class="title">
                {#if row.library_item_id}
                  <a
                    href={`/works/${encodeURIComponent(row.library_item_id)}`}
                    onclick={(event) => handleLinkClick(event, `/works/${row.library_item_id}`)}
                  >
                    {sourceLabel(row.source_key)}
                  </a>
                {:else}
                  {sourceLabel(row.source_key)}
                {/if}
              </span>
              <span class="state" data-state={row.state}>{row.state}</span>
              {#if row.dry_run}<span class="tag">dry run</span>{/if}
              <span class="date">{when(row.created_at)}</span>
            </div>

            <p class="meta break">{row.source_url}</p>

            {#if activeReport(row)}
              <p class="meta">{activeReport(row)}</p>
            {/if}

            <div class="actions">
              {#if row.cancellable}
                <Button
                  variant="quiet"
                  size="sm"
                  onclick={() => cancel(row)}
                  disabled={busy === row.id}
                  aria-label={`Cancel the import of ${row.source_url}`}
                >
                  {busy === row.id ? 'Cancelling…' : 'Cancel'}
                </Button>
              {/if}
              {#if row.state === 'failed'}
                <Button
                  variant="quiet"
                  size="sm"
                  onclick={() => retry(row)}
                  disabled={busy === row.id}
                  aria-label={`Retry the failed chapters of ${row.source_url}`}
                >
                  {busy === row.id ? 'Queueing…' : 'Retry failed chapters'}
                </Button>
              {/if}
              {#if row.state === 'completed' || row.state === 'failed' || row.state === 'cancelled'}
                <Button
                  variant="quiet"
                  size="sm"
                  onclick={() => checkForUpdates(row)}
                  aria-label={`Check ${row.source_url} for changes`}
                >
                  Check for updates
                </Button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <section class="catalogue" aria-labelledby="sources-heading">
    <h2 id="sources-heading">Sources</h2>
    <p class="note">
      What this instance can read, and what each source is able to do. A source that cannot list
      chapters says so here rather than returning an empty work.
    </p>

    {#if loading}
      <Skeleton lines={2} />
    {:else}
      <ul class="sources">
        {#each enabledSources as source (source.key)}
          <li>
            <span class="source-name">{source.display_name}</span>
            <span class="meta">
              {source.adapter_version} · {source.health}
              {source.capabilities.per_chapter_fetch ? ' · per-chapter fetch' : ''}
              {source.capabilities.authentication && source.capabilities.authentication !== 'none'
                ? ` · needs ${source.capabilities.authentication}`
                : ''}
            </span>
          </li>
        {/each}
        {#each unavailableSources as source (source.key)}
          <li class="off">
            <span class="source-name">{source.display_name}</span>
            <span class="meta">unavailable{source.disabled_reason ? `: ${source.disabled_reason}` : ''}</span>
          </li>
        {/each}
      </ul>

      {#if robots}
        <p class="note" class:override={!robots.honour_disallow}>
          {robots.note}{#if robots.honour_crawl_delay}{' '}Each source's published crawl delay is
            enforced either way.{/if}
        </p>
      {/if}
    {/if}
  </section>
{/if}

<style>
  .note,
  .meta {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  /* An instance that has stopped asking is a state a reader should notice
     rather than read past, so the sentence is marked rather than merely present. */
  .note.override {
    color: var(--color-warning, var(--color-muted));
    border-left: 2px solid currentColor;
    padding-left: var(--space-2);
  }

  .break {
    overflow-wrap: anywhere;
  }

  .checker {
    max-width: 44rem;
    margin: var(--space-5) 0;
  }

  .actions {
    display: flex;
    gap: var(--space-3);
    margin: var(--space-4) 0;
    flex-wrap: wrap;
  }

  .preview {
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    padding: var(--space-4);
    margin: var(--space-5) 0;
  }

  .preview h3 {
    margin: 0 0 var(--space-2);
  }

  .byline {
    margin: 0 0 var(--space-3);
    color: var(--color-muted);
  }

  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    list-style: none;
    padding: 0;
    margin: 0 0 var(--space-3);
    font-size: var(--text-sm);
  }

  .facts li {
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: 0 var(--space-2);
  }



  .plan {
    font-weight: 600;
    margin: var(--space-4) 0;
  }

  .warning {
    border-left: 3px solid var(--color-accent);
    padding-left: var(--space-3);
    margin: var(--space-4) 0;
  }

  .chapters {
    margin: var(--space-4) 0;
  }

  .chapters ol {
    columns: 2;
    max-width: 60rem;
  }

  .confirm {
    border-top: var(--border-width) solid var(--color-border);
    padding-top: var(--space-4);
    margin-top: var(--space-4);
    max-width: 44rem;
  }

  .dry-run {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    margin: var(--space-3) 0;
  }

  .started {
    border-left: 3px solid var(--color-accent);
    padding-left: var(--space-3);
  }

  .imports,
  .sources {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .import {
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

  .title {
    font-weight: 600;
    flex: 1;
    min-width: 12ch;
  }

  .state,
  .tag {
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

  .sources li {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    align-items: baseline;
  }

  .source-name {
    font-weight: 600;
    min-width: 14ch;
  }

  .off .source-name {
    color: var(--color-muted);
  }
</style>
