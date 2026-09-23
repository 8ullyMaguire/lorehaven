// Mock the governance component so App.test.ts doesn't need to resolve its
// transitive imports. The real api.ts has no .svelte imports, so mocking
// the component is sufficient.
vi.mock('./lib/components/DirectoryGovernance.svelte', () => ({
  default: () => ({}),
}));

import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';

/**
 * A shell-level integration test.
 *
 * This exists because of a real bug: the appearance selector used
 * `bind:value` *and* `onchange`, and persistence silently depended on which
 * listener Svelte attached first — so choosing a theme applied it for the
 * session but never remembered it. Unit tests on `theme.ts` could not catch
 * that; only rendering the shell and changing the control can.
 *
 * It grew a second job in Milestone 2: the shell has to know who is signed in,
 * and that knowledge is a fetch. A test that mocks the API per URL is the only
 * place where "the header is wired to the session store" can be checked.
 */

const META = {
  name: 'Lorehaven',
  version: '0.1.0',
  build: '0.1.0+abc1234',
  api_version: 'v1',
  environment: 'development',
  base_url: 'http://localhost:8080',
  policy: {
    anonymous_reading: true,
    anonymous_max_rating: 'teen',
    unknown_age_max_rating: 'teen',
    minor_max_rating: 'general',
    adult_max_rating: 'explicit',
    registration_open: true,
    csrf_required: true,
  },
};

const READY = {
  status: 'ready',
  build: '0.1.0+abc1234',
  checks: {
    database: { ok: true, detail: 'sqlite reachable' },
    migrations: { ok: true, detail: '1 migration(s) applied' },
    storage: { ok: true, detail: '/tmp is writable' },
  },
};

const ME = {
  account: {
    id: 'acc-1',
    email: 'reader@example.com',
    age_state: 'declared_adult',
    email_verified: false,
    session_expires_at: '2026-10-01T00:00:00Z',
  },
  pseuds: [{ id: 'pseud-1', handle: 'quill', display_name: 'Quill', bio: null }],
  active_pseud_id: 'pseud-1',
  capabilities: {
    can_read: true,
    can_write: true,
    can_message: true,
    can_be_listed: true,
    max_rating: 'explicit',
  },
};

const UNAUTHORISED = {
  error: { code: 'AUTH_REQUIRED', message: 'Sign in to continue.', request_id: 'req-1' },
};

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

/** Answer each URL the shell asks for. `signedIn` decides who /auth/me says we are. */
function mockShell(signedIn: boolean) {
  return vi.spyOn(globalThis, 'fetch').mockImplementation((input) => {
    const url = String(typeof input === 'string' ? input : (input as Request).url);
    if (url.includes('/health/ready')) return Promise.resolve(json(READY));
    if (url.includes('/auth/me')) {
      return Promise.resolve(signedIn ? json(ME) : json(UNAUTHORISED, 401));
    }
    return Promise.resolve(json(META));
  });
}

beforeEach(() => {
  window.localStorage.clear();
  document.documentElement.removeAttribute('data-theme');
  window.history.pushState({}, '', '/');
});

afterEach(() => {
  vi.restoreAllMocks();
  window.localStorage.clear();
  window.history.pushState({}, '', '/');
});

describe('application shell', () => {
  it('renders the wordmark and the full desktop navigation', () => {
    mockShell(false);
    render(App);

    const brand = document.querySelector('header .brand');
    expect(brand?.textContent?.trim()).toBe('Lorehaven');

    for (const label of [
      'Discover',
      'Search',
      'Library',
      'Write',
      'Community',
      'Notifications',
      'Pseud',
    ]) {
      expect(screen.getAllByText(label).length).toBeGreaterThan(0);
    }
  });

  it('shows real instance values fetched from the API, not placeholders', async () => {
    mockShell(false);
    render(App);

    await waitFor(() => {
      expect(screen.getByText('0.1.0+abc1234')).toBeInTheDocument();
    });
    // The health panel is rendered from the readiness response.
    expect(await screen.findByText('sqlite reachable')).toBeInTheDocument();
    expect(screen.getByText('1 migration(s) applied')).toBeInTheDocument();
  });

  it('applies and persists an appearance choice', async () => {
    mockShell(false);
    render(App);

    const select = screen.getByLabelText('Appearance') as HTMLSelectElement;
    await fireEvent.change(select, { target: { value: 'clear-day' } });

    // Applied immediately…
    expect(document.documentElement.dataset.theme).toBe('clear-day');
    // …and remembered, which is the part that regressed.
    expect(localStorage.getItem('lorehaven.theme')).toBe('clear-day');
  });

  it('honours a stored preference on load', () => {
    mockShell(false);
    window.localStorage.setItem('lorehaven.theme', 'after-hours');
    render(App);
    expect(document.documentElement.dataset.theme).toBe('after-hours');
  });

  it('marks the current destination for assistive technology', async () => {
    mockShell(false);
    window.history.pushState({}, '', '/library');
    render(App);
    window.dispatchEvent(new PopStateEvent('popstate'));

    await waitFor(() => {
      const current = screen
        .getAllByText('Library')
        .filter((node) => node.getAttribute('aria-current') === 'page');
      expect(current.length).toBeGreaterThan(0);
    });
  });

  it('offers ways in when nobody is signed in', async () => {
    mockShell(false);
    render(App);

    // The header is driven by the session store, which asked /auth/me.
    await waitFor(() => {
      expect(screen.getByText('Sign in')).toBeInTheDocument();
    });
    expect(screen.getByText('Register')).toBeInTheDocument();
    expect(screen.queryByTestId('writing-as')).toBeNull();
  });

  it('names the acting pseud once signed in, and points at the account', async () => {
    mockShell(true);
    render(App);

    const writingAs = await screen.findByTestId('writing-as');
    expect(writingAs.textContent).toContain('@quill');
    expect(screen.getByText('Account')).toBeInTheDocument();
    expect(screen.queryByText('Register')).toBeNull();
  });

  it('renders the registration page at /register', async () => {
    mockShell(false);
    window.history.pushState({}, '', '/register');
    render(App);
    window.dispatchEvent(new PopStateEvent('popstate'));

    expect(await screen.findByRole('heading', { name: 'Create an account' })).toBeInTheDocument();
    expect(screen.getByLabelText(/Email address/)).toBeInTheDocument();
  });

  it('renders the account page at /account, which asks the reader to sign in', async () => {
    mockShell(false);
    window.history.pushState({}, '', '/account');
    render(App);
    window.dispatchEvent(new PopStateEvent('popstate'));

    expect(await screen.findByText('Sign in to see this page')).toBeInTheDocument();
  });
});
