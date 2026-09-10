import { afterEach, describe, expect, it, vi } from 'vitest';
import { SessionStore } from './session.svelte';

/**
 * The distinction these tests exist to protect: "the server says there is no
 * session" and "we could not ask" are not the same answer, and an interface
 * that conflates them signs people out for being on a train.
 */

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const ME = {
  account: {
    id: 'acc-1',
    email: 'reader@example.com',
    age_state: 'declared_adult',
    email_verified: false,
    session_expires_at: '2026-10-01T00:00:00Z',
  },
  pseuds: [
    { id: 'pseud-1', handle: 'quill', display_name: 'Quill', bio: null },
    { id: 'pseud-2', handle: 'ink', display_name: 'Ink', bio: 'second face' },
  ],
  active_pseud_id: 'pseud-2',
  capabilities: {
    can_read: true,
    can_write: true,
    can_message: false,
    can_be_listed: true,
    max_rating: 'explicit',
  },
};

const UNAUTHORISED = {
  error: { code: 'AUTH_REQUIRED', message: 'Sign in to continue.', request_id: 'req-1' },
};

afterEach(() => {
  vi.restoreAllMocks();
});

describe('the session store', () => {
  it('starts out not knowing, and becomes anonymous when the server says so', async () => {
    const store = new SessionStore();
    expect(store.status).toBe('unknown');
    expect(store.isSignedIn).toBe(false);

    vi.spyOn(globalThis, 'fetch').mockResolvedValue(json(UNAUTHORISED, 401));
    await store.refresh();

    expect(store.status).toBe('anonymous');
    expect(store.me).toBeNull();
    // A 401 is an answer, not an error to show the reader.
    expect(store.error).toBeNull();
  });

  it('reports the account, its pseuds and the active one', async () => {
    const store = new SessionStore();
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(json(ME));
    await store.refresh();

    expect(store.status).toBe('signed-in');
    expect(store.isSignedIn).toBe(true);
    expect(store.pseuds.map((pseud) => pseud.handle)).toEqual(['quill', 'ink']);
    // A session remembers which face it is wearing, so the active pseud comes
    // from the session rather than from whichever is first.
    expect(store.activePseud?.handle).toBe('ink');
  });

  it('does not claim anybody was signed out when the request simply failed', async () => {
    const store = new SessionStore();
    vi.spyOn(globalThis, 'fetch').mockRejectedValue(new TypeError('Failed to fetch'));
    await store.refresh();

    expect(store.status).toBe('unknown');
    expect(store.error).toBeInstanceOf(Error);
    expect(store.isSignedIn).toBe(false);
  });

  it('de-duplicates concurrent refreshes', async () => {
    const store = new SessionStore();
    const spy = vi.spyOn(globalThis, 'fetch').mockImplementation(() =>
      Promise.resolve(json(ME)),
    );

    await Promise.all([store.refresh(), store.refresh(), store.refresh()]);
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it('still looks signed out after a failed sign-out, and says so', async () => {
    const store = new SessionStore();
    const spy = vi.spyOn(globalThis, 'fetch');

    spy.mockResolvedValueOnce(json(ME));
    await store.refresh();
    expect(store.isSignedIn).toBe(true);

    // The reader asked to leave; the network does not get a vote.
    spy.mockRejectedValueOnce(new TypeError('Failed to fetch'));
    await store.signOutNow();

    expect(store.status).toBe('anonymous');
    expect(store.me).toBeNull();
    // Not thrown — nothing useful could be done with it — but recorded, so the
    // page can admit the session may still be live on the server.
    expect(store.error).toBeInstanceOf(Error);
  });

  it('refreshes after choosing a pseud, so the active face comes from the server', async () => {
    const store = new SessionStore();
    const spy = vi.spyOn(globalThis, 'fetch');

    spy.mockResolvedValueOnce(json(ME));
    await store.refresh();

    const switched = { ...ME, active_pseud_id: 'pseud-1' };
    spy.mockResolvedValueOnce(new Response(null, { status: 204 }));
    spy.mockResolvedValueOnce(json(switched));

    await store.usePseud('pseud-1');
    expect(store.activePseud?.handle).toBe('quill');

    const [activateCall, meCall] = spy.mock.calls.slice(-2);
    expect(String(activateCall[0])).toContain('/pseuds/pseud-1/activate');
    expect(String(meCall[0])).toContain('/auth/me');
  });
});
