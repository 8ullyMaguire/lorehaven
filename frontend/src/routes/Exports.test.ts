import { render, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Exports from './Exports.svelte';
import { session } from '../lib/session.svelte';

/**
 * The exports page's contract:
 *
 *  * a format this instance cannot produce is shown and disabled, with what an
 *    operator would install — not silently missing;
 *  * the privacy notice is a checkbox and the button refuses until it is ticked,
 *    because the server refuses too;
 *  * a queued export shows its state rather than appearing to do nothing;
 *  * the reader's own exports and the copies kept in this browser are listed
 *    separately, because one is on the server and one is on their device.
 */

interface Recorded {
  method: string;
  path: string;
  body: unknown;
}

const CATALOGUE = {
  formats: [
    {
      format: 'epub',
      label: 'EPUB',
      media_type: 'application/epub+zip',
      extension: 'epub',
      builtin: true,
      available: true,
      requires: null,
      converter: null,
      converter_version: null,
    },
    {
      format: 'pdf',
      label: 'PDF',
      media_type: 'application/pdf',
      extension: 'pdf',
      builtin: false,
      available: false,
      requires: 'install Calibre (`ebook-convert`), which provides PDF, AZW3 and MOBI',
      converter: null,
      converter_version: null,
    },
  ],
  privacy_notice: 'An exported file is a copy of this work that leaves the server.',
  retention_days: 7,
};

const READY_EXPORT = {
  id: 'export-1',
  job_id: 'job-1',
  subject_type: 'library_item',
  subject_id: 'item-1',
  format: 'epub',
  label: 'EPUB',
  state: 'ready',
  output_bytes: 4096,
  downloadable: true,
  error: null,
  privacy_acknowledged: true,
  created_at: '2026-09-11T09:00:00Z',
  updated_at: '2026-09-11T09:01:00Z',
};

let calls: Recorded[] = [];
let exports: unknown[] = [];
let requests: unknown[] = [];

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

beforeEach(() => {
  calls = [];
  exports = [];
  requests = [];

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
    calls.push({
      method: init?.method ?? 'GET',
      path,
      body: init?.body ? JSON.parse(String(init.body)) : null,
    });

    if (path === '/api/v1/exports/formats') return json(CATALOGUE);
    if (path === '/api/v1/exports' && (init?.method ?? 'GET') === 'GET') {
      return json({ exports });
    }
    if (path === '/api/v1/exports' && init?.method === 'POST') {
      requests.push(JSON.parse(String(init.body)));
      return json({ ...READY_EXPORT, state: 'queued', downloadable: false }, 202);
    }
    if (path === '/api/v1/exports/export-1') return json(READY_EXPORT);
    return json({ error: { code: 'NOT_FOUND', message: 'no' } }, 404);
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
  window.history.replaceState({}, '', '/exports');
});

describe('the exports page', () => {
  it('offers a format the instance cannot make, and says what to install', async () => {
    // The picker appears where the subject does: a reader starts from a work.
    window.history.replaceState(
      {},
      '',
      '/exports?subject_type=library_item&subject_id=item-1&title=A%20Work',
    );
    render(Exports);
    await waitFor(() => expect(document.body.textContent).toContain('EPUB'));
    // Unavailable, and not hidden: a picker that omits PDF makes the reader think
    // they missed it.
    expect(document.body.textContent).toContain('PDF is not available on this instance');
    expect(document.body.textContent).toContain('install Calibre');
  });

  it('lists an export with its state, and offers the file when it is ready', async () => {
    exports = [READY_EXPORT];
    render(Exports);
    await waitFor(() => expect(document.body.textContent).toContain('Ready'));
    expect(document.body.textContent).toContain('4.0 KB');
    expect(document.body.textContent).toContain('Download');
  });

  it('will not ask for an export until the notice is acknowledged', async () => {
    // A subject in the query string is how a reader arrives from a work.
    window.history.replaceState(
      {},
      '',
      '/exports?subject_type=library_item&subject_id=item-1&title=Mother%20of%20Learning',
    );
    render(Exports);

    const button = await waitFor(() => {
      const found = Array.from(document.querySelectorAll('button')).find((node) =>
        node.textContent?.includes('Make the export'),
      );
      expect(found).toBeTruthy();
      return found as HTMLButtonElement;
    });

    // The notice is on the page, and the action is refused until it is read.
    expect(document.body.textContent).toContain('leaves the server');
    expect(button.disabled).toBe(true);

    const box = document.querySelector('input[type="checkbox"]') as HTMLInputElement;
    box.click();
    await waitFor(() => expect(button.disabled).toBe(false));

    button.click();
    await waitFor(() => expect(requests.length).toBe(1));
    expect(requests[0]).toMatchObject({
      subject_type: 'library_item',
      subject_id: 'item-1',
      format: 'epub',
      acknowledge_privacy: true,
    });
  });

  it('says the export is queued rather than looking like nothing happened', async () => {
    window.history.replaceState(
      {},
      '',
      '/exports?subject_type=library_item&subject_id=item-1&title=A%20Work',
    );
    render(Exports);
    await waitFor(() => expect(document.body.textContent).toContain('Make the export'));

    const box = document.querySelector('input[type="checkbox"]') as HTMLInputElement;
    box.click();
    const button = Array.from(document.querySelectorAll('button')).find((node) =>
      node.textContent?.includes('Make the export'),
    ) as HTMLButtonElement;
    await waitFor(() => expect(button.disabled).toBe(false));
    button.click();

    await waitFor(() => expect(document.body.textContent).toContain('Queued'));
    // The state is shown, not the word "ready": a queued EPUB is a job.
    expect(document.body.textContent).toContain('Waiting');
  });

  it('keeps the exports and the local copies as separate lists', async () => {
    exports = [READY_EXPORT];
    render(Exports);
    await waitFor(() => expect(document.body.textContent).toContain('Your exports'));
    expect(document.body.textContent).toContain('Kept in this browser');
    // Nothing is kept, and the page says so rather than showing an empty list.
    expect(document.body.textContent).toContain('Nothing is kept in this browser yet');
  });
});
