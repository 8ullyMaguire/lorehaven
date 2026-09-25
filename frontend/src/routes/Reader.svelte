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
  import {
    apiFetch, createBookmark, fetchChapter, fetchTypography, saveProgress, type ChapterContent } from '../lib/api';
  import {
    DEFAULT_TYPOGRAPHY,
    PositionFlusher,
    applyTypography,
    readCachedPosition,
    readTypographyPrefs,
    savePosition,
    writeTypographyPrefs,
  } from '../lib/reading';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte.ts';
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
  let narrationEditionId = $state<string | null>(null);

  /**
   * The reading surface's appearance, applied as the page opens.
   *
   * The stored copy (or the defaults) goes on first, so a reader never sees
   * the text at the wrong size; the server's copy is then fetched for a
   * signed-in reader, because the settings panel promises these follow the
   * account to another device and this is what makes that true.
   */
  $effect(() => {
    applyTypography(readTypographyPrefs() ?? DEFAULT_TYPOGRAPHY, document.documentElement);
    if (!session.isSignedIn) return;
    void fetchTypography()
      .then((view) => {
        writeTypographyPrefs(view);
        applyTypography(view, document.documentElement);
      })
      .catch(() => {
        // The local copy is already applied; the server's is a refinement.
      });
  });

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

  /**
   * `savePosition` writes the local copy before it tries the server, which is
   * what makes "a lost request still leaves a position" true. A signed-out
   * reader keeps the position in `localStorage` and uploads nothing.
   */
  const flusher = new PositionFlusher(async (pending) => {
    await savePosition(pending, async (position) => {
      if (!session.isSignedIn) return;
      await saveProgress({
        subject_type: 'work',
        subject_id: position.workId,
        chapter_id: position.chapterId,
        content_revision: position.revision,
        paragraph_anchor: position.anchor,
        position_permille: position.fraction,
        device_id: position.device,
      });
    });
  });

  $effect(() => {
    void load();
  });

  async function load() {
    loading = true;
    error = null;
    chapter = null;
    showSettings = false;
    // A new chapter restarts what whole-work mode has appended: the address
    // names where the reader is, and carrying the previous chapter's tail into
    // a different starting point would draw a work out of order.
    extra = [];
    appendError = null;
    bookmarked = false;
    try {
      chapter = await fetchChapter(workId, chapterId);
      // Arriving in a chapter is itself a reading: it is recorded even if the
      // reader never scrolls, which is what puts an opened work in `/library`
      // and in the history list. The write is debounced, so the restore below
      // replaces it with the position actually landed on.
      remember();
      // Restore a remembered place in this chapter, when there is one.
      const cached = readCachedPosition(workId);
      if (cached && cached.chapterId === chapterId) {
        requestAnimationFrame(() => {
          const fraction = cached.position.position_permille / 1000;
          window.scrollTo({ top: document.body.scrollHeight * fraction });
        });
      }
      void loadNarrationEdition();
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  async function loadNarrationEdition() {
    try {
      const res = await apiFetch<{ items: Array<{ id: string; edition_kind: string; published_at: string | null }> }>(
        `/works/${workId}/editions`,
      );
      const narration = res.items.find((e) => e.edition_kind === 'narration' && e.published_at);
      narrationEditionId = narration?.id ?? null;
    } catch {
      narrationEditionId = null;
    }
  }

  /** Where the reader is, as a permille of the whole chapter. */
  function currentFraction(): number {
    const scrollable = document.body.scrollHeight - window.innerHeight;
    if (scrollable <= 0) return 1000;
    return Math.min(1000, Math.round((window.scrollY / scrollable) * 1000));
  }

  function remember() {
    // The *focused* chapter, not the one the address names: in whole-work mode
    // the reader is past that, and recording the first chapter of the work
    // would put them back at its start next time.
    if (!focused) return;
    flusher.schedule({
      workId,
      chapterId: focused.chapter.id,
      revision: focused.revision_id,
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

  // -------------------------------------------------------------------------
  // Whole-work mode (M8-02, re-scoped from M4)
  // -------------------------------------------------------------------------

  /**
   * Whether to keep reading past the end of this chapter.
   *
   * Spec §9.2 asks for a way to read a work through and, in the same breath,
   * that long works must not require rendering every paragraph at once. Those
   * two are only compatible if the mode *pages*: this renders the chapter the
   * reader asked for, and then appends the next one — and only the next one —
   * as they approach the end of what is loaded. A work of two hundred chapters
   * is two hundred fetches the reader never notices and never pays for at once.
   *
   * The choice is remembered in this browser, because a reader who prefers to
   * read through has told us so once.
   */
  const WHOLE_WORK_KEY = 'lorehaven.reader.whole-work';

  let wholeWork = $state(readWholeWork());
  /** Chapters appended after the one the address names. */
  let extra = $state<ChapterContent[]>([]);
  let appending = $state(false);
  let appendError = $state<unknown>(null);
  /** The sentinel the append watches; `null` until it is in the document. */
  let sentinel = $state<HTMLElement | null>(null);

  function readWholeWork(): boolean {
    try {
      return localStorage.getItem(WHOLE_WORK_KEY) === 'true';
    } catch {
      return false;
    }
  }

  function toggleWholeWork() {
    wholeWork = !wholeWork;
    if (!wholeWork) {
      // Turning it off drops what it appended, so the page is the chapter the
      // address names rather than a partial work whose address disagrees.
      extra = [];
      appendError = null;
    }
    try {
      localStorage.setItem(WHOLE_WORK_KEY, String(wholeWork));
    } catch {
      // A browser refusing storage is not a reason to refuse the mode.
    }
  }

  /** The chapter furthest down the page, which is the one being read. */
  let focused = $derived(extra.length > 0 ? extra[extra.length - 1] : chapter);

  /** The chapter after everything loaded, if there is one. */
  let pending = $derived(
    focused ? focused.next_chapter_id : null,
  );

  /**
   * Append the next chapter.
   *
   * Guarded so a reader who scrolls to the sentinel during a fetch does not
   * start a second one, and so a failure is reported once rather than retried
   * by the observer every time the sentinel comes into view.
   */
  async function appendNext() {
    if (!wholeWork || appending || appendError || !pending) return;
    appending = true;
    try {
      const next = await fetchChapter(workId, pending);
      extra = [...extra, next];
      // An appended chapter is one the reader is now looking at, so it counts
      // as a reading the same way arriving at one does.
      remember();
    } catch (failure) {
      appendError = failure;
    } finally {
      appending = false;
    }
  }

  $effect(() => {
    if (!wholeWork || !sentinel || appendError) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) void appendNext();
      },
      // A margin, so the next chapter is usually there before the reader
      // reaches the end — the mode should not feel like it is loading.
      { rootMargin: '600px' },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  });

  // -------------------------------------------------------------------------
  // Bookmarking the place
  // -------------------------------------------------------------------------

  let bookmarked = $state(false);
  let bookmarkError = $state<unknown>(null);

  /**
   * Save this place, privately.
   *
   * Private unless the reader says otherwise, and the body does not say
   * otherwise (spec §14.1). The position is the fraction of the chapter the
   * reader is at, so the bookmark points where they were rather than at the
   * chapter's start.
   */
  async function bookmarkHere() {
    if (!chapter) return;
    bookmarkError = null;
    try {
      await createBookmark({
        subjectType: 'work',
        subjectId: workId,
        chapterId: focused?.chapter.id ?? chapterId,
        positionPermille: currentFraction(),
        note: '',
        isPublic: false,
      });
      bookmarked = true;
    } catch (failure) {
      bookmarkError = failure;
    }
  }
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
    · <button
      type="button"
      class="link-button"
      aria-pressed={wholeWork}
      onclick={toggleWholeWork}
    >
      {wholeWork ? 'One chapter at a time' : 'Read on without stopping'}
    </button>
  </p>

  {#if showSettings}
    <ReaderSettings signedIn={session.isSignedIn} />
  {/if}

  {#if narrationEditionId}
    <div class="narration-player">
      <audio controls src={`/editions/${narrationEditionId}/audio`} preload="metadata">
        Your browser does not support audio playback.
      </audio>
      <p class="narration-note">Listen to a narrated edition (machine-generated).</p>
    </div>
  {/if}

  <div class="reader-body">
    <!-- Server-sanitized HTML; see the module note. -->
    <article class="prose">{@html chapter.sanitized_html}</article>

    {#if session.isSignedIn}
      <NotePanel subjectType="work" subjectId={workId} signedIn={session.isSignedIn} />
    {/if}
  </div>

  {#if wholeWork}
    <!-- Everything past the chapter the address names. Appended one at a time
         as the sentinel below comes into view, so the browser never lays out a
         work the reader has not read. -->
    {#each extra as appended (appended.chapter.id)}
      <hr class="chapter-break" />
      <h2 class="appended-title">
        <a
          href={`/works/${workId}/chapters/${appended.chapter.id}`}
          onclick={(event) =>
            handleLinkClick(event, `/works/${workId}/chapters/${appended.chapter.id}`)}
        >
          {appended.chapter.title.trim() === '' ? 'Untitled chapter' : appended.chapter.title}
        </a>
      </h2>
      <div class="reader-body">
        <article class="prose">{@html appended.sanitized_html}</article>
      </div>
    {/each}

    {#if appending}
      <p class="appending" role="status">Loading the next chapter…</p>
    {/if}
    {#if appendError}
      <!-- A fetch that failed is a dead end unless it says so and offers the
           way on, and the way on is the same append, tried again. -->
      <ErrorSummary error={appendError} />
      <p>
        <button type="button" class="link-button" onclick={() => { appendError = null; void appendNext(); }}>
          Try the next chapter again
        </button>
        ·
        <a
          href={`/works/${workId}/chapters/${pending}`}
          onclick={(event) => handleLinkClick(event, `/works/${workId}/chapters/${pending}`)}
        >
          Open it on its own page
        </a>
      </p>
    {/if}
    {#if pending && !appendError}
      <!-- Watched, and empty: it exists to be intersected with. -->
      <div bind:this={sentinel} class="sentinel" aria-hidden="true"></div>
    {/if}
  {/if}

  <nav class="pager" aria-label="Chapter navigation">
    <!-- In whole-work mode "next" is what the reader has already been given
         automatically, so the link steps them into the chapter's own page
         rather than pretending there is more below. The previous link still
         goes back a chapter, which is the only way to go up. -->
    {#if chapter.previous_chapter_id}
      <!--
        A `{@const}` per link, because a `let` binding is not narrowed inside an
        `onclick` closure. `chapter` is a `let`, so TypeScript rejected the read
        in the callback while allowing it in the `href` beside it.
      -->
      {@const previousHref = `/works/${workId}/chapters/${chapter.previous_chapter_id}`}
      <a href={previousHref} onclick={(event) => handleLinkClick(event, previousHref)}>
        ← Previous chapter
      </a>
    {/if}
    {#if focused?.next_chapter_id}
      {@const nextHref = `/works/${workId}/chapters/${focused.next_chapter_id}`}
      <a class="next" href={nextHref} onclick={(event) => handleLinkClick(event, nextHref)}>
        Next chapter →
      </a>
    {:else}
      <!-- The end-of-work actions (spec §9.8). Bookmark is M8 and download is
           M7, so they name their milestone rather than pretending to work. -->
      <span class="end-note">You have reached the end of this work.</span>
    {/if}
  </nav>

  {#if !focused?.next_chapter_id}
    <ul class="end-actions">
      <li>
        <a href={`/works/${workId}`} onclick={(event) => handleLinkClick(event, `/works/${workId}`)}>
          Back to the chapter list
        </a>
      </li>
      <li><a href={`/works/${workId}#rate`}>Rate this work</a></li>
      <li>
        <!-- Private unless the reader says otherwise, and this says nothing, so
             it is private. The place is where they are, not the chapter start. -->
        {#if bookmarked}
          <span role="status">Bookmarked this place.</span>
        {:else}
          <button type="button" class="link-button" onclick={() => void bookmarkHere()}>
            Bookmark this place
          </button>
        {/if}
      </li>
      <li>
        <a
          href={`/exports?${new URLSearchParams({
            subject_type: 'work',
            subject_id: workId,
            title: chapter.work.title,
          }).toString()}`}
        >
          Download this work
        </a>
      </li>
    </ul>
    {#if bookmarkError}
      <ErrorSummary error={bookmarkError} />
    {/if}
  {/if}
{/if}

<style>
  .chapter-break {
    border: 0;
    border-top: var(--border-width) solid var(--color-border);
    margin: var(--space-5) 0;
  }

  .appended-title {
    font-size: var(--text-xl);
    margin: var(--space-5) 0 var(--space-2);
  }

  .sentinel {
    height: 1px;
  }

  .appending {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .reader-body {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--space-5);
    /*
     * The reading surface carries the reader's own preset, not the site's
     * theme: spec §9 and THEME.md both require "how the page I am reading
     * looks" to be independent of "how the site looks". The fallbacks keep a
     * page readable if no preset has been resolved yet.
     */
    margin-top: var(--space-3);
    padding: var(--space-4) var(--space-4) var(--space-5);
    border-radius: var(--radius-md);
    background: var(--reader-bg, transparent);
    color: var(--reader-text, inherit);
  }

  .reader-body :global(a) {
    color: var(--reader-link, var(--color-accent));
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
    border-left: 3px solid var(--reader-rule, var(--color-accent));
    margin: var(--space-4) 0;
    padding-left: var(--space-4);
    color: var(--reader-muted, var(--color-muted));
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
    border-top: 1px solid var(--color-border);
  }

  .narration-player {
    margin: var(--space-4) 0;
    padding: var(--space-3);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 8px;
  }

  .narration-player audio {
    width: 100%;
  }

  .narration-note {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
    color: var(--color-muted);
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

</style>
