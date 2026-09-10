import { render, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Jobs from './Jobs.svelte';
import { session } from '../lib/session.svelte';

/**
 * The queue page's contract, which is mostly about what it does *not* do:
 *
 *  * It offers cancel only where the server says the job is cancellable, so a
 *    button cannot be a promise the route refuses to keep.
 *  * It shows why a job failed rather than only that it failed — a state with no
 *    reason sends the reader to the logs.
 *  * It stops polling once nothing can change. A page that polls a queue of
 *    finished jobs for ever is a page that keeps a laptop awake.
 */

const RUNNING = {
  id: 'job-1',
  kind: 'maintenance',
  state: 'running',
  payload: '{"task":"probe"}',
  progress_permille: 400,
  checkpoint: 'step 4',
  last_error: null,
  attempts: 1,
  max_attempts: 5,
  available_at: '2026-09-10T00:00:00Z',
  created_at: '2026-09-10T00:00:00Z',
  updated_at: '2026-09-10T00:00:01Z',
  version: 3,
  requested_by: 'acc-1',
  cancellable: true,
};

const FAILED = {
  ...RUNNING,
  id: 'job-2',
  state: 'failed',
  progress_permille: 250,
  checkpoint: 'step 2',
  last_error: 'the far end hung up',
  attempts: 5,
  cancellable: false,
};

interface Recorded {
  method: string;
  path: string;
}

let calls: Recorded[] = [];
let jobs: unknown[] = [RUNNING];

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

beforeEach(() => {
  calls = [];
  jobs = [RUNNING];
  session.status = 'signed-in';
  session.me = {
    account: {
      id: 'acc-1',
      email: 'writer@example.com',
      age_state: 'declared_adult',
      email_verified: true,
      session_expires_at: '2026-10-01T00:00:00Z',
    },
    pseuds: [{ id: 'pseud-1', handle: 'writer', display_name: 'Writer', bio: null }],
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

    if (init?.method === 'POST' && path.endsWith('/cancel')) {
      return json({ ...RUNNING, state: 'cancelled', cancellable: false });
    }
    if (init?.method === 'POST' && path === '/api/v1/jobs') return json(RUNNING, 202);
    if (path === '/api/v1/jobs') return json({ items: jobs, next_cursor: null });
    return json({ error: { code: 'NOT_FOUND', message: 'no' } }, 404);
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('the queue page', () => {
  it('shows the progress and the checkpoint of a job in flight', async () => {
    render(Jobs);
    await waitFor(() => expect(document.body.textContent).toContain('maintenance'));
    expect(document.body.textContent).toContain('40%');
    expect(document.body.textContent).toContain('step 4');
  });

  it('shows why a failed job failed, and offers no cancel for it', async () => {
    jobs = [FAILED];
    render(Jobs);
    await waitFor(() => expect(document.body.textContent).toContain('the far end hung up'));
    const cancels = [...document.querySelectorAll('button')].filter((button) =>
      (button.getAttribute('aria-label') ?? '').includes('Cancel'),
    );
    expect(cancels.length).toBe(0);
  });

  it('cancels a running job through the route, and shows the answer', async () => {
    render(Jobs);
    await waitFor(() => expect(document.body.textContent).toContain('Cancel'));
    const cancel = [...document.querySelectorAll('button')].find((button) =>
      (button.getAttribute('aria-label') ?? '').includes('Cancel'),
    );
    cancel?.click();

    await waitFor(() => expect(document.body.textContent).toContain('cancelled'));
    expect(calls.some((call) => call.method === 'POST' && call.path.endsWith('/cancel'))).toBe(
      true,
    );
  });

  it('stops asking once every job has finished', async () => {
    jobs = [FAILED];
    vi.useFakeTimers();
    render(Jobs);
    await vi.waitFor(() => expect(document.body.textContent).toContain('failed'));
    const before = calls.filter((call) => call.path === '/api/v1/jobs').length;

    // Well past several polling intervals: nothing is active, so nothing polls.
    await vi.advanceTimersByTimeAsync(5000);
    const after = calls.filter((call) => call.path === '/api/v1/jobs').length;
    expect(after).toBe(before);
  });
});
