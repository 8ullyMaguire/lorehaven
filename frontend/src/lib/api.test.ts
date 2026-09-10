import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError, apiFetch } from './api';

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
