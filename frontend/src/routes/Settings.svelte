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
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to load settings';
    } finally {
      loading = false;
    }
  }

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
</style>
