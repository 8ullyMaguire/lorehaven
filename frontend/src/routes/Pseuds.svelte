<script lang="ts">
  import {
    ApiError,
    createPseud,
    fetchPrivacy,
    fetchPseuds,
    patchPrivacy,
    updatePseud,
    type OwnPseud,
    // Aliased: `PrivacySettings` below is the component that renders them.
    type PrivacySettings as PrivacySettingsResponse,
  } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte';
  import Button from '../lib/components/Button.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import PrivacySettings from '../lib/components/PrivacySettings.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import TextField from '../lib/components/TextField.svelte';
  import Textarea from '../lib/components/Textarea.svelte';

  /** Which pseud has its editor or settings open, and which of the two. */
  let open = $state<{ id: string; panel: 'edit' | 'privacy' } | null>(null);

  /**
   * The two panels, flattened to a single id each.
   *
   * Derived rather than read from `open` inside the markup: the template
   * cannot narrow a nullable $state, so `open.id` would need a non-null
   * assertion in every branch that already checked it.
   */
  let editingId = $derived(open?.panel === 'edit' ? open.id : null);
  let privacyId = $derived(open?.panel === 'privacy' ? open.id : null);

  let pseuds = $state<OwnPseud[] | null>(null);
  let privacy = $state<PrivacySettingsResponse | null>(null);
  let error = $state<unknown>(null);
  let loading = $state(false);

  // Create form.
  let newHandle = $state('');
  let newDisplayName = $state('');
  let newBio = $state('');
  let creating = $state(false);
  let createError = $state<unknown>(null);
  let createFieldErrors = $state<Record<string, string>>({});

  // Edit form, keyed to one pseud at a time.
  let editDisplayName = $state('');
  let editBio = $state('');
  let editDiscoverability = $state('listed');
  let saving = $state(false);
  let editError = $state<unknown>(null);
  let editFieldErrors = $state<Record<string, string>>({});

  let busyId = $state<string | null>(null);

  $effect(() => {
    if (session.isSignedIn) void load();
  });

  async function load() {
    loading = true;
    error = null;
    try {
      const [list, privacySettings] = await Promise.all([fetchPseuds(), fetchPrivacy()]);
      pseuds = list;
      privacy = privacySettings;
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  function openEditor(pseud: OwnPseud) {
    editDisplayName = pseud.display_name;
    editBio = pseud.bio ?? '';
    editDiscoverability = pseud.discoverability;
    editError = null;
    editFieldErrors = {};
    open = { id: pseud.id, panel: 'edit' };
  }

  function togglePrivacy(pseud: OwnPseud) {
    open = open?.panel === 'privacy' && open.id === pseud.id ? null : { id: pseud.id, panel: 'privacy' };
  }

  async function create(event: SubmitEvent) {
    event.preventDefault();
    creating = true;
    createError = null;
    createFieldErrors = {};
    try {
      await createPseud({
        handle: newHandle.trim(),
        display_name: newDisplayName.trim() || undefined,
        bio: newBio.trim() || undefined,
      });
      newHandle = '';
      newDisplayName = '';
      newBio = '';
      await load();
      // A new pseud may change what the header offers, so re-read the session.
      await session.refresh();
    } catch (failure) {
      createError = failure;
      if (failure instanceof ApiError) createFieldErrors = failure.fieldErrors;
    } finally {
      creating = false;
    }
  }

  async function saveEdit(event: SubmitEvent, pseud: OwnPseud) {
    event.preventDefault();
    saving = true;
    editError = null;
    editFieldErrors = {};
    try {
      await updatePseud(pseud.id, {
        expected_version: pseud.version,
        display_name: editDisplayName.trim(),
        bio: editBio.trim() || null,
        discoverability: editDiscoverability,
      });
      open = null;
      await load();
      await session.refresh();
    } catch (failure) {
      editError = failure;
      if (failure instanceof ApiError) editFieldErrors = failure.fieldErrors;
    } finally {
      saving = false;
    }
  }

  async function makeActive(pseud: OwnPseud) {
    busyId = pseud.id;
    error = null;
    try {
      await session.usePseud(pseud.id);
      await load();
    } catch (failure) {
      error = failure;
    } finally {
      busyId = null;
    }
  }

  /** Pseud-scoped keys for one pseud, taken from the server's own schema. */
  function pseudKeys(id: string) {
    const values = privacy?.pseuds[id] ?? {};
    return (privacy?.schema ?? []).filter((key) => key.key in values);
  }

  let canBeListed = $derived(session.me?.capabilities.can_be_listed ?? false);
</script>

<section class="page">
  <h1>Your pseuds</h1>
  <p class="lede">
    A pseud is a public face. Nothing links one of your pseuds to another, or to your
    account, in anything this API returns.
  </p>

  {#if !session.isSignedIn}
    <EmptyState
      title="Sign in to manage your pseuds"
      description="A pseud is a form of participation, so it needs an account."
    >
      {#snippet action()}
        <a href="/sign-in" onclick={(event) => handleLinkClick(event, '/sign-in')}>Sign in</a>
      {/snippet}
    </EmptyState>
  {:else}
    {#if session.me?.capabilities.restriction_note}
      <p class="restriction" role="status">{session.me.capabilities.restriction_note}</p>
    {/if}

    {#if error}
      <ErrorSummary {error} onretry={load} />
    {/if}

    {#if loading && !pseuds}
      <Skeleton lines={4} label="Loading your pseuds" />
    {:else if pseuds}
      <ul class="pseuds">
        {#each pseuds as pseud (pseud.id)}
          <li class="card">
            <div class="row">
              <div class="identity">
                <span class="handle">@{pseud.handle}</span>
                <span class="name">{pseud.display_name}</span>
                <span class="badges">
                  {#if pseud.active}<span class="badge accent">Acting as this</span>{/if}
                  <span class="badge">
                    {pseud.discoverability === 'listed' ? 'Listed' : 'Hidden'}
                  </span>
                </span>
              </div>
              <div class="row-actions">
                {#if !pseud.active}
                  <Button
                    variant="secondary"
                    size="sm"
                    loading={busyId === pseud.id}
                    onclick={() => makeActive(pseud)}
                  >
                    Act as this
                  </Button>
                {/if}
                <Button
                  variant="quiet"
                  size="sm"
                  onclick={() =>
                    open?.id === pseud.id && open.panel === 'edit'
                      ? (open = null)
                      : openEditor(pseud)}
                >
                  Edit
                </Button>
                <Button variant="quiet" size="sm" onclick={() => togglePrivacy(pseud)}>
                  Privacy
                </Button>
                <a
                  class="public"
                  href={`/pseud/${encodeURIComponent(pseud.handle)}`}
                  onclick={(event) =>
                    handleLinkClick(event, `/pseud/${encodeURIComponent(pseud.handle)}`)}
                >
                  Public profile
                </a>
              </div>
            </div>

            {#if pseud.bio}
              <p class="bio">{pseud.bio}</p>
            {/if}

            {#if editingId === pseud.id}
              <form class="panel" onsubmit={(event) => saveEdit(event, pseud)} novalidate>
                {#if editError}
                  <ErrorSummary error={editError} onretry={() => openEditor(pseud)} />
                {/if}
                <TextField
                  id={`edit-name-${pseud.id}`}
                  label="Display name"
                  value={editDisplayName}
                  error={editFieldErrors.display_name}
                  oninput={(event) => (editDisplayName = event.currentTarget.value)}
                />
                <Textarea
                  id={`edit-bio-${pseud.id}`}
                  label="Biography"
                  rows={5}
                  value={editBio}
                  hint="At most 2000 characters."
                  error={editFieldErrors.bio}
                  oninput={(event) => (editBio = event.currentTarget.value)}
                />
                <fieldset class="discoverability">
                  <legend>Public profile</legend>
                  <label>
                    <input
                      type="radio"
                      name={`discoverability-${pseud.id}`}
                      value="listed"
                      checked={editDiscoverability === 'listed'}
                      onchange={() => (editDiscoverability = 'listed')}
                    />
                    Listed — visible in the author directory
                  </label>
                  <label>
                    <input
                      type="radio"
                      name={`discoverability-${pseud.id}`}
                      value="hidden"
                      checked={editDiscoverability === 'hidden'}
                      onchange={() => (editDiscoverability = 'hidden')}
                    />
                    Hidden — reachable only by its direct address
                  </label>
                </fieldset>
                <div class="panel-actions">
                  <Button type="submit" loading={saving}>Save</Button>
                  <Button variant="quiet" onclick={() => (open = null)}>Cancel</Button>
                </div>
              </form>
            {/if}

            {#if privacyId === pseud.id && privacy}
              <div class="panel">
                <PrivacySettings
                  legend={`Privacy for @${pseud.handle}`}
                  note="These apply to this pseud alone; another pseud can answer differently."
                  keys={pseudKeys(pseud.id)}
                  values={privacy.pseuds[pseud.id] ?? {}}
                  onsave={async (changes) => {
                    privacy = await patchPrivacy(changes, pseud.id);
                  }}
                />
              </div>
            {/if}
          </li>
        {/each}
      </ul>

      {#if !canBeListed}
        <p class="note">
          This account cannot hold a listed pseud under the instance's age policy, so a new
          pseud will be hidden.
        </p>
      {/if}

      <form class="panel create" onsubmit={create} novalidate>
        <h2>Another pseud</h2>
        <p class="note">
          Pseuds are not separate accounts: they share this account's email, password and
          sessions.
        </p>
        {#if createError}
          <ErrorSummary error={createError} />
        {/if}
        <TextField
          id="new-handle"
          label="Handle"
          required
          value={newHandle}
          error={createFieldErrors.handle}
          oninput={(event) => (newHandle = event.currentTarget.value)}
        />
        <TextField
          id="new-display-name"
          label="Display name"
          value={newDisplayName}
          hint="Optional; defaults to the handle."
          error={createFieldErrors.display_name}
          oninput={(event) => (newDisplayName = event.currentTarget.value)}
        />
        <Textarea
          id="new-bio"
          label="Biography"
          rows={4}
          value={newBio}
          error={createFieldErrors.bio}
          oninput={(event) => (newBio = event.currentTarget.value)}
        />
        <div class="panel-actions">
          <Button type="submit" loading={creating}>Create pseud</Button>
        </div>
      </form>
    {/if}
  {/if}
</section>

<style>
  .page {
    max-width: 52rem;
  }

  h1 {
    font-family: var(--font-heading);
    margin-bottom: var(--space-3);
  }

  .lede {
    color: var(--color-muted);
    max-width: 56ch;
    margin-bottom: var(--space-5);
  }

  .restriction {
    border-left: 3px solid var(--color-accent);
    background: var(--color-accent-soft);
    padding: var(--space-3);
    border-radius: var(--radius-sm);
    margin: 0 0 var(--space-4);
  }

  .pseuds {
    list-style: none;
    padding: 0;
    margin: 0 0 var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .card {
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    padding: var(--space-4);
    background: var(--color-surface);
  }

  .row {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: var(--space-4);
    flex-wrap: wrap;
  }

  .identity {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 12rem;
  }

  .handle {
    font-family: var(--font-heading);
    font-size: var(--text-lg);
    font-weight: 600;
  }

  .name {
    color: var(--color-muted);
  }

  .badges {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
    margin-top: var(--space-1);
  }

  .badge {
    font-size: var(--text-xs);
    border: var(--border-width) solid var(--color-border-strong);
    border-radius: 999px;
    padding: 0 var(--space-2);
    color: var(--color-muted);
  }

  .badge.accent {
    border-color: var(--color-accent);
    background: var(--color-accent-soft);
    color: var(--color-text);
  }

  .row-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .bio {
    margin: var(--space-3) 0 0;
    white-space: pre-wrap;
    max-width: 60ch;
  }

  .panel {
    margin-top: var(--space-4);
    padding-top: var(--space-4);
    border-top: var(--border-width) solid var(--color-border);
  }

  .panel-actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-top: var(--space-2);
  }

  .discoverability {
    border: 0;
    padding: 0;
    margin: 0 0 var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .discoverability legend {
    font-size: var(--text-sm);
    font-weight: 600;
    padding: 0;
    margin-bottom: var(--space-2);
  }

  .discoverability label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }

  .create {
    max-width: 36rem;
  }

  .create h2 {
    margin-top: 0;
    font-family: var(--font-heading);
  }

  .note {
    color: var(--color-muted);
    font-size: var(--text-sm);
    max-width: 60ch;
  }

  .public {
    font-size: var(--text-sm);
  }
</style>
