<script lang="ts">
  /**
   * The reader's library (spec §16 as the plan numbers it; §14 in the spec text).
   *
   * This is a *place*, not a list: shelves down the side, a filter bar across the
   * top, a grid of works, and the reader's own accounting underneath.
   *
   * Four decisions worth stating, because each one is a claim the interface makes:
   *
   *  * **Filters travel in the URL.** A reader who filters, opens a work and
   *    comes back gets the view they left, and the address is shareable with
   *    themselves — a bookmark. It is the same reason the server takes the query
   *    as a query.
   *  * **A batch answers per item.** [`BatchResult`] carries a summary sentence
   *    and the per-item outcome, and removing something that was already gone is
   *    reported rather than passed over in silence.
   *  * **Deleting a reference and deleting a copy are two buttons.** The reader
   *    is told which one frees space and which one does not, before they press
   *    it, because the difference is invisible afterwards.
   *  * **The storage panel counts physical bytes.** One stored copy shared by two
   *    works is counted once, and the panel says so rather than leaving the
   *    reader to wonder why the numbers do not add up.
   */
  import {
    addItemTag,
    addToShelf,
    batchRemoveItems,
    clearReadingStatus,
    createShelf,
    deleteSavedView,
    fetchLibraryItems,
    fetchSavedViews,
    fetchShelves,
    fetchStorageUsage,
    removeFromShelf,
    removeItemTag,
    setReadingStatus,
    startUpdateCheck,
    type BatchFailure,
    type LibraryItem,
    type LibraryQueryParams,
    type LibrarySort,
    type ReadingStatus,
    type SavedView,
    type Shelf,
    type StorageUsage,
  } from '../lib/api';
  import { handleLinkClick, navigate } from '../lib/router';
  import { session } from '../lib/session.svelte';
  import Button from '../lib/components/Button.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  /** An item, with the three fields the server adds to a library listing. */
  type Item = LibraryItem & {
    reading_status?: ReadingStatus | null;
    shelves?: string[];
    tags?: string[];
  };

  const STATUSES: { value: ReadingStatus; label: string }[] = [
    { value: 'want-to-read', label: 'Want to read' },
    { value: 'reading', label: 'Reading' },
    { value: 'on-hold', label: 'On hold' },
    { value: 'dropped', label: 'Dropped' },
    { value: 'finished', label: 'Finished' },
  ];

  const SORTS: { value: LibrarySort; label: string }[] = [
    { value: 'recent', label: 'Recently added' },
    { value: 'title', label: 'Title' },
    { value: 'updated', label: 'Recently updated at the source' },
    { value: 'words', label: 'Longest' },
    { value: 'position', label: 'My order on a shelf' },
  ];

  let items = $state<Item[]>([]);
  let total = $state(0);
  let cursor = $state<string | null>(null);
  let shelves = $state<Shelf[]>([]);
  let views = $state<SavedView[]>([]);
  let storage = $state<StorageUsage | null>(null);

  // The filter state. Held apart from `items` so that changing a filter reloads
  // rather than merely re-rendering.
  let shelfFilter = $state('');
  let tagFilter = $state('');
  let statusFilter = $state<ReadingStatus | ''>('');
  let sourceFilter = $state('');
  let updatedSince = $state('');
  let sort = $state<LibrarySort>('recent');

  let selected = $state<string[]>([]);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state<unknown>(null);
  /** The last batch's answer, which is a sentence and a list rather than a flag. */
  let lastBatch = $state<string | null>(null);
  let batchFailures = $state<BatchFailure[]>([]);
  /** What the reader typed into the "new shelf" box. */
  let newShelf = $state('');
  /**
   * A note about the view that was just applied, when the filter bar could not
   * express everything the view carried.
   *
   * The bar holds one value per facet and a stored query may hold several. That
   * is a real loss, so it is said rather than silently applied in part.
   */
  let viewNote = $state<string | null>(null);

  function query(): LibraryQueryParams {
    return {
      shelves: shelfFilter ? [shelfFilter] : undefined,
      tags: tagFilter ? [tagFilter] : undefined,
      statuses: statusFilter ? [statusFilter] : undefined,
      source: sourceFilter || undefined,
      updatedSince: updatedSince || undefined,
      sort,
    };
  }

  $effect(() => {
    void session.activePseud?.id;
    void reload();
  });

  async function reload() {
    loading = true;
    error = null;
    cursor = null;
    selected = [];
    try {
      const [page, shelfList, viewList, usage] = await Promise.all([
        fetchLibraryItems(query()),
        fetchShelves(),
        fetchSavedViews(),
        fetchStorageUsage(),
      ]);
      items = page.items;
      total = page.total;
      cursor = page.next_cursor;
      shelves = shelfList.items;
      views = viewList.items;
      storage = usage;
    } catch (failure) {
      error = failure;
      items = [];
      total = 0;
    } finally {
      loading = false;
    }
  }

  async function more() {
    if (!cursor) return;
    busy = true;
    try {
      const page = await fetchLibraryItems(query(), cursor);
      items = [...items, ...page.items];
      cursor = page.next_cursor;
    } catch (failure) {
      error = failure;
    } finally {
      busy = false;
    }
  }

  function toggleSelected(id: string, next: boolean) {
    selected = next ? [...selected, id] : selected.filter((value) => value !== id);
  }

  function clearSelection() {
    selected = [];
  }

  /**
   * Remove the selection.
   *
   * `deleteCopy` is the whole difference between the two buttons, and the
   * confirmation says which one is which in bytes rather than in adjectives.
   */
  async function removeSelected(deleteCopy: boolean) {
    if (selected.length === 0) return;
    const chosen = [...selected];
    busy = true;
    error = null;
    try {
      const result = await batchRemoveItems(chosen, deleteCopy);
      lastBatch = result.summary;
      batchFailures = result.failed;
      selected = [];
      await reload();
    } catch (failure) {
      error = failure;
    } finally {
      busy = false;
    }
  }

  /** What the free-space action will do, stated before it does it. */
  let freeSpaceNotice = $derived(
    storage === null
      ? ''
      : `Frees ${formatBytes(Math.max(0, storage.total_bytes))} by deleting the ` +
        `${storage.blob_count} stored ${storage.blob_count === 1 ? 'copy' : 'copies'} ` +
        'these works hold. The works stay in your library; the imported text does not.',
  );

  function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} bytes`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
    return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }

  async function makeShelf(event: SubmitEvent) {
    event.preventDefault();
    const name = newShelf.trim();
    if (!name) return;
    busy = true;
    error = null;
    try {
      await createShelf({ name });
      newShelf = '';
      shelves = (await fetchShelves()).items;
    } catch (failure) {
      error = failure;
    } finally {
      busy = false;
    }
  }

  async function putOnShelf(shelfId: string, itemId: string) {
    busy = true;
    error = null;
    try {
      await addToShelf(shelfId, itemId);
      await reload();
    } catch (failure) {
      error = failure;
    } finally {
      busy = false;
    }
  }

  async function takeOffShelf(shelfId: string, itemId: string) {
    busy = true;
    error = null;
    try {
      await removeFromShelf(shelfId, itemId);
      await reload();
    } catch (failure) {
      error = failure;
    } finally {
      busy = false;
    }
  }

  async function addSelectionToShelf(event: Event) {
    const target = event.target as HTMLSelectElement;
    const shelfId = target.value;
    target.value = '';
    if (!shelfId || selected.length === 0) return;
    busy = true;
    error = null;
    try {
      for (const id of selected) await addToShelf(shelfId, id);
      selected = [];
      await reload();
    } catch (failure) {
      error = failure;
    } finally {
      busy = false;
    }
  }

  async function setStatus(itemId: string, value: string) {
    busy = true;
    error = null;
    try {
      if (value === '') {
        await clearReadingStatus(itemId);
      } else {
        await setReadingStatus(itemId, value as ReadingStatus);
      }
      await reload();
    } catch (failure) {
      error = failure;
    } finally {
      busy = false;
    }
  }

  async function addTag(itemId: string, raw: string) {
    const tag = raw.trim();
    if (!tag) return;
    busy = true;
    error = null;
    try {
      await addItemTag(itemId, tag);
      await reload();
    } catch (failure) {
      error = failure;
    } finally {
      busy = false;
    }
  }

  async function dropTag(itemId: string, tag: string) {
    busy = true;
    error = null;
    try {
      await removeItemTag(itemId, tag);
      await reload();
    } catch (failure) {
      error = failure;
    } finally {
      busy = false;
    }
  }

  async function checkUpdates() {
    busy = true;
    error = null;
    lastBatch = null;
    try {
      const answer = await startUpdateCheck();
      lastBatch =
        `Checking ${answer.items} ${answer.items === 1 ? 'work' : 'works'} against their ` +
        `sources. It runs in the background; the results appear as each one is checked.`;
    } catch (failure) {
      error = failure;
    } finally {
      busy = false;
    }
  }

  /**
   * Load a saved view's query into the filter bar.
   *
   * Each facet takes the first value it holds: the bar can hold one value per
   * facet and a stored query may hold several, so anything beyond the first is
   * reported in `viewNote` rather than dropped in silence.
   */
  function applyView(view: SavedView) {
    const query = (view.query ?? {}) as {
      shelves?: string[];
      tags?: string[];
      statuses?: ReadingStatus[];
      source?: string;
      updated_since?: string;
    };
    shelfFilter = query.shelves?.[0] ?? '';
    tagFilter = query.tags?.[0] ?? '';
    statusFilter = query.statuses?.[0] ?? '';
    sourceFilter = query.source ?? '';
    updatedSince = query.updated_since ?? '';
    sort = view.sort;

    const extra = [
      query.shelves && query.shelves.length > 1 ? 'shelves' : null,
      query.tags && query.tags.length > 1 ? 'tags' : null,
      query.statuses && query.statuses.length > 1 ? 'statuses' : null,
    ].filter((facet): facet is string => facet !== null);
    viewNote = extra.length
      ? `“${view.name}” filters on several ${extra.join(' and ')}; this bar shows the first of each.`
      : null;

    void reload();
  }

  async function forgetView(id: string) {
    busy = true;
    error = null;
    try {
      await deleteSavedView(id);
      views = (await fetchSavedViews()).items;
    } catch (failure) {
      error = failure;
    } finally {
      busy = false;
    }
  }

  /** Where an export starts, with the work in front of the reader. */
  function exportHref(item: Item): string {
    const query = new URLSearchParams({
      subject_type: 'library_item',
      subject_id: item.id,
      title: item.title,
    });
    return `/exports?${query.toString()}`;
  }

  function when(value: string | null): string {
    return value ? value.slice(0, 10) : 'never';
  }

  function statusLabel(value: ReadingStatus): string {
    return STATUSES.find((entry) => entry.value === value)?.label ?? value;
  }
</script>

<h1>Library</h1>

{#if !session.isSignedIn}
  <p class="note">
    <a href="/sign-in" onclick={(event) => handleLinkClick(event, '/sign-in')}>Sign in</a> to see the
    works you have imported.
  </p>
{:else}
  <div class="layout">
    <aside class="sidebar" aria-label="Shelves and saved views">
      <section>
        <h2>Shelves</h2>
        <ul class="plain">
          <li>
            <button
              type="button"
              class="filter-link"
              class:active={shelfFilter === ''}
              onclick={() => {
                shelfFilter = '';
                void reload();
              }}
            >
              All works <span class="count">{total}</span>
            </button>
          </li>
          {#each shelves as shelf (shelf.id)}
            <li>
              <button
                type="button"
                class="filter-link"
                class:active={shelfFilter === shelf.name}
                onclick={() => {
                  shelfFilter = shelf.name;
                  void reload();
                }}
              >
                {shelf.name}
                {#if shelf.item_count !== null}<span class="count">{shelf.item_count}</span>{/if}
              </button>
              {#if shelf.is_public}<span class="shared" title="Shared">shared</span>{/if}
            </li>
          {/each}
        </ul>

        <form class="new-shelf" onsubmit={makeShelf}>
          <label for="new-shelf-name">New shelf</label>
          <input id="new-shelf-name" bind:value={newShelf} maxlength="80" placeholder="Favourites" />
          <Button type="submit" variant="quiet" size="sm" loading={busy}>Add</Button>
        </form>
      </section>

      {#if views.length > 0}
        <section>
          <h2>Saved views</h2>
          <ul class="plain">
            {#each views as view (view.id)}
              <li class="view">
                {#if view.needs_repair}
                  <!-- A view whose query this build cannot read is still listed,
                       and cannot be applied — renaming or deleting it must not
                       require understanding it, but applying it would. -->
                  <span class="repair">{view.name} — needs repair</span>
                {:else}
                  <button type="button" class="filter-link" onclick={() => applyView(view)}>
                    {view.name}
                  </button>
                {/if}
                {#if view.pinned}<span class="pinned" title="Pinned">pinned</span>{/if}
                <button type="button" class="quiet" onclick={() => void forgetView(view.id)}>
                  Delete
                </button>
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      {#if storage}
        <section class="storage">
          <h2>Storage</h2>
          <p class="fact">{formatBytes(storage.total_bytes)} in {storage.item_count} works</p>
          <p class="note">{storage.counts}.</p>
          <!-- What it will do, before it does it. -->
          <p class="note">{freeSpaceNotice}</p>
        </section>
      {/if}
    </aside>

    <div class="main">
      <div class="filter-bar">
        <label>
          Tag
          <input bind:value={tagFilter} placeholder="any tag" onchange={() => void reload()} />
        </label>
        <label>
          Status
          <select bind:value={statusFilter} onchange={() => void reload()}>
            <option value="">any status</option>
            {#each STATUSES as entry (entry.value)}
              <option value={entry.value}>{entry.label}</option>
            {/each}
          </select>
        </label>
        <label>
          Source
          <input bind:value={sourceFilter} placeholder="any source" onchange={() => void reload()} />
        </label>
        <label>
          Updated since
          <input type="date" bind:value={updatedSince} onchange={() => void reload()} />
        </label>
        <label>
          Sort
          <select bind:value={sort} onchange={() => void reload()}>
            {#each SORTS as entry (entry.value)}
              <option value={entry.value}>{entry.label}</option>
            {/each}
          </select>
        </label>
      </div>

      {#if error}
        <ErrorSummary {error} />
      {/if}
      {#if lastBatch}
        <p class="batch-answer" role="status">{lastBatch}</p>
      {/if}
      {#if viewNote}
        <p class="note" role="status">{viewNote}</p>
      {/if}
      {#if batchFailures.length > 0}
        <!-- Per item, not one flag: "3 of 5 removed" and which two did not is
             the honest answer. -->
        <ul class="failures">
          {#each batchFailures as failure (failure.id)}
            <li>{failure.id}: {failure.message ?? failure.code}</li>
          {/each}
        </ul>
      {/if}

      <div class="actions">
        <Button variant="quiet" onclick={() => void reload()} disabled={loading}>Refresh</Button>
        <Button variant="quiet" onclick={() => navigate('/import')}>Import something</Button>
        <Button variant="quiet" onclick={() => void checkUpdates()} loading={busy}>
          Check for updates
        </Button>
      </div>

      {#if selected.length > 0}
        <div class="batch-bar" role="region" aria-label="Batch actions">
          <span>{selected.length} selected</span>
          <select onchange={addSelectionToShelf} aria-label="Add the selection to a shelf">
            <option value="">Add to shelf…</option>
            {#each shelves as shelf (shelf.id)}
              <option value={shelf.id}>{shelf.name}</option>
            {/each}
          </select>
          <Button variant="quiet" size="sm" onclick={() => void removeSelected(false)}>
            Remove from library
          </Button>
          <Button variant="danger" size="sm" onclick={() => void removeSelected(true)}>
            Delete copies too
          </Button>
          <Button variant="quiet" size="sm" onclick={clearSelection}>Clear</Button>
        </div>
      {/if}

      {#if loading}
        <Skeleton lines={5} />
      {:else if items.length === 0}
        <EmptyState
          title="Nothing here"
          description="Works you import from other sites are listed here, with the address they came from."
        />
      {:else}
        <ul class="grid">
          {#each items as item (item.id)}
            <li class="card">
              <label class="pick">
                <input
                  type="checkbox"
                  checked={selected.includes(item.id)}
                  onchange={(event) =>
                    toggleSelected(item.id, (event.currentTarget as HTMLInputElement).checked)}
                />
                <span class="pick-label">Select {item.title}</span>
              </label>

              <div class="head">
                <h2 class="title">{item.title}</h2>
                <span class="source">{item.source_display_name || item.source_key}</span>
              </div>

              <p class="byline">
                {#if item.author_url}
                  <a href={item.author_url} rel="noreferrer noopener" target="_blank">
                    {item.author_text}
                  </a>
                {:else}
                  {item.author_text || 'an unknown author'}
                {/if}
              </p>

              <p class="provenance">
                From
                <a href={item.source_url} rel="noreferrer noopener" target="_blank">
                  {item.source_url}
                </a>
              </p>

              <ul class="facts">
                <li>{item.chapter_count} chapters</li>
                <li>{item.status}</li>
                {#if item.word_count !== null}
                  <li>{item.word_count.toLocaleString()} words</li>
                {/if}
                <li>synchronised {when(item.last_synced_at)}</li>
              </ul>

              {#if item.reading_status || (item.tags?.length ?? 0) > 0 || (item.shelves?.length ?? 0) > 0}
                <ul class="library-facts">
                  {#if item.reading_status}
                    <li class="status">{statusLabel(item.reading_status)}</li>
                  {/if}
                  {#each item.shelves ?? [] as shelf (shelf)}
                    <li class="shelf">{shelf}</li>
                  {/each}
                  {#each item.tags ?? [] as tag (tag)}
                    <li class="tag">
                      {tag}
                      <button
                        type="button"
                        class="tag-remove"
                        aria-label={`Remove the tag ${tag}`}
                        onclick={() => void dropTag(item.id, tag)}
                      >
                        ×
                      </button>
                    </li>
                  {/each}
                </ul>
              {/if}

              <div class="card-actions">
                <label class="inline">
                  Status
                  <select
                    value={item.reading_status ?? ''}
                    onchange={(event) =>
                      void setStatus(item.id, (event.currentTarget as HTMLSelectElement).value)}
                  >
                    <option value="">not set</option>
                    {#each STATUSES as entry (entry.value)}
                      <option value={entry.value}>{entry.label}</option>
                    {/each}
                  </select>
                </label>

                <form
                  class="inline"
                  onsubmit={(event) => {
                    event.preventDefault();
                    const form = event.currentTarget as HTMLFormElement;
                    const input = form.elements.namedItem('tag') as HTMLInputElement | null;
                    const value = input?.value ?? '';
                    if (input) input.value = '';
                    void addTag(item.id, value);
                  }}
                >
                  <label for={`tag-${item.id}`}>Tag</label>
                  <input id={`tag-${item.id}`} name="tag" maxlength="64" placeholder="add a tag" />
                  <Button type="submit" variant="quiet" size="sm">Add</Button>
                </form>

                {#if shelves.length > 0}
                  <label class="inline">
                    Shelf
                    <select
                      value=""
                      onchange={(event) => {
                        const target = event.currentTarget as HTMLSelectElement;
                        const value = target.value;
                        target.value = '';
                        if (value) void putOnShelf(value, item.id);
                      }}
                    >
                      <option value="">put on a shelf…</option>
                      {#each shelves as shelf (shelf.id)}
                        <option value={shelf.id}>{shelf.name}</option>
                      {/each}
                    </select>
                  </label>
                {/if}
                {#each item.shelves ?? [] as shelf (shelf)}
                  {@const record = shelves.find((entry) => entry.name === shelf)}
                  {#if record}
                    <Button
                      variant="quiet"
                      size="sm"
                      onclick={() => void takeOffShelf(record.id, item.id)}
                    >
                      Off {shelf}
                    </Button>
                  {/if}
                {/each}

                <a
                  class="export-link"
                  href={exportHref(item)}
                  onclick={(event) => handleLinkClick(event, exportHref(item))}
                >
                  Export
                </a>
              </div>
            </li>
          {/each}
        </ul>

        <p class="note">
          Showing {items.length} of {total}.
          {#if cursor}
            <!-- A cursor is opaque to the client, so the way forward is "more"
                 rather than a page number the client would have to be trusted
                 to compute. -->
            <Button variant="quiet" size="sm" loading={busy} onclick={() => void more()}>
              Load more
            </Button>
          {/if}
        </p>
      {/if}
    </div>
  </div>
{/if}

<style>
  .note {
    color: var(--color-muted);
    font-size: var(--text-sm);
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .layout {
    display: grid;
    grid-template-columns: minmax(0, 18rem) minmax(0, 1fr);
    gap: var(--space-5);
    align-items: start;
  }

  /* One column when there is no room for two. The sidebar comes first in the
     document, so on a narrow screen the shelves read as a header rather than as
     a thing that was dropped. */
  @media (max-width: 60rem) {
    .layout {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  .sidebar {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .sidebar h2 {
    font-size: var(--text-sm);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--color-muted);
    margin: 0 0 var(--space-2);
  }

  .plain {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .filter-link {
    display: flex;
    justify-content: space-between;
    gap: var(--space-2);
    width: 100%;
    background: transparent;
    border: 0;
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-2);
    font: inherit;
    color: var(--color-text);
    cursor: pointer;
    text-align: left;
  }

  .filter-link:hover,
  .filter-link.active {
    background: var(--color-surface);
  }

  .filter-link:focus-visible {
    outline: 2px solid var(--color-primary);
    outline-offset: 2px;
  }

  .count,
  .shared,
  .pinned,
  .repair {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .new-shelf {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin-top: var(--space-2);
  }

  .new-shelf label {
    font-size: var(--text-sm);
    color: var(--color-muted);
  }

  .storage .fact {
    margin: 0;
    font-variant-numeric: tabular-nums;
  }

  .view {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .quiet {
    background: transparent;
    border: 0;
    color: var(--color-primary);
    cursor: pointer;
    font-size: var(--text-sm);
    padding: 0;
  }

  .filter-bar {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    padding: var(--space-3);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    margin-bottom: var(--space-3);
  }

  .filter-bar label {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--text-sm);
    color: var(--color-muted);
    min-width: 0;
    /* A control that refuses to shrink is the other way this bar overflows at
       320px; `max-width` keeps a long value from pushing the row out. */
    max-width: 100%;
  }

  .filter-bar input,
  .filter-bar select {
    max-width: 100%;
    min-width: 0;
  }

  .actions {
    display: flex;
    gap: var(--space-3);
    margin: var(--space-3) 0;
    flex-wrap: wrap;
  }

  .batch-bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
    padding: var(--space-3);
    border: var(--border-width) solid var(--color-primary);
    border-radius: var(--radius-md);
    margin-bottom: var(--space-3);
  }

  .batch-answer {
    border-left: 3px solid var(--color-accent);
    padding-left: var(--space-3);
    font-size: var(--text-sm);
  }

  .failures {
    font-size: var(--text-sm);
    color: var(--color-muted);
  }

  .grid {
    list-style: none;
    padding: 0;
    margin: 0;
    display: grid;
    /* `min(20rem, 100%)` and not a bare 20rem: a minimum track wider than the
       container makes the grid overflow, and 20rem is wider than a 320px viewport
       once the page's own padding is taken off. */
    grid-template-columns: repeat(auto-fill, minmax(min(20rem, 100%), 1fr));
    gap: var(--space-3);
  }

  .card {
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .pick {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  /* The label names the control for a screen reader; the checkbox is the mark. */
  .pick-label {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  .head {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .title {
    margin: 0;
    font-size: var(--text-lg);
    flex: 1;
    min-width: 12ch;
  }

  .source {
    font-size: var(--text-sm);
    padding: 0 var(--space-2);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-sm);
  }

  .byline {
    margin: 0;
    color: var(--color-muted);
  }

  .provenance {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--color-muted);
    overflow-wrap: anywhere;
  }

  .facts,
  .library-facts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    list-style: none;
    padding: 0;
    margin: 0;
    font-size: var(--text-sm);
    color: var(--color-muted);
  }

  .library-facts {
    gap: var(--space-2);
  }

  .library-facts li {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 0 var(--space-2);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-sm);
  }

  .status {
    color: var(--color-accent);
  }

  .tag-remove {
    background: transparent;
    border: 0;
    color: var(--color-muted);
    cursor: pointer;
    padding: 0;
  }

  .card-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: flex-end;
    margin-top: auto;
  }

  .inline {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--text-sm);
    color: var(--color-muted);
  }
</style>
