<script lang="ts">
  /**
   * Author media dashboard (spec §32.7.8).
   *
   * Three surfaces in one place, because they share auth and intent:
   *   1. Media health report — per-work link health at a glance.
   *   2. Insert media — paste a URL and the system dedups/reuses before
   *      it ever stores a duplicate reference.
   *   3. Preferences — auto-submit, curator edits, minimum healthy links.
   */

  import {
    fetchAuthorMediaHealth,
    fetchMediaPreferences,
    fetchWorkMediaReferences,
    postMediaReference,
    postTargetedBounty,
    putMediaPreferences,
    reportBrokenLink,
    type AuthorMediaHealthRow,
    type MediaPreferences,
    type WorkMediaReferenceView,
  } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte';
  import Button from '../lib/components/Button.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import TextField from '../lib/components/TextField.svelte';

  type Tab = 'health' | 'insert' | 'preferences';

  let activeTab = $state<Tab>('health');
  let health = $state<AuthorMediaHealthRow[] | null>(null);
  let healthLoading = $state(true);
  let healthError = $state<unknown>(null);

  let prefs = $state<MediaPreferences | null>(null);
  let prefsError = $state<unknown>(null);
  let prefsSaving = $state(false);
  let prefsNotice = $state<string | null>(null);

  // Insertion form.
  let insertUrl = $state('');
  let insertContext = $state('reference');
  let insertWorkId = $state('');
  let insertNote = $state('');
  let insertBusy = $state(false);
  let insertError = $state<unknown>(null);
  let insertNotice = $state<string | null>(null);
  let insertExistingRef = $state<boolean | null>(null);
  let previewRefs = $state<WorkMediaReferenceView[] | null>(null);

  // Bounty form.
  let bountyWorkId = $state('');
  let bountyRefId = $state('');
  let bountyReward = $state(10);
  let bountyDesc = $state('');
  let bountyBusy = $state(false);
  let bountyError = $state<unknown>(null);
  let bountyNotice = $state<string | null>(null);

  const CONTEXT_OPTIONS = [
    { value: 'reference', label: 'Reference' },
    { value: 'faceclaim', label: 'Faceclaim' },
    { value: 'moodboard', label: 'Moodboard' },
    { value: 'fanart', label: 'Fanart' },
    { value: 'playlist', label: 'Playlist' },
    { value: 'inline_embed', label: 'Inline Embed' },
  ];

  $effect(() => {
    if (!session.me) return;
    void loadHealth();
  });

  async function loadHealth() {
    healthLoading = true;
    healthError = null;
    try {
      const res = await fetchAuthorMediaHealth();
      health = res.items;
    } catch (failure) {
      healthError = failure;
    } finally {
      healthLoading = false;
    }
  }

  async function loadPrefs() {
    prefsError = null;
    try {
      prefs = await fetchMediaPreferences();
    } catch (failure) {
      prefsError = failure;
    }
  }

  async function savePrefs(event: SubmitEvent) {
    event.preventDefault();
    if (!prefs) return;
    prefsSaving = true;
    prefsError = null;
    prefsNotice = null;
    try {
      await putMediaPreferences({
        auto_submit_to_archive: prefs.auto_submit_to_archive,
        prefer_curator_verified: prefs.prefer_curator_verified,
        broken_link_notifications: prefs.broken_link_notifications,
        allow_curator_edits: prefs.allow_curator_edits,
        minimum_healthy_links: prefs.minimum_healthy_links,
      });
      prefsNotice = 'Preferences updated.';
    } catch (failure) {
      prefsError = failure;
    } finally {
      prefsSaving = false;
    }
  }

  async function lookupUrl() {
    if (!insertWorkId.trim()) {
      insertError = 'Enter a work ID first.';
      return;
    }
    insertBusy = true;
    insertError = null;
    previewRefs = null;
    try {
      const res = await fetchWorkMediaReferences(insertWorkId.trim());
      previewRefs = res.items;
      if (res.items.some((r) => r.display_url === insertUrl.trim())) {
        insertExistingRef = true;
        insertNotice = 'This URL is already referenced — it will be reused.';
      } else {
        insertExistingRef = null;
        insertNotice = null;
      }
    } catch (failure) {
      insertError = failure;
    } finally {
      insertBusy = false;
    }
  }

  async function insertReference() {
    if (!insertUrl.trim() || !insertWorkId.trim()) {
      insertError = 'Work ID and URL are required.';
      return;
    }
    insertBusy = true;
    insertError = null;
    insertNotice = null;
    insertExistingRef = null;
    try {
      await postMediaReference({
        work_id: insertWorkId.trim(),
        url: insertUrl.trim(),
        context: insertContext,
        author_note: insertNote.trim() || undefined,
      });
      insertNotice = 'Reference added.';
      insertUrl = '';
      insertNote = '';
      previewRefs = null;
      if (session.me) {
        await loadHealth();
      }
    } catch (failure) {
      insertError = failure;
    } finally {
      insertBusy = false;
    }
  }

  async function submitBounty() {
    if (!bountyWorkId.trim() || bountyReward <= 0) {
      bountyError = 'Work ID and a positive reward are required.';
      return;
    }
    bountyBusy = true;
    bountyError = null;
    bountyNotice = null;
    try {
      await postTargetedBounty({
        work_id: bountyWorkId.trim(),
        media_reference_id: bountyRefId.trim() || undefined,
        reward: bountyReward,
        description: bountyDesc.trim() || undefined,
      });
      bountyNotice = 'Bounty posted.';
      bountyDesc = '';
      bountyReward = 10;
    } catch (failure) {
      bountyError = failure;
    } finally {
      bountyBusy = false;
    }
  }

  async function onReportBroken(referenceId: string) {
    try {
      await reportBrokenLink(referenceId);
      if (session.me) {
        await loadHealth();
      }
    } catch {
      /* per-refailure error surfaced inline elsewhere */
    }
  }


  const totalRefs = $derived(
    health?.reduce((acc, r) => acc + r.total_references, 0) ?? 0,
  );
  const totalHealthy = $derived(
    health?.reduce((acc, r) => acc + r.healthy_references, 0) ?? 0,
  );
  const totalBroken = $derived(
    health?.reduce((acc, r) => acc + r.broken_references, 0) ?? 0,
  );
</script>

<svelte:head><title>Media | Lorehaven</title></svelte:head>

<div class="page">
  <nav class="tabs" aria-label="Author media sections">
    <button
      class="tab"
      class:active={activeTab === 'health'}
      onclick={() => (activeTab = 'health')}
    >
      Health Report
    </button>
    <button
      class="tab"
      class:active={activeTab === 'insert'}
      onclick={() => (activeTab = 'insert')}
    >
      Insert Media
    </button>
    <button
      class="tab"
      class:active={activeTab === 'preferences'}
      onclick={() => {
        activeTab = 'preferences';
        if (!prefs) void loadPrefs();
      }}
    >
      Preferences
    </button>
  </nav>

  {#if activeTab === 'health'}
    <section class="section">
      <h1>Your Works — Media Health Report</h1>
      <p class="lead">
        Every media reference, grouped by health. References with 3+ healthy
        links need nothing; 1-2 need mirroring; 0 are broken.
      </p>

      {#if totalRefs > 0}
        <div class="summary-row">
          <div class="summary-card">
            <div class="summary-num">{totalRefs}</div>
            <div class="summary-label">Total refs</div>
          </div>
          <div class="summary-card ok">
            <div class="summary-num">{totalHealthy}</div>
            <div class="summary-label">Healthy (3+ links)</div>
          </div>
          <div class="summary-card danger">
            <div class="summary-num">{totalBroken}</div>
            <div class="summary-label">Broken (0 links)</div>
          </div>
        </div>
      {/if}

      {#if healthLoading}
        <Skeleton lines={4} />
      {:else if healthError}
        <ErrorSummary error={healthError} />
      {:else if health && health.length === 0}
        <EmptyState
          title="No media yet"
          description="Add media references from the Insert Media tab."
        />
      {:else if health}
        <ul class="health-list">
          {#each health as row (row.work_id)}
            <li class="health-row">
              <div class="health-row-main">
                <a href={`/works/${row.work_id}`} onclick={(event) => handleLinkClick(event, `/works/${row.work_id}`)}>
                  {row.work_title}
                </a>
                <span class="muted">{row.total_references} refs</span>
              </div>
              <div class="health-badges">
                {#if row.healthy_references > 0}
                  <span class="badge badge-ok">{row.healthy_references} healthy</span>
                {/if}
                {#if row.at_risk_references > 0}
                  <span class="badge badge-warn">{row.at_risk_references} at risk</span>
                {/if}
                {#if row.broken_references > 0}
                  <span class="badge badge-danger">{row.broken_references} broken</span>
                {/if}
              </div>
              <div class="health-row-actions">
                <Button
                  size="sm"
                  variant="quiet"
                  onclick={() => {
                    bountyWorkId = row.work_id;
                    activeTab = 'insert';
                  }}
                >
                  Add bounty
                </Button>
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  {:else if activeTab === 'insert'}
    <section class="section">
      <h1>Insert Media Reference</h1>
      <p class="lead">
        Paste a URL. If it is already in the system, the existing reference is
        reused — no duplicates.
      </p>

      <form class="form" onsubmit={(e) => { e.preventDefault(); void insertReference(); }}>
                  <TextField label="Work ID" bind:value={insertWorkId} placeholder="insert-work-uuid" />
                  <TextField label="URL" bind:value={insertUrl} placeholder="https://..." onchange={() => void lookupUrl()} />
        <label>
          Context
          <select bind:value={insertContext}>
            {#each CONTEXT_OPTIONS as opt}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </label>
                  <TextField label="Note (optional)" bind:value={insertNote} placeholder="Source, artist, etc." />

        {#if insertNotice}
          <p class="notice">{insertNotice}</p>
        {/if}
        {#if insertExistingRef === true}
          <p class="notice warn">This URL already exists — it will be reused.</p>
        {/if}
        {#if insertError}
          <ErrorSummary error={insertError} />
        {/if}

        <div class="form-actions">
          <Button type="submit" loading={insertBusy}>Add Reference</Button>
        </div>
      </form>

      {#if previewRefs && previewRefs.length > 0}
        <h2>Existing references for this work</h2>
        <ul class="preview-list">
          {#each previewRefs as ref (ref.id)}
            <li class="preview-row">
              <span class="muted">{ref.context}</span>
              <a href={ref.best_url ?? ref.display_url} target="_blank" rel="noreferrer">
                {ref.display_url}
              </a>
              <span class="badge badge-ok">{ref.healthy_links}/{ref.total_links} links</span>
              {#if ref.healthy_links === 0}
                <Button size="sm" variant="quiet" onclick={() => onReportBroken(ref.id)}>
                  Report broken
                </Button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}

      <h2>Post a Targeted Bounty</h2>
      <form class="form" onsubmit={(e) => { e.preventDefault(); void submitBounty(); }}>
        <TextField label="Work ID" bind:value={bountyWorkId} placeholder="work-uuid" />
        <TextField label="Media Reference ID (optional)" bind:value={bountyRefId} placeholder="ref-uuid" />
        <label>
          Reward (credits)
          <input type="number" bind:value={bountyReward} min="1" />
        </label>
                  <TextField label="Description (optional)" bind:value={bountyDesc} placeholder="What needs doing" />

        {#if bountyNotice}
          <p class="notice">{bountyNotice}</p>
        {/if}
        {#if bountyError}
          <ErrorSummary error={bountyError} />
        {/if}

        <div class="form-actions">
          <Button type="submit" loading={bountyBusy}>Post Bounty</Button>
        </div>
      </form>
    </section>
  {:else if activeTab === 'preferences'}
    <section class="section">
      <h1>Media Preferences</h1>

      {#if prefsError}
        <ErrorSummary error={prefsError} />
      {:else if !prefs}
        <Skeleton lines={3} />
      {:else}
        <form class="form" onsubmit={savePrefs}>
          <label class="checkbox">
            <input type="checkbox" bind:checked={prefs.auto_submit_to_archive} />
            Auto-submit new media to Internet Archive
          </label>
          <label class="checkbox">
            <input type="checkbox" bind:checked={prefs.prefer_curator_verified} />
            Prefer curator-verified links in recommendations
          </label>
          <label class="checkbox">
            <input type="checkbox" bind:checked={prefs.allow_curator_edits} />
            Allow curators to add mirrors to my works
          </label>
          <label>
            Broken link notifications
            <select bind:value={prefs.broken_link_notifications}>
              <option value="none">None</option>
              <option value="digest_weekly">Weekly digest</option>
              <option value="immediate">Immediate</option>
            </select>
          </label>
          <label>
            Minimum healthy links per reference
            <input type="number" bind:value={prefs.minimum_healthy_links} min="0" />
          </label>

          {#if prefsNotice}
            <p class="notice">{prefsNotice}</p>
          {/if}

          <div class="form-actions">
            <Button type="submit" loading={prefsSaving}>Save Preferences</Button>
          </div>
        </form>
      {/if}
    </section>
  {/if}
</div>

<style>
  .page {
    max-width: 800px;
    margin: 0 auto;
    padding: var(--space-6);
  }
  .tabs {
    display: flex;
    gap: var(--space-2);
    border-bottom: 1px solid var(--color-border);
    margin-bottom: var(--space-6);
  }
  .tab {
    background: none;
    border: none;
    padding: var(--space-3) var(--space-4);
    cursor: pointer;
    font-weight: 500;
    color: var(--color-muted);
    border-bottom: 2px solid transparent;
  }
  .tab.active {
    color: var(--color-text);
    border-bottom-color: var(--color-accent);
  }
  .lead {
    color: var(--color-muted);
    margin-bottom: var(--space-4);
  }
  .summary-row {
    display: flex;
    gap: var(--space-4);
    margin-bottom: var(--space-6);
  }
  .summary-card {
    flex: 1;
    padding: var(--space-4);
    border: 1px solid var(--color-border);
    border-radius: var(--radius);
    text-align: center;
  }
  .summary-card.ok {
    border-color: var(--color-success);
  }
  .summary-card.danger {
    border-color: var(--color-danger);
  }
  .summary-num {
    font-size: 1.5rem;
    font-weight: 700;
  }
  .summary-label {
    font-size: 0.85rem;
    color: var(--color-muted);
  }
  .health-list {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .health-row {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-3);
    border: 1px solid var(--color-border);
    border-radius: var(--radius);
  }
  .health-row-main {
    flex: 1;
    display: flex;
    flex-direction: column;
  }
  .health-badges {
    display: flex;
    gap: var(--space-2);
  }
  .badge {
    font-size: 0.75rem;
    padding: 2px 8px;
    border-radius: 999px;
    font-weight: 500;
  }
  .badge-ok {
    background: var(--color-success-bg);
    color: var(--color-success);
  }
  .badge-warn {
    background: var(--color-warn-bg);
    color: var(--color-warn);
  }
  .badge-danger {
    background: var(--color-danger-bg);
    color: var(--color-danger);
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    margin-bottom: var(--space-6);
  }
  .form label {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-weight: 500;
  }
  .form label.checkbox {
    flex-direction: row;
    align-items: center;
    gap: var(--space-2);
  }
  .form input[type='number'],
  .form select {
    padding: var(--space-2);
    border: 1px solid var(--color-border);
    border-radius: var(--radius);
  }
  .form-actions {
    display: flex;
    gap: var(--space-3);
  }
  .notice {
    padding: var(--space-2) var(--space-3);
    background: var(--color-success-bg);
    color: var(--color-success);
    border-radius: var(--radius);
  }
  .notice.warn {
    background: var(--color-warn-bg);
    color: var(--color-warn);
  }
  .preview-list {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .preview-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2);
    border: 1px solid var(--color-border);
    border-radius: var(--radius);
  }
  .muted {
    color: var(--color-muted);
    font-size: 0.85rem;
  }
</style>
