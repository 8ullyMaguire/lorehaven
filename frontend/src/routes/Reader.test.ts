import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Reader from './Reader.svelte';
import { session } from '../lib/session.svelte';
import { TYPOGRAPHY_STORAGE_KEY } from '../lib/reading';

/**
 * The reading surface's two invisible jobs, both of which were found by
 * driving the site in a browser rather than by reading the code:
 *
 *  * **Arriving in a chapter is a reading.** The reader reported a position
 *    only on scroll, and nothing at all recorded that a work had been opened,
 *    so `/library/history` was empty for every real reader.
 *  * **The reader's appearance is applied on load.** The stored copy went on
 *    first (a pre-paint script does this in the browser), then the server's,
 *    because the settings panel promises these follow the account to another
 *    device — and only the *local* half was ever applied.
 */

const CHAPTER = {
  chapter: {
    id: 'chapter-1',
    title: 'One',
    order_key: 1,
    word_count: 4,
    revision_count: 1,
    version: 1,
    updated_at: '2026-09-10T00:00:00Z',
    created_at: '2026-09-10T00:00:00Z',
    has_content: true,
    current_revision_id: 'rev-1',
  },
  document: null,
  sanitized_html: '<p>Text to read.</p>',
  plain_text: 'Text to read.',
  word_count: 4,
  revision_number: 1,
  revision_id: 'rev-1',
  editable: false,
  previous_chapter_id: null,
  next_chapter_id: 'chapter-2',
  work: {
    id: 'work-1',
    title: 'The Long Road',
    lifecycle: 'published',
    authors: [],
  },
};

/** The chapter whole-work mode appends. */
const SECOND = {
  ...CHAPTER,
  chapter: { ...CHAPTER.chapter, id: 'chapter-2', title: 'Two' },
  sanitized_html: '<p>And on it goes.</p>',
  plain_text: 'And on it goes.',
  revision_id: 'rev-2',
  previous_chapter_id: 'chapter-1',
  next_chapter_id: null,
};

/**
 * jsdom has no IntersectionObserver, so the real one is replaced by one that
 * fires as soon as it is asked to watch something — which is the state the
 * reader reaches by scrolling to the end, and the only part of the behaviour a
 * unit test can reach without a layout engine.
 */
class ImmediateObserver {
  static watching = 0;

  private callback: IntersectionObserverCallback;

  constructor(callback: IntersectionObserverCallback) {
    this.callback = callback;
  }

  observe(target: Element) {
    ImmediateObserver.watching += 1;
    this.callback(
      [{ isIntersecting: true, target } as unknown as IntersectionObserverEntry],
      this as unknown as IntersectionObserver,
    );
  }

  disconnect() {}
  unobserve() {}
  takeRecords(): IntersectionObserverEntry[] {
    return [];
  }
}

const TYPOGRAPHY = {
  font_scale: 1.3,
  line_height: 2,
  measure: 60,
  reader_theme: 'dark',
  distraction_free: false,
  version: 7,
};

interface Recorded {
  method: string;
  path: string;
  body: unknown;
}

let calls: Recorded[] = [];
let secondChapterFails = false;

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

beforeEach(() => {
  calls = [];
  secondChapterFails = false;
  ImmediateObserver.watching = 0;
  vi.stubGlobal('IntersectionObserver', ImmediateObserver);
  // jsdom has no layout and says so loudly; the reader restores a position.
  vi.stubGlobal('scrollTo', vi.fn());
  session.status = 'signed-in';
  session.me = {
    account: {
      id: 'acc-1',
      email: 'reader@example.com',
      age_state: 'declared_adult',
      email_verified: true,
      session_expires_at: '2026-10-01T00:00:00Z',
    },
    pseuds: [{ id: 'pseud-1', handle: 'reader', display_name: 'Reader', bio: null }],
    active_pseud_id: 'pseud-1',
    capabilities: {
      can_read: true,
      can_write: true,
      can_message: true,
      can_be_listed: true,
      max_rating: 'explicit',
    },
  } as never;

  vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input.toString();
    const method = init?.method ?? 'GET';
    const body = init?.body ? JSON.parse(String(init.body)) : undefined;
    calls.push({ method, path: new URL(url, 'http://localhost').pathname, body });

    if (url.includes('/settings/typography')) return json(TYPOGRAPHY);
    if (url.includes('/chapters/chapter-2')) {
      return secondChapterFails
        ? json({ error: { code: 'INTERNAL', message: 'no' } }, 500)
        : json(SECOND);
    }
    if (url.includes('/chapters/')) return json(CHAPTER);
    if (url.includes('/reading/progress')) return new Response(null, { status: 204 });
    return json({ error: { code: 'NOT_FOUND', message: 'no' } }, 404);
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
  for (const property of ['reader', 'distractionFree']) {
    delete (document.documentElement.dataset as Record<string, string | undefined>)[property];
  }
});

describe('the reading surface', () => {
  it('shows the chapter the server sent', async () => {
    render(Reader, { workId: 'work-1', chapterId: 'chapter-1' });
    await waitFor(() => expect(document.body.textContent).toContain('Text to read.'));
  });

  it("applies the reader's stored appearance, which is what the type controls promise", async () => {
    render(Reader, { workId: 'work-1', chapterId: 'chapter-1' });
    await waitFor(() =>
      expect(document.documentElement.style.getPropertyValue('--reader-font-scale')).toBe('1.3'),
    );
    expect(document.documentElement.dataset.reader).toBe('dark');
    // The stored copy for the next load, written from the server's answer.
    expect(localStorage.getItem(TYPOGRAPHY_STORAGE_KEY)).toContain('"font_scale":1.3');
  });

  it('records the reading when the reader arrives, without waiting for a scroll', async () => {
    render(Reader, { workId: 'work-1', chapterId: 'chapter-1' });
    await waitFor(
      () => expect(calls.some((call) => call.method === 'PUT' && call.path.endsWith('/reading/progress'))).toBe(true),
      { timeout: 4000 },
    );

    const progress = calls.find(
      (call) => call.method === 'PUT' && call.path.endsWith('/reading/progress'),
    );
    expect(progress?.body).toMatchObject({
      subject_type: 'work',
      subject_id: 'work-1',
      chapter_id: 'chapter-1',
      content_revision: 'rev-1',
    });
    // The local copy goes on first, so a lost request still leaves a position.
    expect(localStorage.getItem('lorehaven.reading-position')).toContain('work-1');
  });

  it('reads on without stopping, appending the next chapter at the end', async () => {
    render(Reader, { workId: 'work-1', chapterId: 'chapter-1' });
    await waitFor(() => expect(document.body.textContent).toContain('Text to read.'));

    // Off by default: the reader is in one chapter and nothing else is fetched.
    expect(document.body.textContent).not.toContain('And on it goes.');
    expect(ImmediateObserver.watching).toBe(0);

    await fireEvent.click(
      [...document.querySelectorAll('button')].find((button) =>
        (button.textContent ?? '').includes('Read on without stopping'))!,
    );

    await waitFor(() => expect(document.body.textContent).toContain('And on it goes.'));
    // The first chapter is still on the page: appending adds, it does not
    // replace, which is what makes it reading on rather than navigating.
    expect(document.body.textContent).toContain('Text to read.');
    expect(localStorage.getItem('lorehaven.reader.whole-work')).toBe('true');
  });

  it('says so and offers the way on when a chapter cannot be appended', async () => {
    secondChapterFails = true;
    render(Reader, { workId: 'work-1', chapterId: 'chapter-1' });
    await waitFor(() => expect(document.body.textContent).toContain('Text to read.'));

    await fireEvent.click(
      [...document.querySelectorAll('button')].find((button) =>
        (button.textContent ?? '').includes('Read on without stopping'))!,
    );

    // A failed fetch is a dead end unless it says so and offers the way on: the
    // same append, tried again, and the chapter's own page.
    await waitFor(() =>
      expect(document.body.textContent).toContain('Try the next chapter again'),
    );
    expect(document.body.textContent).toContain('Open it on its own page');
  });

  it('drops what it appended when the reader turns the mode off', async () => {
    render(Reader, { workId: 'work-1', chapterId: 'chapter-1' });
    await waitFor(() => expect(document.body.textContent).toContain('Text to read.'));

    await fireEvent.click(
      [...document.querySelectorAll('button')].find((button) =>
        (button.textContent ?? '').includes('Read on without stopping'))!,
    );
    await waitFor(() => expect(document.body.textContent).toContain('And on it goes.'));

    await fireEvent.click(
      [...document.querySelectorAll('button')].find((button) =>
        (button.textContent ?? '').includes('One chapter at a time'))!,
    );

    // The page is the chapter the address names again, so the address and the
    // page agree about what is on screen.
    await waitFor(() => expect(document.body.textContent).not.toContain('And on it goes.'));
    expect(document.body.textContent).toContain('Text to read.');
  });
});
