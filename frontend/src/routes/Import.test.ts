import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Import from './Import.svelte';
import { session } from '../lib/session.svelte';

/**
 * The import page's contract, which is mostly about consent:
 *
 *  * A preview writes nothing. If checking what would happen already imported
 *    it, the confirmation step would be theatre.
 *  * Confirming carries the plan the reader was shown, so the server can refuse
 *    a consent that no longer applies to what would happen.
 *  * Capability absence is visible: a source this build cannot read is listed
 *    as unavailable rather than left out.
 */

interface Recorded {
  method: string;
  path: string;
  body: unknown;
}

const PREVIEW = {
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
  chapter_count: 109,
  chapters: [
    { ordinal: 1, source_chapter_key: '301778', title: 'Good Morning' },
    { ordinal: 2, source_chapter_key: '301779', title: 'The Tests' },
  ],
  plan: { plan: 'create', added: 109, removed: 0, reordered: 0, retitled: 0, changes: [] },
  is_new: true,
  duplicate_warning: null,
};

let calls: Recorded[] = [];
let imports: unknown[] = [];
let sources: unknown[] = [];

/**
 * Text with runs of whitespace collapsed.
 *
 * A sentence in a Svelte template wraps across lines, so `textContent` carries
 * the source file's indentation. Comparing against that would make the test fail
 * the day somebody reflowed a paragraph — a test asserting the layout rather
 * than the words.
 */
function squashed(text: string): string {
  return text.replace(/\s+/g, ' ');
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

beforeEach(() => {
  calls = [];
  imports = [];
  sources = [
    {
      key: 'royalroad',
      display_name: 'Royal Road',
      adapter_version: '1',
      enabled: true,
      disabled_reason: null,
      health: 'ok',
      last_checked_at: null,
      capabilities: { known: true, metadata: true, chapters: true, per_chapter_fetch: true },
      robots: {
        honour_disallow: true,
        honour_crawl_delay: true,
        note: "paths a source's robots.txt forbids are refused, and the failure names the rule",
      },
    },
    {
      key: 'ffnet',
      display_name: 'FanFiction.net',
      adapter_version: '1',
      enabled: false,
      disabled_reason: 'the site refuses automated requests',
      health: 'unavailable',
      last_checked_at: null,
      capabilities: { known: false },
      robots: {
        honour_disallow: true,
        honour_crawl_delay: true,
        note: "paths a source's robots.txt forbids are refused, and the failure names the rule",
      },
    },
  ];

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
    const method = init?.method ?? 'GET';
    calls.push({ method, path, body: init?.body ? JSON.parse(String(init.body)) : null });

    if (path === '/api/v1/imports/sources') return json({ items: sources });
    if (path === '/api/v1/imports' && method === 'GET') {
      return json({ items: imports, next_cursor: null });
    }
    if (path === '/api/v1/imports/preview') return json(PREVIEW);
    if (path === '/api/v1/imports' && method === 'POST') {
      return json(
        {
          import_id: 'imp-1',
          job_id: 'job-1',
          source_key: 'royalroad',
          destination: 'library',
          dry_run: false,
          state: 'queued',
          created_at: '2026-09-10T00:00:00Z',
        },
        202,
      );
    }
    return json({ error: { code: 'NOT_FOUND', message: 'no' } }, 404);
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('the import page', () => {
  it('describes what confirming would do, and stores nothing while previewing', async () => {
    render(Import);
    await waitFor(() => expect(document.body.textContent).toContain('Sources'));

    await fireEvent.input(screen.getByLabelText(/Address of the work/), {
      target: { value: PREVIEW.source_url },
    });
    await fireEvent.submit(document.querySelector('form') as HTMLFormElement);

    await waitFor(() => expect(document.body.textContent).toContain('Mother of Learning'));
    expect(document.body.textContent).toContain('109 chapters');
    expect(document.body.textContent).toContain('imported as a new work');

    // The load-bearing assertion: a preview fetched nothing into the library.
    expect(calls.some((call) => call.method === 'POST' && call.path === '/api/v1/imports')).toBe(
      false,
    );
  });

  it('confirms with the plan it showed, so the server can refuse a stale consent', async () => {
    render(Import);
    await waitFor(() => expect(document.body.textContent).toContain('Sources'));

    await fireEvent.input(screen.getByLabelText(/Address of the work/), {
      target: { value: PREVIEW.source_url },
    });
    await fireEvent.submit(document.querySelector('form') as HTMLFormElement);

    await waitFor(() => expect(document.body.textContent).toContain('Mother of Learning'));
    [...document.querySelectorAll('button')]
      .find((button) => (button.textContent ?? '').includes('Import 109 chapters'))
      ?.click();

    await waitFor(() =>
      expect(calls.some((call) => call.method === 'POST' && call.path === '/api/v1/imports')).toBe(
        true,
      ),
    );
    const start = calls.find((call) => call.method === 'POST' && call.path === '/api/v1/imports');
    expect(start?.body).toMatchObject({
      destination: 'library',
      confirmed_plan: 'create',
      dry_run: false,
    });

    await waitFor(() => expect(document.body.textContent).toContain('Import queued'));
  });

  it('lists a source this build cannot read as unavailable, rather than hiding it', async () => {
    render(Import);
    await waitFor(() => expect(document.body.textContent).toContain('FanFiction.net'));
    expect(document.body.textContent).toContain('refuses automated requests');
  });

  it('offers a dry run that stores no chapters', async () => {
    render(Import);
    await waitFor(() => expect(document.body.textContent).toContain('Sources'));

    await fireEvent.input(screen.getByLabelText(/Address of the work/), {
      target: { value: PREVIEW.source_url },
    });
    await fireEvent.submit(document.querySelector('form') as HTMLFormElement);

    await waitFor(() => expect(document.body.textContent).toContain('Mother of Learning'));

    const dryRun = document.querySelector('input[type="checkbox"]') as HTMLInputElement;
    await fireEvent.click(dryRun);

    // The button relabels itself, so the reader can see the run is a dry one.
    await waitFor(() =>
      expect(
        [...document.querySelectorAll('button')].some((button) =>
          (button.textContent ?? '').includes('Run without storing'),
        ),
      ).toBe(true),
    );
    [...document.querySelectorAll('button')]
      .find((button) => (button.textContent ?? '').includes('Run without storing'))
      ?.click();

    await waitFor(() =>
      expect(calls.some((call) => call.method === 'POST' && call.path === '/api/v1/imports')).toBe(
        true,
      ),
    );
    const start = calls.find((call) => call.method === 'POST' && call.path === '/api/v1/imports');
    expect(start?.body).toMatchObject({ dry_run: true });
  });

  it('offers cancel only where the server says the import can still be cancelled', async () => {
    imports = [
      {
        id: 'imp-1',
        source_key: 'royalroad',
        source_url: 'https://www.royalroad.com/fiction/21220/x',
        destination: 'library',
        state: 'running',
        dry_run: false,
        library_item_id: null,
        report: null,
        created_at: '2026-09-10T00:00:00Z',
        updated_at: '2026-09-10T00:00:01Z',
        cancellable: true,
      },
    ];
    render(Import);
    await waitFor(() => expect(document.body.textContent).toContain('running'));
    expect(
      [...document.querySelectorAll('button')].some((button) =>
        (button.getAttribute('aria-label') ?? '').includes('Cancel'),
      ),
    ).toBe(true);
  });

  it('states the terms the instance reads sources on', async () => {
    render(Import);
    // Waits for a *source*, not for the heading: the terms arrive with the
    // catalogue, so a page that has rendered "Sources" has not yet rendered
    // what it means to read one.
    await waitFor(() => expect(document.body.textContent).toContain('Royal Road'));

    expect(squashed(document.body.textContent ?? '')).toContain('forbids are refused');
  });

  it('marks the instance that has stopped honouring a source\'s rules', async () => {
    // The state a reader on such an instance has no other way to see. It is
    // marked rather than merely mentioned, because the difference between an
    // instance that asks and one that does not is not a detail of the layout.
    const terms = {
      honour_disallow: false,
      honour_crawl_delay: true,
      note: 'this instance reads paths a source\'s robots.txt forbids',
    };
    sources = (sources as { key: string }[]).map((source) => ({ ...source, robots: terms }));

    render(Import);
    await waitFor(() => expect(document.body.textContent).toContain('Royal Road'));

    const note = [...document.querySelectorAll('p')].find((p) =>
      (p.textContent ?? '').includes('robots.txt forbids'),
    );
    expect(note?.className).toContain('override');
    // And it says plainly that the pace was not switched off with it.
    expect(squashed(note?.textContent ?? '')).toContain(
      'crawl delay is enforced either way',
    );
  });
});
