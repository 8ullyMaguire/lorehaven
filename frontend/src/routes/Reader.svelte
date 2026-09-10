<script lang="ts">
  /**
   * The reading surface: one chapter, with position awareness (spec §9).
   *
   * Three things the milestone plan asks for are implemented here rather than
   * left to a component:
   *
   *  * **One chapter at a time.** Spec §9.2: "Long works must not require
   *    rendering every paragraph at once." Following *next chapter* is a
   *    navigation, not an append, so the DOM never grows without bound. Do not
   *    "simplify" this into rendering the whole work.
   *  * **Position is saved on scroll-end and on `visibilitychange`**, debounced,
   *    and written to `localStorage` first so a lost request still leaves a
   *    position (`reading.ts` owns that, and `PositionFlusher` owns the debounce).
   *  * **The body is server-sanitized HTML.** `{@html}` is used only for
   *    `sanitized_html`, which the *server* produced from the validated editor
   *    document (ADR 0002, spec §8.3). A client-side renderer would be a second
   *    set of escaping rules to get wrong.
   *
   * A *visitor* gets the same page: anonymous reading is a spec requirement
   * (§7), so nothing here is behind a session. Only the position is; a
   * signed-out reader keeps it in `localStorage` and it is never uploaded.
   */
  import { fetchChapter, saveProgress, type ChapterContent } from '../lib/api';
  import { PositionFlusher, readCachedPosition } from '../lib/reading';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import NotePanel from '../lib/components/NotePanel.svelte';
  import ReaderSettings from '../lib/components/ReaderSettings.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  interface Props {
    workId: string;
    chapterId: string;
  }

  let { workId, chapterId }: Props = $props();

  let chapter = $state<ChapterContent | null>(null);
  let error = $state<unknown>(null);
  let loading = $state(true);
  let showSettings = $state(false);

  /**
   * This browser's own identifier, so two devices keep two positions rather
   * than overwriting one another (spec §9.3).
   */
  const DEVICE_KEY = 'lorehaven.device-id';

  function deviceId(): string {
    try {
      const existing = localStorage.getItem(DEVICE_KEY);
      if (existing) return existing;
      const created = crypto.randomUUID();
      localStorage.setItem(DEVICE_KEY, created);
      return created;
    } catch {
      return 'unknown-device';
    }
  }

  const flusher = new PositionFlusher(async (pending) => {
    if (!session.isSignedIn) return;
    try {
      await saveProgress({
        subject_type: 'work',
        subject_id: pending.workId,
        chapter_id: pending.chapterId,
        content_revision: pending.revision,
        paragraph_anchor: pending.anchor,
        position_permille: pending.fraction,
        device_id: pending.device,
      });
    } catch {
      // Already cached locally; the next visit retries from the cache.
    }
  });

  $effect(() => {
    void load();
  });

  async function load() {
    loading = true;
    error = null;
    chapter = null;
    showSettings = false;
    try {
      chapter = await fetchChapter(workId, chapterId);
      // Restore a remembered place in this chapter, when there is one.
      const cached = readCachedPosition(workId);
      if (cached && cached.chapterId === chapterId) {
        requestAnimationFrame(() => {
          const fraction = cached.position.position_permille / 1000;
          window.scrollTo({ top: document.body.scrollHeight * fraction });
        });
      }
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  /** Where the reader is, as a permille of the whole chapter. */
  function currentFraction(): number {
    const scrollable = document.body.scrollHeight - window.innerHeight;
    if (scrollable <= 0) return 1000;
    return Math.min(1000, Math.round((window.scrollY / scrollable) * 1000));
  }

  function remember() {
    if (!chapter) return;
    flusher.schedule({
      workId,
      chapterId,
      revision: chapter.revision_id,
      anchor: null,
      fraction: currentFraction(),
      device: deviceId(),
    });
  }

  $effect(() => {
    // A scroll-end debounce: `scroll` fires continuously, so the write is
    // delayed until the reader has settled. Leaving the tab flushes it at once,
    // because a reader who closes the tab never triggers another scroll.
    let timer: ReturnType<typeof setTimeout> | null = null;
    const onScroll = () => {
      if (timer !== null) clearTimeout(timer);
      timer = setTimeout(remember, 400);
    };
    const onVisibility = () => {
      if (document.visibilityState === 'hidden') void flusher.flushNow();
    };

    window.addEventListener('scroll', onScroll, { passive: true });
    document.addEventListener('visibilitychange', onVisibility);
    window.addEventListener('pagehide', onVisibility);

    return () => {
      window.removeEventListener('scroll', onScroll);
      document.removeEventListener('visibilitychange', onVisibility);
      window.removeEventListener('pagehide', onVisibility);
      void flusher.flushNow();
    };
  });
</script>

{#if loading}
  <Skeleton lines={5} />
{:else if error}
  <h1>Not found</h1>
  <ErrorSummary error={error} />
  <p>
    <a href={`/works/${workId}`} onclick={(event) => handleLinkClick(event, `/works/${workId}`)}>
      Back to the work
    </a>
  </p>
{:else if chapter}
  <nav class="crumbs">
    <a href={`/works/${workId}`} onclick={(event) => handleLinkClick(event, `/works/${workId}`)}>
      {chapter.work.title.trim() === '' ? 'Untitled' : chapter.work.title}
    </a>
  </nav>

  <h1>{chapter.chapter.title.trim() === '' ? 'Untitled chapter' : chapter.chapter.title}</h1>

  <p class="meta">
    {chapter.word_count} words
    <!-- An estimate, and labelled as one: 200 words per minute (spec §3.2). -->
    · about {Math.max(1, Math.ceil(chapter.word_count / 200))} min read
    {#if chapter.revision_number !== null}· revision {chapter.revision_number}{/if}
    {#if chapter.editable}
      · <a href={`/write/${workId}`} onclick={(event) => handleLinkClick(event, `/write/${workId}`)}>Edit</a>
    {/if}
    · <button type="button" class="link-button" onclick={() => (showSettings = !showSettings)}>
      Reading settings
    </button>
  </p>

  {#if showSettings}
    <ReaderSettings signedIn={session.isSignedIn} />
  {/if}

  <div class="reader-body">
    <!-- Server-sanitized HTML; see the module note. -->
    <article class="prose">{@html chapter.sanitized_html}</article>

    {#if session.isSignedIn}
      <NotePanel subjectType="work" subjectId={workId} signedIn={session.isSignedIn} />
    {/if}
  </div>

  <nav class="pager" aria-label="Chapter navigation">
    {#if chapter.previous_chapter_id}
      <a
        href={`/works/${workId}/chapters/${chapter.previous_chapter_id}`}
        onclick={(event) => handleLinkClick(event, `/works/${workId}/chapters/${chapter.previous_chapter_id}`)}
      >
        ← Previous chapter
      </a>
    {/if}
    {#if chapter.next_chapter_id}
      <a
        class="next"
        href={`/works/${workId}/chapters/${chapter.next_chapter_id}`}
        onclick={(event) => handleLinkClick(event, `/works/${workId}/chapters/${chapter.next_chapter_id}`)}
      >
        Next chapter →
      </a>
    {:else}
      <!-- The end-of-work actions (spec §9.8). Bookmark is M8 and download is
           M7, so they name their milestone rather than pretending to work. -->
      <span class="end-note">You have reached the end of this work.</span>
    {/if}
  </nav>

  {#if !chapter.next_chapter_id}
    <ul class="end-actions">
      <li>
        <a href={`/works/${workId}`} onclick={(event) => handleLinkClick(event, `/works/${workId}`)}>
          Back to the chapter list
        </a>
      </li>
      <li><a href={`/works/${workId}#rate`}>Rate this work</a></li>
      <li class="planned">Bookmarks arrive in Milestone 8</li>
      <li class="planned">Downloads arrive in Milestone 7</li>
    </ul>
  {/if}
{/if}

<style>
  .reader-body {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--space-5);
  }

  @media (min-width: 62rem) {
    .reader-body {
      grid-template-columns: minmax(0, 1fr) 18rem;
      align-items: start;
    }
  }

  .prose {
    max-width: var(--reader-measure, 66ch);
    font-family: var(--font-reading);
    font-size: calc(var(--text-lg) * var(--reader-font-scale, 1));
    line-height: var(--reader-line-height, 1.75);
  }

  .prose :global(blockquote) {
    border-left: 3px solid var(--color-accent);
    margin: var(--space-4) 0;
    padding-left: var(--space-4);
    color: var(--color-muted);
  }

  .prose :global(hr) {
    border: none;
    text-align: center;
    margin: var(--space-5) 0;
  }

  .prose :global(hr)::after {
    content: '* * *';
    color: var(--color-muted);
    letter-spacing: 0.4em;
  }

  .meta,
  .crumbs {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .link-button {
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    color: var(--color-accent);
    cursor: pointer;
    text-decoration: underline;
  }

  .pager {
    display: flex;
    justify-content: space-between;
    gap: var(--space-4);
    margin-top: var(--space-6);
    padding-top: var(--space-4);
    border-top: var(--border-width) solid var(--color-border);
  }

  .next {
    margin-left: auto;
  }

  .end-note {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .end-actions {
    list-style: none;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-4);
    font-size: var(--text-sm);
  }

  .end-actions .planned {
    color: var(--color-muted);
  }
</style>
