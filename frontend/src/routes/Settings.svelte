<script lang="ts">
  /**
   * Unified settings surface (spec §46.5, §47).
   *
   * Groups settings by domain (search, content-filters, notifications) with
   * client-side search. Built from the same schema that validates storage.
   */
  import {
    fetchSearchSettings,
    fetchContentFilters,
    fetchNotificationRoutes,
    patchSearchSettings,
    deleteSearchSetting,
    addContentFilter,
    deleteContentFilter,
    patchNotificationRoutes,
    deleteNotificationRoute,
    exportSettings,
    importSettings,
    fetchRecEngine,
    patchRecEngine,
    fetchTokens,
    createToken,
    revokeToken,
    isKnownTokenScope,
    parseTokenScopes,
    KNOWN_TOKEN_SCOPES,
    type ApiToken,
    type RecEngineView,
    type ResolvedSetting,
    type ContentFilterView,
    type NotificationRouteView,
  } from '../lib/api';
  import { session } from '../lib/session.svelte.ts';
  import Button from '../lib/components/Button.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import Tabs from '../lib/components/Tabs.svelte';

  const TABS = [
    { id: 'search', label: 'Search Defaults' },
    { id: 'content', label: 'Content Filters' },
    { id: 'notifications', label: 'Notifications' },
    { id: 'recommendations', label: 'Recommendations' },
    { id: 'applications', label: 'Linked applications' },
  ];

  let tab = $state('search');
  let loading = $state(true);
  let error = $state<string | null>(null);
  let notice = $state<{ message: string; tone: 'info' | 'success' | 'danger' } | null>(null);

  // Search settings
  let searchSettings = $state<ResolvedSetting[]>([]);
  let newSearchKey = $state('');
  let newSearchValue = $state('');

  // Content filters
  let filters = $state<ContentFilterView[]>([]);
  let newFilterType = $state('tag');
  let newFilterValue = $state('');

  // Recommendation engine preference (M52-09, spec §16.1b)
  let recEngine = $state<RecEngineView | null>(null);
  let savingRecEngine = $state(false);
  // The select's own value, seeded from the server once it loads. Separate
  // from `recEngine.engine` so an in-progress choice is not overwritten by a
  // re-render, and so the submit button can tell "unchanged" from "changed".
  let newRecEngine = $state('');

  // Linked applications (spec §23.1, §46.5).
  let tokens = $state<ApiToken[] | null>(null);
  let tokensError = $state<string | null>(null);
  let newTokenName = $state('');
  let chosenScopes = $state<string[]>([]);
  let creatingToken = $state(false);
  let revokingId = $state<string | null>(null);
  /** The raw value, held only while the reader has not yet dismissed the
   *  one-time notice. Cleared by `dismissNewToken` and by no other code path,
   *  because there is no second chance to read it. */
  let freshToken = $state<{ token: string; id: string } | null>(null);

  // Notification routes
  let routes = $state<NotificationRouteView[]>([]);
  let newRouteEvent = $state('');
  let newRouteChannel = $state('email');
  let newRouteEnabled = $state(true);

  // Settings search (client-side, spec §46.5)
  let searchQuery = $state('');

  const FILTER_TYPES = [
    { value: 'tag', label: 'Tag' },
    { value: 'fandom', label: 'Fandom' },
    { value: 'warning', label: 'Warning' },
    { value: 'author', label: 'Author' },
  ];

  const CHANNELS = [
    { value: 'email', label: 'Email' },
    { value: 'in_app', label: 'In-App' },
  ];

  const COMMON_EVENTS = [
    'work_published',
    'chapter_published',
    'comment_received',
    'appreciation_received',
    'mention',
    'import_complete',
    'export_complete',
  ];

  async function loadAll() {
    loading = true;
    error = null;
    try {
      const [search, notif, rec] = await Promise.all([
        fetchSearchSettings(),
        fetchNotificationRoutes(),
        // A failure here must not blank the whole page: the other three tabs
        // are still usable, and this one reports its own error inline.
        fetchRecEngine().catch(() => null),
      ]);
      const cf = await fetchContentFilters();
      searchSettings = search.settings;
      filters = cf.filters;
      routes = notif.routes;
      recEngine = rec;
      newRecEngine = rec?.engine ?? '';
      // Tokens are fetched on entering the tab rather than here: they are the
      // one part of this surface a reader may not be signed in to see, and a
      // failure must not take the other four tabs down with it.
      loadTokens();
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to load settings';
    } finally {
      loading = false;
    }
  }

  // Fetch the token list when the tab is first opened, not on every render.
  $effect(() => {
    if (tab === 'applications' && tokens === null && !tokensError) {
      loadTokens();
    }
  });

  // Load on mount
  $effect(() => {
    if (session.isSignedIn) {
      loadAll();
    }
  });

  // Client-side search filtering (spec §46.5)
  const filteredSettings = $derived.by(() => {
    const q = searchQuery.toLowerCase().trim();
    if (!q) return searchSettings;
    return searchSettings.filter(
      (s) =>
        s.key.toLowerCase().includes(q) ||
        s.summary.toLowerCase().includes(q) ||
        String(s.value).toLowerCase().includes(q)
    );
  });

  async function handleSaveSearch() {
    notice = null;
    if (!newSearchKey.trim()) {
      notice = { message: 'Enter a setting key first.', tone: 'danger' };
      return;
    }
    let parsedValue: unknown = newSearchValue;
    try {
      parsedValue = JSON.parse(newSearchValue);
    } catch {
      // Keep as string
    }
    try {
      const result = await patchSearchSettings([{ key: newSearchKey, value: parsedValue }]);
      searchSettings = result.settings;
      newSearchKey = '';
      newSearchValue = '';
      notice = { message: 'Search default saved.', tone: 'success' };
    } catch (e) {
      notice = {
        message: e instanceof Error ? e.message : 'Failed to save search default',
        tone: 'danger',
      };
    }
  }

  async function handleDeleteSearch(key: string) {
    notice = null;
    try {
      await deleteSearchSetting(key);
      searchSettings = searchSettings.filter((s) => s.key !== key);
      notice = { message: `Deleted "${key}" — using inherited value.`, tone: 'success' };
    } catch (e) {
      notice = {
        message: e instanceof Error ? e.message : 'Failed to delete',
        tone: 'danger',
      };
    }
  }

  async function handleAddFilter() {
    notice = null;
    if (!newFilterValue.trim()) {
      notice = { message: 'Enter a filter value first.', tone: 'danger' };
      return;
    }
    try {
      await addContentFilter(newFilterType, newFilterValue.trim());
      const result = await fetchContentFilters();
      filters = result.filters;
      newFilterValue = '';
      notice = { message: 'Content filter added.', tone: 'success' };
    } catch (e) {
      notice = {
        message: e instanceof Error ? e.message : 'Failed to add filter',
        tone: 'danger',
      };
    }
  }

  async function handleDeleteFilter(filter: ContentFilterView) {
    notice = null;
    try {
      await deleteContentFilter(filter.filter_type, filter.value);
      filters = filters.filter(
        (f) => !(f.filter_type === filter.filter_type && f.value === filter.value)
      );
      notice = { message: 'Filter removed.', tone: 'success' };
    } catch (e) {
      notice = {
        message: e instanceof Error ? e.message : 'Failed to delete filter',
        tone: 'danger',
      };
    }
  }

  async function handleSaveRoute() {
    notice = null;
    if (!newRouteEvent.trim()) {
      notice = { message: 'Enter an event type first.', tone: 'danger' };
      return;
    }
    try {
      await patchNotificationRoutes([
        { event_type: newRouteEvent, channel: newRouteChannel, enabled: newRouteEnabled },
      ]);
      const result = await fetchNotificationRoutes();
      routes = result.routes;
      newRouteEvent = '';
      notice = { message: 'Notification route saved.', tone: 'success' };
    } catch (e) {
      notice = {
        message: e instanceof Error ? e.message : 'Failed to save route',
        tone: 'danger',
      };
    }
  }

  async function handleDeleteRoute(eventType: string) {
    notice = null;
    try {
      await deleteNotificationRoute(eventType);
      routes = routes.filter((r) => r.event_type !== eventType);
      notice = { message: `Deleted route for "${eventType}".`, tone: 'success' };
    } catch (e) {
      notice = {
        message: e instanceof Error ? e.message : 'Failed to delete route',
        tone: 'danger',
      };
    }
  }

  async function handleExport() {
    notice = null;
    try {
      const data = await exportSettings();
      const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = 'lorehaven-settings.json';
      a.click();
      URL.revokeObjectURL(url);
      notice = { message: 'Settings exported.', tone: 'success' };
    } catch (e) {
      notice = {
        message: e instanceof Error ? e.message : 'Export failed',
        tone: 'danger',
      };
    }
  }

  async function handleImport(event: Event) {
    notice = null;
    const input = event.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    try {
      const text = await file.text();
      const data = JSON.parse(text);
      const report = await importSettings(data);
      await loadAll();
      notice = {
        message: `Imported: ${report.accepted.length} accepted, ${report.rejected.length} rejected.`,
        tone: report.rejected.length > 0 ? 'info' : 'success',
      };
    } catch (e) {
      notice = {
        message: e instanceof Error ? e.message : 'Import failed',
        tone: 'danger',
      };
    }
    input.value = '';
  }

  /**
   * Set or clear the reader's engine choice (M52-09, spec §16.1b).
   *
   * The server's message is shown verbatim on a 422, because it names the
   * engines this instance accepts. Replacing it with "could not save" would
   * hide the one piece of information that lets the reader fix it.
   */
  // -------------------------------------------------------------------------
  // Linked applications
  // -------------------------------------------------------------------------

  async function loadTokens() {
    tokensError = null;
    try {
      tokens = await fetchTokens();
    } catch (e) {
      // Kept separate from the page-level `error`: a token list that fails to
      // load is one broken panel, not a broken settings page, and blanking the
      // four working tabs would make the failure look like a login problem.
      tokensError = e instanceof Error ? e.message : 'Failed to load tokens';
      tokens = [];
    }
  }

  function toggleScope(scope: string) {
    chosenScopes = chosenScopes.includes(scope)
      ? chosenScopes.filter((s) => s !== scope)
      : [...chosenScopes, scope];
  }

  async function submitToken(event: SubmitEvent) {
    event.preventDefault();
    if (!newTokenName.trim() || chosenScopes.length === 0) return;
    creatingToken = true;
    tokensError = null;
    try {
      freshToken = await createToken(newTokenName.trim(), chosenScopes);
      newTokenName = '';
      chosenScopes = [];
      await loadTokens();
    } catch (e) {
      tokensError = e instanceof Error ? e.message : 'Failed to create the token';
    } finally {
      creatingToken = false;
    }
  }

  async function revoke(id: string) {
    revokingId = id;
    tokensError = null;
    try {
      await revokeToken(id);
      // Drop it locally rather than refetching: the list is the only thing
      // that changed, and a refetch would make the row vanish for reasons
      // unrelated to the revoke if the network is slow.
      tokens = (tokens ?? []).filter((t) => t.id !== id);
    } catch (e) {
      tokensError = e instanceof Error ? e.message : 'Failed to revoke the token';
    } finally {
      revokingId = null;
    }
  }

  function dismissNewToken() {
    freshToken = null;
  }

  async function chooseRecEngine(engine: string) {
    savingRecEngine = true;
    notice = null;
    try {
      recEngine = await patchRecEngine(engine);
      newRecEngine = recEngine.engine ?? '';
      notice = {
        message:
          recEngine.choice.state === 'honored'
            ? `Recommendations now come from ${recEngine.engine}.`
            : 'Recommendations use the instance default.',
        tone: 'success',
      };
    } catch (e) {
      notice = {
        message: e instanceof Error ? e.message : 'Could not change the engine',
        tone: 'danger',
      };
    } finally {
      savingRecEngine = false;
    }
  }

  const sourceLabel = (source: string): string => {
    switch (source) {
      case 'context':
        return 'context override';
      case 'pseud':
        return 'pseud default';
      case 'account':
        return 'account default';
      case 'instance':
        return 'instance default';
      default:
        return source;
    }
  };

</script>

<section class="settings-page">
  <header>
    <h1>Settings</h1>
    <p class="subtitle">
      Configure search defaults, content filters, and notification delivery.
    </p>
    <div class="actions">
      <Button variant="quiet" onclick={handleExport}>Export</Button>
      <label class="button button-quiet">
        Import
        <input
          type="file"
          accept=".json"
          onchange={handleImport}
          style="display: none"
        />
      </label>
    </div>
  </header>

  {#if notice}
    <div class={`notice notice-${notice.tone}`} role="status">
      {notice.message}
    </div>
  {/if}

  <Tabs tabs={TABS} value={tab} onchange={(id) => (tab = id)} />

  {#if loading}
    <Skeleton />
  {:else if error}
    <ErrorSummary {error} />
  {:else if tab === 'search'}
    <div class="tab-content">
      <div class="section-header">
        <h2>Search Defaults</h2>
        <input
          type="search"
          placeholder="Search settings…"
          bind:value={searchQuery}
          aria-label="Search settings"
        />
      </div>

      <form
        class="add-form"
        onsubmit={(e) => {
          e.preventDefault();
          handleSaveSearch();
        }}
      >
        <input
          placeholder="Key (e.g. min_words)"
          bind:value={newSearchKey}
          aria-label="Setting key"
        />
        <input
          placeholder="Value (JSON)"
          bind:value={newSearchValue}
          aria-label="Setting value"
        />
        <Button type="submit">Add</Button>
      </form>

      {#if filteredSettings.length === 0}
        <EmptyState title="No search defaults" description="Instance defaults apply." />
      {:else}
        <ul class="setting-list">
          {#each filteredSettings as setting (setting.key)}
            <li class="setting-row">
              <div class="setting-info">
                <code>{setting.key}</code>
                <span class="value">{JSON.stringify(setting.value)}</span>
                <span class="source">({sourceLabel(setting.source)})</span>
                {#if setting.summary}
                  <span class="summary">{setting.summary}</span>
                {/if}
              </div>
              {#if setting.source !== 'instance'}
                <Button variant="danger" onclick={() => handleDeleteSearch(setting.key)}>
                  Reset
                </Button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {:else if tab === 'content'}
    <div class="tab-content">
      <div class="section-header">
        <h2>Content Filters</h2>
        <p>Works matching these filters are hidden from search, feeds, and recommendations.</p>
      </div>

      <form
        class="add-form"
        onsubmit={(e) => {
          e.preventDefault();
          handleAddFilter();
        }}
      >
        <select bind:value={newFilterType} aria-label="Filter type">
          {#each FILTER_TYPES as ft}
            <option value={ft.value}>{ft.label}</option>
          {/each}
        </select>
        <input
          placeholder="Value (tag name, fandom, …)"
          bind:value={newFilterValue}
          aria-label="Filter value"
        />
        <Button type="submit">Block</Button>
      </form>

      {#if filters.length === 0}
        <EmptyState title="No content filters" description="Nothing is being filtered." />
      {:else}
        <ul class="filter-list">
          {#each filters as filter (`${filter.filter_type}:${filter.value}`)}
            <li class="filter-chip">
              <span class="filter-type">{filter.filter_type}</span>
              <span class="filter-value">{filter.value}</span>
              <button
                class="remove"
                onclick={() => handleDeleteFilter(filter)}
                aria-label="Remove filter"
              >
                ×
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {:else if tab === 'notifications'}
    <div class="tab-content">
      <div class="section-header">
        <h2>Notification Routing</h2>
        <p>Choose how you're notified for each event. Unconfigured events default to email.</p>
      </div>

      <form
        class="add-form"
        onsubmit={(e) => {
          e.preventDefault();
          handleSaveRoute();
        }}
      >
        <select bind:value={newRouteEvent} aria-label="Event type">
          <option value="">Select event…</option>
          {#each COMMON_EVENTS as ev}
            <option value={ev}>{ev}</option>
          {/each}
          <option value="custom">Custom…</option>
        </select>
        {#if newRouteEvent === 'custom'}
          <input
            placeholder="Custom event_type"
            bind:value={newRouteEvent}
            aria-label="Custom event type"
          />
        {/if}
        <select bind:value={newRouteChannel} aria-label="Channel">
          {#each CHANNELS as ch}
            <option value={ch.value}>{ch.label}</option>
          {/each}
        </select>
        <label class="toggle">
          <input type="checkbox" bind:checked={newRouteEnabled} />
          Enabled
        </label>
        <Button type="submit" disabled={!newRouteEvent || newRouteEvent === 'custom'}>Save</Button>
      </form>

      {#if routes.length === 0}
        <EmptyState title="No custom routes" description="Email is the default for all events." />
      {:else}
        <ul class="route-list">
          {#each routes as route (route.event_type)}
            <li class="route-row">
              <code>{route.event_type}</code>
              <span class="channel">{route.channel}</span>
              <span class="status" class:disabled={!route.enabled}>
                {route.enabled ? 'Enabled' : 'Disabled'}
              </span>
              <Button variant="danger" onclick={() => handleDeleteRoute(route.event_type)}>
                Reset
              </Button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {:else if tab === 'recommendations'}
    <div class="tab-content">
      <div class="section-header">
        <h2>Recommendation Engine</h2>
        <p>
          Choose which engine produces your recommendations, or leave it on the
          instance default to follow this site's configuration.
        </p>
      </div>

      {#if !recEngine}
        <EmptyState
          title="Not available"
          description="This instance could not report its recommendation engines. Everything else in your settings still works."
        />
      {:else}
        {#if recEngine.choice.state === 'unavailable'}
          <!-- The case that matters: the operator disabled the engine this
               reader chose. Say so, name both sides, and keep the stored
               choice so re-enabling restores it. -->
          <div class="notice notice-info" role="status">
            <strong>{recEngine.choice.engine}</strong> is not enabled on this
            instance any more, so your recommendations are coming from
            {recEngine.choice.using.join(', ')} instead. Your choice is
            remembered — it will take effect again if this is re-enabled.
          </div>
        {/if}

        <form
          class="add-form"
          onsubmit={(e) => {
            e.preventDefault();
            chooseRecEngine(newRecEngine);
          }}
        >
          <select bind:value={newRecEngine} aria-label="Recommendation engine" disabled={savingRecEngine}>
            <option value="">Instance default</option>
            {#each recEngine.available as name (name)}
              <option value={name}>{name}</option>
            {/each}
          </select>
          <Button type="submit" disabled={savingRecEngine || newRecEngine === (recEngine.engine ?? '')}>
            {savingRecEngine ? 'Saving…' : 'Use this engine'}
          </Button>
        </form>

        <p class="hint">
          {#if recEngine.choice.state === 'honored'}
            Your recommendations are coming from <code>{recEngine.choice.engine}</code>.
          {:else if recEngine.choice.state === 'unavailable'}
            Using <code>{recEngine.choice.using.join(', ')}</code> for now.
          {:else}
            Following this site's configuration:
            <code>{recEngine.choice.using.join(', ')}</code>.
          {/if}
        </p>
        <p class="hint">
          This choice belongs to the face you are writing as, so a second face
          can use a different engine.
        </p>
      {/if}
    </div>
  {:else if tab === 'applications'}
    <div class="tab-content">
      <div class="section-header">
        <h2>Linked applications</h2>
        <p>
          A token lets a script act as you, with only the permissions you pick.
          Revoking one takes effect immediately.
        </p>
      </div>

      {#if tokensError}
        <div class="notice notice-error" role="alert">{tokensError}</div>
      {/if}

      <!-- The one-time notice. This value is not retrievable after this
           moment, so the panel says so in as many words and the reader has to
           dismiss it deliberately. -->
      {#if freshToken}
        <div class="notice notice-info" role="status">
          <p>
            <strong>Copy this token now.</strong> It is shown once and cannot be
            shown again — there is no way to recover it, and closing this
            without copying it means creating a new one.
          </p>
          <code class="token-value">{freshToken.token}</code>
          <Button type="button" onclick={dismissNewToken}>
            I have copied it
          </Button>
        </div>
      {/if}

      <form class="add-form" onsubmit={submitToken}>
        <input
          type="text"
          placeholder="What is this for?"
          aria-label="Token name"
          bind:value={newTokenName}
          disabled={creatingToken}
        />
        <Button
          type="submit"
          disabled={creatingToken || !newTokenName.trim() || chosenScopes.length === 0}
        >
          {creatingToken ? 'Creating…' : 'Create token'}
        </Button>
      </form>

      <!-- At least one scope, or a token that can do nothing. The button is
           disabled rather than the form refusing, so the reason is visible
           where the choice is made. -->
      <fieldset class="scope-picker">
        <legend>Permissions</legend>
        {#each KNOWN_TOKEN_SCOPES as scope (scope)}
          <label class="scope-option">
            <input
              type="checkbox"
              checked={chosenScopes.includes(scope)}
              onchange={() => toggleScope(scope)}
              disabled={creatingToken}
            />
            <code>{scope}</code>
          </label>
        {/each}
      </fieldset>
      {#if chosenScopes.length === 0}
        <p class="hint">Pick at least one permission.</p>
      {/if}

      {#if tokens === null}
        <Skeleton lines={3} label="Loading linked applications" />
      {:else if tokens.length === 0}
        <EmptyState
          title="No linked applications"
          description="Tokens you create appear here, with their permissions and when they were last used."
        />
      {:else}
        <ul class="token-list">
          {#each tokens as token (token.id)}
            <li class="token-row">
              <div class="token-main">
                <span class="token-name">{token.name}</span>
                {#if token.kind === 'bot'}
                  <!-- A bot token is not one this panel made, and revoking it
                       unlinks a bot somebody else runs. Say which it is. -->
                  <span class="token-kind">bot</span>
                {/if}
                <span class="token-meta">
                  created {token.created_at.slice(0, 10)}
                  {#if token.last_used_at}
                    · last used {token.last_used_at.slice(0, 10)}
                  {:else}
                    · never used
                  {/if}
                  {#if token.expires_at}
                    · expires {token.expires_at.slice(0, 10)}
                  {/if}
                </span>
                <span class="token-scopes">
                  {#each parseTokenScopes(token.scopes) as scope (scope)}
                    <code class:unknown-scope={!isKnownTokenScope(scope)}>{scope}</code>
                  {/each}
                </span>
                {#if !token.acting_pseud_id}
                  <!-- A token with no acting pseud is refused by the API. A
                       reader looking at one should know before wiring it up. -->
                  <span class="token-warning">
                    this token has no acting pseud and will be refused
                  </span>
                {/if}
              </div>
              <Button
                type="button"
                disabled={revokingId === token.id}
                onclick={() => revoke(token.id)}
              >
                {revokingId === token.id ? 'Revoking…' : 'Revoke'}
              </Button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
</section>

<style>
  .settings-page {
    max-width: 800px;
    margin: 0 auto;
    padding: 2rem 1rem;
  }

  header {
    margin-bottom: 2rem;
  }

  h1 {
    margin: 0 0 0.5rem;
    font-size: 1.75rem;
  }

  .subtitle {
    color: var(--text-muted);
    margin: 0 0 1rem;
  }

  .actions {
    display: flex;
    gap: 0.5rem;
  }

  .button-quiet {
    display: inline-flex;
    align-items: center;
    padding: 0.4rem 0.8rem;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--surface);
    cursor: pointer;
    font-size: 0.875rem;
  }

  .notice {
    padding: 0.75rem 1rem;
    border-radius: 4px;
    margin-bottom: 1rem;
    font-size: 0.875rem;
  }

  .notice-success {
    background: var(--success-bg);
    color: var(--success-fg);
  }

  .notice-danger {
    background: var(--danger-bg);
    color: var(--danger-fg);
  }

  .notice-info {
    background: var(--info-bg);
    color: var(--info-fg);
  }

  .tab-content {
    padding: 1.5rem 0;
  }

  .section-header {
    margin-bottom: 1.5rem;
  }

  .section-header h2 {
    margin: 0 0 0.5rem;
  }

  .section-header p {
    color: var(--text-muted);
    font-size: 0.875rem;
    margin: 0.5rem 0 0;
  }

  /* Explanatory copy under a control, as distinct from the control's own
     label. Quiet enough to read twice, not so quiet it reads as chrome. */
  .hint {
    color: var(--text-muted);
    font-size: 0.8125rem;
    line-height: 1.5;
    margin: 0.75rem 0 0;
  }

  .hint code {
    background: var(--surface-raised, var(--surface));
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0.05rem 0.3rem;
  }

  .section-header input[type='search'] {
    margin-top: 0.75rem;
    width: 100%;
    max-width: 300px;
    padding: 0.5rem;
    border: 1px solid var(--border);
    border-radius: 4px;
  }

  .add-form {
    display: flex;
    gap: 0.5rem;
    margin-bottom: 1.5rem;
    flex-wrap: wrap;
    align-items: center;
  }

  .add-form input,
  .add-form select {
    padding: 0.4rem 0.6rem;
    border: 1px solid var(--border);
    border-radius: 4px;
    font-size: 0.875rem;
  }

  .add-form input {
    flex: 1;
    min-width: 120px;
  }

  .toggle {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.875rem;
  }

  .setting-list,
  .route-list,
  .filter-list {
    list-style: none;
    padding: 0;
    margin: 0;
  }

  .setting-row,
  .route-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.75rem;
    border: 1px solid var(--border);
    border-radius: 4px;
    margin-bottom: 0.5rem;
    gap: 1rem;
  }

  .setting-info {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
  }

  .setting-info code {
    font-weight: 600;
    background: var(--code-bg);
    padding: 0.15rem 0.4rem;
    border-radius: 3px;
  }

  .setting-info .value {
    color: var(--text-muted);
    font-family: monospace;
  }

  .setting-info .source {
    font-size: 0.75rem;
    color: var(--text-muted);
    font-style: italic;
  }

  .setting-info .summary {
    font-size: 0.75rem;
    color: var(--text-muted);
    width: 100%;
  }

  .filter-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.4rem 0.75rem;
    background: var(--surface-alt);
    border: 1px solid var(--border);
    border-radius: 999px;
    margin: 0 0.5rem 0.5rem 0;
    font-size: 0.875rem;
  }

  .filter-type {
    font-size: 0.7rem;
    text-transform: uppercase;
    color: var(--text-muted);
    letter-spacing: 0.05em;
  }

  .filter-value {
    font-weight: 500;
  }

  .filter-chip .remove {
    background: none;
    border: none;
    font-size: 1.1rem;
    cursor: pointer;
    padding: 0 0.2rem;
    color: var(--danger-fg);
    line-height: 1;
  }

  .route-row code {
    font-weight: 600;
    background: var(--code-bg);
    padding: 0.15rem 0.4rem;
    border-radius: 3px;
  }

  .route-row .channel {
    color: var(--text-muted);
    font-size: 0.875rem;
  }

  .route-row .status {
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--success-fg);
  }

  .route-row .status.disabled {
    color: var(--text-muted);
  }

  .filter-list {
    display: flex;
    flex-wrap: wrap;
  }

  .scope-picker {
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0.75rem 1rem;
    margin: 0.75rem 0;
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem 1.25rem;
  }

  .scope-picker legend {
    font-size: 0.875rem;
    font-weight: 600;
    padding: 0 0.35rem;
  }

  .scope-option {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.875rem;
    cursor: pointer;
  }

  .token-list {
    list-style: none;
    padding: 0;
    margin: 1rem 0 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .token-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.75rem;
    border: 1px solid var(--border);
    border-radius: 4px;
  }

  .token-main {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    min-width: 0;
  }

  .token-name {
    font-weight: 600;
  }

  /* A scope this build does not recognise, shown rather than hidden. It is
     usually a token issued before the scope was renamed, and a reader who
     cannot see it cannot work out why the token does less than it should. */
  .unknown-scope {
    text-decoration: underline dotted;
  }

  .token-kind {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 0.1rem 0.35rem;
    border: 1px solid var(--border);
    border-radius: 3px;
    color: var(--text-muted);
    margin-left: 0.4rem;
  }

  .token-meta {
    font-size: 0.8rem;
    color: var(--text-muted);
  }

  .token-scopes {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }

  .token-scopes code,
  .token-value {
    font-size: 0.75rem;
    background: var(--code-bg);
    padding: 0.15rem 0.4rem;
    border-radius: 3px;
  }

  /* The raw value. Wraps rather than overflowing, because a token that runs
     off the edge of a panel is a token a reader cannot reliably select. */
  .token-value {
    display: block;
    margin: 0.5rem 0;
    word-break: break-all;
    white-space: pre-wrap;
    user-select: all;
  }

  .token-warning {
    font-size: 0.8rem;
    color: var(--warning-fg, var(--text-muted));
  }
</style>
