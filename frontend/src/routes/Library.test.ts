import { render, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Library from './Library.svelte';
import { session } from '../lib/session.svelte';

/**
 * The library page's contract:
 *
 *  * An imported work shows where it came from. For a copy, the source address
 *    is the only way back to the original.
 *  * Checking for updates reports what the source says. "Nothing has changed"
 *    is an answer, and leaving it blank reads like a failure.
 *  * Checking stores nothing — it is a preview, and the reader confirms before
 *    anything is written.
 */

interface Recorded {
  method: string;
  path: string;
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
  status: 'complete',
  source_updated_at: '2026-09-01T00:00:00Z',
  last_synced_at: '2026-09-09T00:00:00Z',
  created_at: '2026-09-08T00:00:00Z',
  updated_at: '2026-09-09T00:00:00Z',
};

let calls: Recorded[] = [];
let items: unknown[] = [];
let plan = { plan: 'no_change', added: 0, removed: 0, reordered: 0, retitled: 0, changes: [] };

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

beforeEach(() => {
  calls = [];
  items = [ITEM];
  plan = { plan: 'no_change', added: 0, removed: 0, reordered: 0, retitled: 0, changes: [] };

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
    const path = new URL(url, 'http://localhost').pathname;
    calls.push({ method: init?.method ?? 'GET', path });

    if (path === '/api/v1/library/items') return json({ items, next_cursor: null });
    if (path === '/api/v1/imports/preview') {
      return json({ ...ITEM, source_work_key: '21220', chapter_count: 109, chapters: [], plan, is_new: false, duplicate_warning: null });
    }
    return json({ error: { code: 'NOT_FOUND', message: 'no' } }, 404);
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('the library page', () => {
  it('lists an imported work with the address it came from', async () => {
    render(Library);
    await waitFor(() => expect(document.body.textContent).toContain('Mother of Learning'));
    // Provenance, which is the whole point of an imported copy.
    expect(document.body.textContent).toContain('royalroad.com/fiction/21220');
    expect(document.body.textContent).toContain('nobody103');
    expect(document.body.textContent).toContain('last synchronised 2026-09-09');
  });

  it('says plainly when the source has nothing new', async () => {
    render(Library);
    await waitFor(() => expect(document.body.textContent).toContain('Check for updates'));

    [...document.querySelectorAll('button')]
      .find((button) => (button.textContent ?? '').includes('Check for updates'))
      ?.click();

    await waitFor(() =>
      expect(document.body.textContent).toContain('No changes at the source.'),
    );
  });

  it('reports new chapters when the source has published more', async () => {
    plan = { plan: 'update', added: 3, removed: 0, reordered: 0, retitled: 0, changes: [] };
    render(Library);
    await waitFor(() => expect(document.body.textContent).toContain('Check for updates'));

    [...document.querySelectorAll('button')]
      .find((button) => (button.textContent ?? '').includes('Check for updates'))
      ?.click();

    await waitFor(() =>
      expect(document.body.textContent).toContain('3 new chapters at the source.'),
    );
  });

  it('stores nothing when checking for updates', async () => {
    render(Library);
    await waitFor(() => expect(document.body.textContent).toContain('Check for updates'));

    [...document.querySelectorAll('button')]
      .find((button) => (button.textContent ?? '').includes('Check for updates'))
      ?.click();

    await waitFor(() => expect(document.body.textContent).toContain('No changes at the source.'));
    // A check reads; it does not write. Only the two GETs and one preview.
    expect(calls.some((call) => call.method === 'POST' && call.path === '/api/v1/imports')).toBe(
      false,
    );
  });

  it('says so when the library is empty rather than showing an empty list', async () => {
    items = [];
    render(Library);
    await waitFor(() => expect(document.body.textContent).toContain('Nothing imported yet'));
  });
});
