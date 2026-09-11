<script lang="ts">
  /**
   * The reader's library of imported works (spec §11.3, §11.4).
   *
   * What is here is the part Milestone 6 owns: the imported works and where each
   * one came from. A work's provenance is not decoration — for an imported copy
   * the source address is the only way back to the original, and the only way to
   * ask whether it has changed. So every row carries its source, its source
   * address, when it was last synchronised, and one action: check for updates.
   *
   * Two deliberate choices:
   *
   *  * **Checking for updates is a preview, not a fetch.** It asks the source
   *    what it holds and the server what it would do with it, and reports that.
   *    Nothing is stored until the reader confirms on the import page, which is
   *    the same consent the import itself required.
   *  * **Shelves, private tags and storage accounting are not here.** They are
   *    Milestone 8, and a placeholder that looked like them would be a claim
   *    this build cannot keep (spec §1.1).
   */
  import { fetchLibraryItems, previewImport, type LibraryItem, type PreviewView } from '../lib/api';
  import { handleLinkClick, navigate } from '../lib/router';
  import { session } from '../lib/session.svelte';
  import Button from '../lib/components/Button.svelte';
  import ClampedText from '../lib/components/ClampedText.svelte';
import EmptyState from '../lib/components/EmptyState.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  let items = $state<LibraryItem[]>([]);

  function chapters(count: number): string {
    return `${count} ${count === 1 ? 'chapter' : 'chapters'}`;
  }
  let loading = $state(true);
  let error = $state<unknown>(null);
  let busy = $state<string | null>(null);
  /** The last update answer, keyed by library item id. */
  let answers = $state<Record<string, string>>({});
  let failures = $state<Record<string, unknown>>({});

  $effect(() => {
    void session.activePseud?.id;
    void load();
  });

  async function load() {
    loading = true;
    error = null;
    try {
      const view = await fetchLibraryItems();
      items = view.items;
    } catch (failure) {
      error = failure;
      items = [];
    } finally {
      loading = false;
    }
  }

  async function checkForUpdates(item: LibraryItem) {
    busy = item.id;
    const nextAnswers = { ...answers };
    delete nextAnswers[item.id];
    answers = nextAnswers;
    const nextFailures = { ...failures };
    delete nextFailures[item.id];
    failures = nextFailures;

    try {
      const preview: PreviewView = await previewImport(item.source_url, session.activePseud?.id);
      answers = { ...answers, [item.id]: describe(preview) };
    } catch (failure) {
      failures = { ...failures, [item.id]: failure };
    } finally {
      busy = null;
    }
  }

  /**
   * What the source's answer means, in one sentence.
   *
   * `no_change` is stated plainly rather than left blank: a reader who asked
   * whether there is anything new has been answered, and silence reads like a
   * failure.
   */
  function describe(preview: PreviewView): string {
    const plan = preview.plan;
    if (plan.plan === 'no_change') return 'No changes at the source.';
    if (plan.added > 0 && plan.removed === 0 && plan.reordered === 0 && plan.retitled === 0) {
      return `${plan.added} new ${plan.added === 1 ? 'chapter' : 'chapters'} at the source.`;
    }
    const parts: string[] = [];
    if (plan.added) parts.push(`${plan.added} added`);
    if (plan.removed) parts.push(`${plan.removed} removed`);
    if (plan.reordered) parts.push(`${plan.reordered} reordered`);
    if (plan.retitled) parts.push(`${plan.retitled} retitled`);
    return parts.length
      ? `The source has changed: ${parts.join(', ')}.`
      : 'The source has changed.';
  }

  function when(value: string | null): string {
    return value ? value.slice(0, 10) : 'never';
  }

  /**
   * Where an export starts.
   *
   * The subject travels in the query string rather than in the route, and the
   * *same* string is used for the link and for the click handler: passing
   * `/exports` to the handler and the full path to the browser would send a
   * modified click to the export page with a subject and a plain left click to
   * the same page without one.
   */
  function exportHref(item: LibraryItem): string {
    const query = new URLSearchParams({
      subject_type: 'library_item',
      subject_id: item.id,
      title: item.title,
    });
    return `/exports?${query.toString()}`;
  }

  function words(count: number | null): string {
    return count === null ? 'unknown length' : `${count.toLocaleString()} words`;
  }
</script>

<h1>Library</h1>

{#if !session.isSignedIn}
  <p class="note">
    <a href="/sign-in" onclick={(event) => handleLinkClick(event, '/sign-in')}>Sign in</a> to see the
    works you have imported.
  </p>
{:else if loading}
  <Skeleton lines={4} />
{:else}
  <p class="note">
    Works you have imported, with the site each one came from. Your library is private.
  </p>

  {#if error}
    <ErrorSummary error={error} />
  {/if}

  <div class="actions">
    <Button variant="quiet" onclick={() => void load()} disabled={loading}>Refresh</Button>
    <Button variant="quiet" onclick={() => navigate('/import')}>Import something</Button>
  </div>

  {#if items.length === 0}
    <EmptyState
      title="Nothing imported yet"
      description="Works you import from other sites are listed here, with the address they came from."
    />
  {:else}
    <ul class="items">
      {#each items as item (item.id)}
        <li class="item">
          <div class="head">
            <h2 class="title">{item.title}</h2>
            <span class="source">{item.source_display_name || item.source_key}</span>
          </div>

          <p class="byline">
            {#if item.author_url}
              <a href={item.author_url} rel="noreferrer noopener" target="_blank"
                >{item.author_text}</a
              >
            {:else}
              {item.author_text || 'an unknown author'}
            {/if}
          </p>

          <ul class="facts">
            <li>{chapters(item.chapter_count)}</li>
            <li>{item.status}</li>
            <li>{words(item.word_count)}</li>
            {#if item.language}<li>{item.language}</li>{/if}
            <li>last synchronised {when(item.last_synced_at)}</li>
            {#if item.source_updated_at}
              <li>source updated {when(item.source_updated_at)}</li>
            {/if}
          </ul>

          {#if item.summary}
            <ClampedText text={item.summary} />
          {/if}

          <p class="meta break">
            From
            <a href={item.source_url} rel="noreferrer noopener" target="_blank">{item.source_url}</a>
          </p>

          <div class="actions">
            <Button
              variant="quiet"
              size="sm"
              onclick={() => checkForUpdates(item)}
              disabled={busy === item.id}
              aria-label={`Check ${item.source_url} for changes`}
            >
              {busy === item.id ? 'Checking…' : 'Check for updates'}
            </Button>
            <!-- Where an export starts: with the work in front of the reader.
                 The format and the privacy notice are chosen on the exports
                 page, which is the only place the notice is shown. -->
            <a
              class="export-link"
              href={exportHref(item)}
              onclick={(event) => handleLinkClick(event, exportHref(item))}
            >
              Export
            </a>
          </div>

          {#if answers[item.id]}
            <p class="answer" role="status">{answers[item.id]}</p>
          {/if}
          {#if failures[item.id]}
            <ErrorSummary error={failures[item.id]} />
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

  .break {
    overflow-wrap: anywhere;
  }

  .actions {
    display: flex;
    gap: var(--space-3);
    margin: var(--space-3) 0;
    flex-wrap: wrap;
  }

  .items {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .item {
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    padding: var(--space-3);
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
    /* No `text-transform: lowercase` here. It was right while this badge held a
       machine key — `royalroad` reads the same either way — and it is wrong now
       that it holds the source's name, because the transform renders
       "Royal Road" as "royal road" and a proper noun is not ours to recase. */
    padding: 0 var(--space-2);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-sm);
  }

  .byline {
    margin: var(--space-1) 0;
    color: var(--color-muted);
  }

  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    list-style: none;
    padding: 0;
    margin: var(--space-2) 0;
    font-size: var(--text-sm);
    color: var(--color-muted);
  }



  .answer {
    border-left: 3px solid var(--color-accent);
    padding-left: var(--space-3);
    font-size: var(--text-sm);
  }
</style>
