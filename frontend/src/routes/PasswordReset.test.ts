import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import PasswordReset from './PasswordReset.svelte';
import { session } from '../lib/session.svelte';

/**
 * The reset flow has one property that is easy to get wrong and hard to see:
 * redeeming a token ends *every* session on the account, including the one the
 * browser is holding. A browser check found the shell still claiming a session
 * afterwards, so it is pinned here.
 */

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const UNAUTHORISED = {
  error: { code: 'AUTH_REQUIRED', message: 'Sign in to continue.', request_id: 'req-1' },
};

function mockApi() {
  return vi.spyOn(globalThis, 'fetch').mockImplementation((input) => {
    const url = String(typeof input === 'string' ? input : (input as Request).url);
    if (url.includes('/auth/password-reset/complete')) {
      return Promise.resolve(new Response(null, { status: 204 }));
    }
    if (url.includes('/auth/password-reset')) {
      return Promise.resolve(
        json({ message: 'If that address has an account, a link is on its way.', development_token: 'tok-1' }),
      );
    }
    // Every session was just revoked, so the session endpoint now says 401.
    return Promise.resolve(json(UNAUTHORISED, 401));
  });
}

beforeEach(() => {
  session.status = 'unknown';
  session.me = null;
  session.error = null;
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('password reset', () => {
  it('offers the token step when this instance cannot send mail', async () => {
    mockApi();
    const { container } = render(PasswordReset);

    await fireEvent.input(screen.getByLabelText(/Email address/), {
      target: { value: 'reader@example.com' },
    });
    await fireEvent.submit(container.querySelector('form') as HTMLFormElement);

    expect(await screen.findByLabelText(/Reset token/)).toBeInTheDocument();
    // The token is prefilled and the page says why it is visible.
    await waitFor(() => {
      expect((screen.getByLabelText(/Reset token/) as HTMLInputElement).value).toBe('tok-1');
    });
    expect(screen.getByText(/no mail transport configured/)).toBeInTheDocument();
  });

  it('tells the shell that every session has ended', async () => {
    mockApi();
    const { container } = render(PasswordReset);

    await fireEvent.input(screen.getByLabelText(/Email address/), {
      target: { value: 'reader@example.com' },
    });
    await fireEvent.submit(container.querySelector('form') as HTMLFormElement);

    // Wait for the second step; the first submit is asynchronous.
    await screen.findByLabelText(/Reset token/);

    await fireEvent.input(screen.getByLabelText(/New password/), {
      target: { value: 'an-even-longer-password' },
    });
    await fireEvent.submit(container.querySelector('form') as HTMLFormElement);

    expect(await screen.findByText('Password changed')).toBeInTheDocument();
    await waitFor(() => {
      expect(session.isSignedIn).toBe(false);
      expect(session.status).toBe('anonymous');
    });
  });
});
