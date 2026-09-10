import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Register from './Register.svelte';

/**
 * The registration form's real job is the request it makes. These tests pin the
 * body (including that an empty display name is omitted rather than sent as an
 * empty string) and the CSRF header, and that the server's `field_errors` land
 * on the fields that caused them.
 */

const CREATED = {
  account: {
    id: 'acc-1',
    email: 'new@example.com',
    age_state: 'declared_adult',
    email_verified: false,
    session_expires_at: '2026-10-01T00:00:00Z',
  },
  capabilities: {
    can_read: true,
    can_write: true,
    can_message: true,
    can_be_listed: true,
    max_rating: 'explicit',
  },
};

const ME = {
  ...CREATED,
  pseuds: [{ id: 'pseud-1', handle: 'quill', display_name: 'Quill', bio: null }],
  active_pseud_id: 'pseud-1',
};

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

/** Route the two calls registration makes to the responses for this test. */
function mockApi(
  registerResponse: () => Response | Promise<Response>,
  meResponse: Response = json(ME),
) {
  return vi.spyOn(globalThis, 'fetch').mockImplementation((input) => {
    const url = String(typeof input === 'string' ? input : (input as Request).url);
    if (url.includes('/auth/register')) return Promise.resolve(registerResponse());
    if (url.includes('/auth/me')) return Promise.resolve(meResponse);
    return Promise.resolve(json({}));
  });
}

beforeEach(() => {
  Object.defineProperty(document, 'cookie', {
    configurable: true,
    value: 'lorehaven_csrf=csrf-token',
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('registration', () => {
  it('sends the details the server asked for, and nothing it did not', async () => {
    const spy = mockApi(() => json(CREATED, 201));
    const { container } = render(Register);

    await fireEvent.input(screen.getByLabelText(/Email address/), {
      target: { value: 'new@example.com' },
    });
    await fireEvent.input(screen.getByLabelText(/^Password/), {
      target: { value: 'a-long-enough-password' },
    });
    await fireEvent.input(screen.getByLabelText(/^Handle/), { target: { value: 'quill' } });
    await fireEvent.change(screen.getByLabelText(/18 or older/), {
      target: { value: 'adult' },
    });

    await fireEvent.submit(container.querySelector('form') as HTMLFormElement);

    await waitFor(() => expect(spy).toHaveBeenCalled());
    const [url, init] = spy.mock.calls[0];
    expect(String(url)).toContain('/api/v1/auth/register');
    expect(init?.method).toBe('POST');

    const body = JSON.parse(String(init?.body));
    expect(body).toEqual({
      email: 'new@example.com',
      password: 'a-long-enough-password',
      handle: 'quill',
      age_band: 'adult',
    });
    // An untouched optional field is omitted, so the server applies its own
    // default rather than being handed an empty string to validate.
    expect(body.display_name).toBeUndefined();

    // A state-changing, cookie-authenticated request carries the token.
    const headers = new Headers(init?.headers as HeadersInit);
    expect(headers.get('x-csrf-token')).toBe('csrf-token');

    // The password is not left in the form after a successful registration.
    await waitFor(() => {
      expect((screen.getByLabelText(/^Password/) as HTMLInputElement).value).toBe('');
    });
  });

  it('shows a duplicate handle on the handle field, not in a generic banner alone', async () => {
    mockApi(() =>
      json(
        {
          error: {
            code: 'VALIDATION_FAILED',
            message: 'That handle is already taken.',
            field_errors: { handle: 'That handle is already taken.' },
            request_id: 'req-7',
          },
        },
        422,
      ),
    );

    const { container } = render(Register);
    await fireEvent.input(screen.getByLabelText(/^Handle/), { target: { value: 'quill' } });
    await fireEvent.submit(container.querySelector('form') as HTMLFormElement);

    // It appears twice on purpose: once in the summary, so it is announced and
    // carries the request id, and once attached to the field that caused it.
    const shown = await screen.findAllByText('That handle is already taken.');
    expect(shown.length).toBeGreaterThanOrEqual(2);
    // The field itself is marked invalid for assistive technology.
    await waitFor(() => {
      expect(screen.getByLabelText(/^Handle/).getAttribute('aria-invalid')).toBe('true');
    });
    // And the request id is offered so it can be quoted to support.
    expect(screen.getByText('req-7')).toBeInTheDocument();
  });

  it('does not claim to have registered when the request failed', async () => {
    mockApi(() => json({ error: { code: 'INTERNAL', message: 'boom', request_id: 'r' } }, 500));
    const { container } = render(Register);

    await fireEvent.input(screen.getByLabelText(/Email address/), {
      target: { value: 'new@example.com' },
    });
    await fireEvent.input(screen.getByLabelText(/^Password/), {
      target: { value: 'a-long-enough-password' },
    });
    await fireEvent.input(screen.getByLabelText(/^Handle/), { target: { value: 'quill' } });
    await fireEvent.submit(container.querySelector('form') as HTMLFormElement);

    expect(await screen.findByText('boom')).toBeInTheDocument();
    // The form is still there, with the password still in it, because nothing
    // succeeded.
    expect((screen.getByLabelText(/^Password/) as HTMLInputElement).value).toBe(
      'a-long-enough-password',
    );
  });
});
