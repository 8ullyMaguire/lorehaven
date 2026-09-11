import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Library from './Library.svelte';
import { session } from '../lib/session.svelte';

/**
 * The library page's contract:
 *
 *  * An imported work shows where it came from. For a copy, the source address
 *    is the only way back to the original.
 *  * The reader's *own* facts about a work — status, tags, shelves — are drawn
 *    beside the work's, because those are what make it a library rather than a
 *    list.
 *  * Removing a selection answers per item. "3 of 5 removed" and which two did
 *    not is the honest answer; one boolean is not.
 *  * Taking a work off the shelf and deleting the copy it holds are two
 *    actions, and the request says which one the reader chose.
 *  * The free-space action says what it will delete *before* it deletes it.
 */

interface Recorded {
  method: string;
  path: string;
  body: unknown;
}

const ITEM = {
  id: 'item-1',
  source_key: 'royalroad',
  source_work_key: '21220',
  source_url: 'https://www.royalroad.com/fiction/21220/mother-of-learning',
  title: 'Mother of Learning',
  author_text: 'nobody103',
  author_url: 'https://www.royalroad.com/profile/26557',
  summary: 'Zorian is a mage student.',
  language: 'en',
  word_count: 806306,
  chapter_count: 109,
  source_display_name: 'Royal Road',
  status: 'complete',
  source_updated_at: '2026-09-01T00:00:00Z',
  last_synced_at: '2026-09-09T00:00:00Z',
  created_at: '2026-09-08T00:00:00Z',
  updated_at: '2026-09-09T00:00:00Z',
  reading_status: null as string | null,
  shelves: [] as string[],
  tags: [] as string[],
};

const SHELF = {
  id: 'shelf-1',
  name: 'Favourites',
  description: '',
  is_public: false,
  position: 0,
  item_count: 1,
  created_at: '2026-09-08T00:00:00Z',
  updated_at: '2026-09-08T00:00:00Z',
  version: 1,
};

const STORAGE = {
  imported_bytes: 2048,
  export_bytes: 0,
  total_bytes: 2048,
  item_count: 1,
  blob_count: 2,
  counts: 'stored bytes, each blob counted once',
};

let calls: Recorded[] = [];
let items: typeof ITEM[] = [];
let shelves: unknown[] = [];
let views: unknown[] = [];
let batch: unknown = null;
let storage: unknown = STORAGE;

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

beforeEach(() => {
  calls = [];
  items = [{ ...ITEM }];
  shelves = [SHELF];
  views = [];
  storage = STORAGE;
  batch = {
    succeeded: ['item-1', 'item-2'],
    failed: [{ id: 'item-3', code: 'NOT_FOUND' }],
    summary: '2 of 3 removed; 1 could not be removed',
    freed_bytes: 0,
    delete_copy: false,
    removed: 2,
  };

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
    const parsed = new URL(url, 'http://localhost');
    const path = parsed.pathname;
    calls.push({
      method: init?.method ?? 'GET',
      path: `${path}${parsed.search}`,
      body: init?.body ? JSON.parse(String(init.body)) : null,
    });

    if (path === '/api/v1/library/items') {
      return json({ items, total: items.length, next_cursor: null });
    }
    if (path === '/api/v1/shelves' && (init?.method ?? 'GET') === 'GET') {
      return json({ items: shelves, next_cursor: null });
    }
    if (path === '/api/v1/saved-views') return json({ items: views, next_cursor: null });
    if (path === '/api/v1/library/storage') return json(storage);
    if (path === '/api/v1/library/items/batch') return json(batch);
    if (path === '/api/v1/library/updates/check') return json({ job_id: 'job-1', items: 3 });
    return json({ error: { code: 'NOT_FOUND', message: 'no' } }, 404);
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

function buttonWith(text: string): HTMLButtonElement | undefined {
  return [...document.querySelectorAll('button')].find((button) =>
    (button.textContent ?? '').includes(text),
  ) as HTMLButtonElement | undefined;
}

describe('the library page', () => {
  it('lists an imported work with the address it came from', async () => {
    render(Library);
    await waitFor(() => expect(document.body.textContent).toContain('Mother of Learning'));
    // Provenance, which is the whole point of an imported copy.
    expect(document.body.textContent).toContain('royalroad.com/fiction/21220');
    expect(document.body.textContent).toContain('nobody103');
  });

  it('shows the reader’s own status, tags and shelves beside the work', async () => {
    items = [
      {
        ...ITEM,
        reading_status: 'reading',
        tags: ['wip', 'reread'],
        shelves: ['Favourites'],
      },
    ];
    render(Library);

    // Waiting on the card, not on a word the filter bar also contains: 'Reading'
    // is an <option> in the status select, so waiting for it would pass before a
    // single work had loaded, and the assertions below would run against an
    // empty page.
    await waitFor(() => expect(document.body.textContent).toContain('Mother of Learning'));
    await waitFor(() => expect(document.body.textContent).toContain('Reading'));
    // A tag and a shelf are the reader's own rows, and they are drawn as such.
    expect(document.body.textContent).toContain('wip');
    expect(document.body.textContent).toContain('reread');
    expect(document.body.textContent).toContain('Favourites');
  });

  it('removing a selection reports per item rather than as one flag', async () => {
    render(Library);
    await waitFor(() => expect(document.body.textContent).toContain('Mother of Learning'));

    const pick = document.querySelector(
      'input[type="checkbox"][aria-label], .card input[type="checkbox"]',
    ) as HTMLInputElement;
    await fireEvent.change(pick, { target: { checked: true } });
    await waitFor(() => expect(document.body.textContent).toContain('1 selected'));

    await fireEvent.click(buttonWith('Remove from library')!);

    await waitFor(() =>
      expect(document.body.textContent).toContain('2 of 3 removed; 1 could not be removed'),
    );
    // And which one did not, by identifier — the part a boolean cannot carry.
    expect(document.body.textContent).toContain('item-3');
  });

  it('offers taking a work off the shelf and deleting its copy as two actions', async () => {
    render(Library);
    await waitFor(() => expect(document.body.textContent).toContain('Mother of Learning'));

    const pick = document.querySelector('.card input[type="checkbox"]') as HTMLInputElement;
    await fireEvent.change(pick, { target: { checked: true } });
    await waitFor(() => expect(document.body.textContent).toContain('1 selected'));

    await fireEvent.click(buttonWith('Remove from library')!);
    await waitFor(() => expect(calls.some((call) => call.path.includes('batch'))).toBe(true));

    const referenceOnly = calls.find((call) => call.path.includes('batch'));
    expect(referenceOnly?.body).toMatchObject({ delete_copy: false });

    // The second action says the other thing, because the difference is
    // invisible afterwards. The checkbox is re-queried: the removal replaced the
    // card, so the element captured before it is detached and firing on it would
    // reach nothing.
    // Waited for, not merely queried: the removal puts the grid back into its
    // loading state, so the card is briefly absent and a bare query would
    // capture nothing.
    await waitFor(() =>
      expect(document.querySelector('.card input[type="checkbox"]')).not.toBeNull(),
    );
    const afterReload = document.querySelector('.card input[type="checkbox"]') as HTMLInputElement;
    await fireEvent.change(afterReload, { target: { checked: true } });
    await waitFor(() => expect(document.body.textContent).toContain('1 selected'));
    await fireEvent.click(buttonWith('Delete copies too')!);

    await waitFor(() =>
      expect(calls.filter((call) => call.path.includes('batch')).length).toBeGreaterThan(1),
    );
    const withCopy = calls.filter((call) => call.path.includes('batch')).at(-1);
    expect(withCopy?.body).toMatchObject({ delete_copy: true });
  });

  it('says what the free-space action will delete before it deletes it', async () => {
    render(Library);
    await waitFor(() => expect(document.body.textContent).toContain('Storage'));

    // The notice names the operation and the consequence, before anything is
    // pressed: the works stay, the imported text does not.
    expect(document.body.textContent).toContain('2.0 KB');
    expect(document.body.textContent).toContain('stored copies');
    expect(document.body.textContent).toContain('The works stay in your library');
  });

  it('carries the filters into the request rather than filtering what arrived', async () => {
    render(Library);
    await waitFor(() => expect(document.body.textContent).toContain('Mother of Learning'));

    const tag = document.querySelector('input[placeholder="any tag"]') as HTMLInputElement;
    // Both events: the value binding listens on `input`, and the reload listens
    // on `change` — a real reader produces both by typing and then leaving the
    // field, and the page is built for that rather than for a keystroke reload.
    await fireEvent.input(tag, { target: { value: 'wip' } });
    await fireEvent.change(tag, { target: { value: 'wip' } });

    await waitFor(() =>
      expect(calls.some((call) => call.path.includes('tags=wip'))).toBe(true),
    );
  });

  it('queues an update check as a background job', async () => {
    render(Library);
    await waitFor(() => expect(document.body.textContent).toContain('Check for updates'));

    await fireEvent.click(buttonWith('Check for updates')!);

    await waitFor(() =>
      expect(calls.some((call) => call.path === '/api/v1/library/updates/check')).toBe(true),
    );
    expect(calls.find((call) => call.path === '/api/v1/library/updates/check')?.method).toBe('POST');
    await waitFor(() =>
      expect(document.body.textContent).toContain('It runs in the background'),
    );
  });

  it('loads a saved view’s query into the filter bar, and says what it could not', async () => {
    views = [
      {
        id: 'view-1',
        name: 'Unread royalroad',
        query: {
          shelves: ['Favourites', 'Later'],
          tags: ['wip'],
          statuses: [],
          source: 'royalroad',
          updated_since: '2026-01-01T00:00:00Z',
          sort: 'words',
        },
        needs_repair: false,
        query_version: 1,
        sort: 'words',
        scope: 'library',
        pinned: true,
        is_public: false,
        created_at: '2026-09-08T00:00:00Z',
        updated_at: '2026-09-08T00:00:00Z',
        version: 1,
      },
    ];
    render(Library);
    await waitFor(() => expect(document.body.textContent).toContain('Unread royalroad'));

    await fireEvent.click(buttonWith('Unread royalroad')!);

    await waitFor(() =>
      expect(
        calls.some((call) => call.path.includes('tags=wip') && call.path.includes('source=royalroad')),
      ).toBe(true),
    );
    const applied = calls.filter((call) => call.path.includes('source=royalroad')).at(-1);
    expect(applied?.path).toContain('shelves=Favourites');
    expect(applied?.path).toContain('sort=words');

    // The bar holds one value per facet and the view held two shelves, so the
    // loss is said rather than dropped in silence.
    await waitFor(() => expect(document.body.textContent).toContain('shows the first of each'));
  });

  it('says so when the library is empty rather than showing an empty list', async () => {
    items = [];
    render(Library);
    await waitFor(() => expect(document.body.textContent).toContain('Nothing here'));
  });
});
