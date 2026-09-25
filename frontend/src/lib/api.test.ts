import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError, apiFetch, reverseMediaSearch } from './api';

/**
 * The client's job is to turn the server's documented error envelope into
 * something a component can act on: a stable code, field errors, and a request
 * id. These tests pin that contract, and pin the rule that a failure never
 * navigates.
 */

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('apiFetch', () => {
  it('returns parsed JSON on success', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      jsonResponse({ name: 'Lorehaven', api_version: 'v1' }),
    );

    const meta = await apiFetch<{ name: string; api_version: string }>('/meta');
    expect(meta.name).toBe('Lorehaven');
    expect(meta.api_version).toBe('v1');
  });

  it('turns the error envelope into an ApiError with code, fields and request id', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      jsonResponse(
        {
          error: {
            code: 'VALIDATION_FAILED',
            message: 'The handle is already taken.',
            field_errors: { handle: 'already taken' },
            request_id: 'req-9',
          },
        },
        422,
      ),
    );

    const failure = await apiFetch('/pseuds').catch((error: unknown) => error);
    expect(failure).toBeInstanceOf(ApiError);

    const api = failure as ApiError;
    expect(api.code).toBe('VALIDATION_FAILED');
    expect(api.status).toBe(422);
    expect(api.requestId).toBe('req-9');
    expect(api.fieldErrors.handle).toBe('already taken');
    expect(api.isAuthRequired).toBe(false);
    expect(api.isRetryable).toBe(false);
  });

  it('classifies rate limits and server faults as retryable, and 401 as auth', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      jsonResponse({ error: { code: 'RATE_LIMITED', message: 'slow down', request_id: 'r1' } }, 429),
    );
    const limited = (await apiFetch('/x').catch((e: unknown) => e)) as ApiError;
    expect(limited.isRetryable).toBe(true);

    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      jsonResponse({ error: { code: 'AUTH_REQUIRED', message: 'sign in', request_id: 'r2' } }, 401),
    );
    const unauth = (await apiFetch('/x').catch((e: unknown) => e)) as ApiError;
    expect(unauth.isAuthRequired).toBe(true);
  });

  it('does not throw on a non-JSON error body', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response('<html>gateway timeout</html>', {
        status: 504,
        headers: { 'content-type': 'text/html' },
      }),
    );

    const failure = (await apiFetch('/x').catch((e: unknown) => e)) as ApiError;
    expect(failure).toBeInstanceOf(ApiError);
    expect(failure.status).toBe(504);
    expect(failure.isRetryable).toBe(true);
  });

  it('reports an unreachable server as a network error rather than a crash', async () => {
    vi.spyOn(globalThis, 'fetch').mockRejectedValue(new TypeError('Failed to fetch'));

    const failure = (await apiFetch('/meta').catch((e: unknown) => e)) as ApiError;
    expect(failure.code).toBe('NETWORK_UNAVAILABLE');
    expect(failure.message).toContain('Could not reach Lorehaven');
  });

  it('handles 204 responses with no body', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(null, { status: 204 }));
    await expect(apiFetch('/x')).resolves.toBeUndefined();
  });

  it('sends a CSRF header on state-changing requests only', async () => {
    // A cookie is the transport for the CSRF token (spec §3.5).
    Object.defineProperty(document, 'cookie', {
      configurable: true,
      value: 'lorehaven_csrf=token-abc',
    });

    // A fresh Response per call: a body can only be read once.
    const spy = vi
      .spyOn(globalThis, 'fetch')
      .mockImplementation(() => Promise.resolve(jsonResponse({ ok: true })));

    await apiFetch('/works', { method: 'POST', body: JSON.stringify({}) });
    const postHeaders = new Headers(spy.mock.calls[0][1]?.headers as HeadersInit);
    expect(postHeaders.get('x-csrf-token')).toBe('token-abc');
    expect(postHeaders.get('content-type')).toBe('application/json');

    await apiFetch('/works');
    const getHeaders = new Headers(spy.mock.calls[1][1]?.headers as HeadersInit);
    expect(getHeaders.get('x-csrf-token')).toBeNull();
  });

  it('never navigates, whatever the status', async () => {
    // A background poll that 401s must not eject a reader from the page.
    const assign = vi.fn();
    Object.defineProperty(window, 'location', {
      configurable: true,
      value: { ...window.location, assign, href: '/' },
    });
    Object.defineProperty(document, 'cookie', { configurable: true, value: '' });

    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      jsonResponse({ error: { code: 'AUTH_REQUIRED', message: 'no session', request_id: 'r3' } }, 401),
    );

    await apiFetch('/library').catch(() => undefined);
    expect(assign).not.toHaveBeenCalled();
  });
});

describe('community list fetchers', () => {
  it('unwrap the server { items } envelope for the community page', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      jsonResponse({ items: [{ id: 'c1', name: 'General', position: 0, min_trust: 0 }] }),
    );
    const { fetchForums } = await import('./api');
    const forums = await fetchForums();
    expect(forums).toHaveLength(1);
    expect(forums[0].name).toBe('General');
  });

  it('unwrap the { items } envelope for blocks', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      jsonResponse({
        items: [{ blocked: 'acc-2', scope: 'all', note: null, created_at: '2026-09-15T00:00:00Z' }],
      }),
    );
    const { fetchBlocks } = await import('./api');
    const blocks = await fetchBlocks();
    expect(blocks).toHaveLength(1);
    expect(blocks[0].blocked).toBe('acc-2');
  });
});

/**
 * Reverse media search (spec §32.7.2-32.7.3). The client has to be able to
 * tell an exact match from a perceptual one, and must not be handed a
 * confidence number for a comparison the server could not make: a missing or
 * malformed hash is "not comparable", not "very different".
 */
describe('reverseMediaSearch', () => {
  it('passes the hash and algorithm through and returns the matches in server order', async () => {
    const spy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      jsonResponse({
        references: [
          {
            id: 'ref-exact',
            media_kind: 'image',
            perceptual_hash: '00ff',
            content_hash: 'sha256:a',
            curator_verified: false,
            match_kind: 'exact',
            match_distance: 0,
            match_confidence: 1,
            auto_attach: true,
          },
          {
            id: 'ref-near',
            media_kind: 'image',
            perceptual_hash: '00fc',
            content_hash: 'sha256:b',
            curator_verified: false,
            match_kind: 'perceptual',
            match_distance: 2,
            match_confidence: 0.96875,
            auto_attach: false,
          },
        ],
        works: [],
      }),
    );

    const view = await reverseMediaSearch({ hash: '00ff', algorithm: 'phash' });

    const body = JSON.parse(String(spy.mock.calls[0][1]?.body));
    expect(body).toEqual({ hash: '00ff', algorithm: 'phash' });
    // Order is the server's to decide and it is closest-first; the client must
    // not re-sort, or it would lose the server's tiebreak.
    expect(view.references.map((r) => r.id)).toEqual(['ref-exact', 'ref-near']);
    expect(view.references[0].match_kind).toBe('exact');
    expect(view.references[0].auto_attach).toBe(true);
    expect(view.references[1].match_kind).toBe('perceptual');
    expect(view.references[1].auto_attach).toBe(false);
  });

  it('reads an incomparable match as null rather than a confident low score', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      jsonResponse({
        references: [
          {
            id: 'ref-unknown',
            media_kind: 'image',
            perceptual_hash: null,
            content_hash: 'sha256:c',
            curator_verified: false,
            match_kind: 'perceptual',
            match_distance: null,
            match_confidence: null,
            auto_attach: false,
          },
        ],
        works: [],
      }),
    );

    const view = await reverseMediaSearch({ hash: '00ff' });
    const ref = view.references[0];
    expect(ref.match_distance).toBeNull();
    expect(ref.match_confidence).toBeNull();
    // A null confidence must never be read as 0, which would look like a
    // confident "not a match" and could hide a real one.
    expect(ref.auto_attach).toBe(false);
  });
});
