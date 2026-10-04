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

    // The four destinations a person arrives to do are plain links in the row.
    for (const label of ['Discover', 'Search', 'Library', 'Write']) {
      expect(screen.getAllByText(label).length).toBeGreaterThan(0);
    }

    // The rest live under five menus rather than in one eighteen-link row. The
    // menu TRIGGERS are always rendered; their contents are not, because a menu
    // that renders its items when closed is not a menu.
    const triggers = screen.getAllByTestId('menu-trigger').map((b) => b.textContent?.trim() ?? '');
    for (const menu of ['Read', 'Create', 'Shelf', 'Forum', 'More']) {
      expect(triggers).toContain(menu);
    }

    /**
     * The regression that made this row overflow: "Write" and "Library" were both
     * a primary link AND a menu trigger, so the row read Discover, Library,
     * Search, Write, Read, Write, Library, Community, More -- the same word twice,
     * twice over. Asserting the absence is the only way to catch a duplicate
     * label, because asserting the presence passes either way.
     */
    const labels = [
      ...[...document.querySelectorAll('nav.desktop > a')].map((a) => a.textContent?.trim()),
      ...triggers,
    ];
    expect(labels.length).toBe(new Set(labels).size);

    // The point of the redesign: a reader must not have to scroll a hidden
    // overflow row to find where things are. Every destination is reachable
    // from the header, by link or by opening one menu.
    const inHeader = new Set<string>();
    for (const a of document.querySelectorAll('header a')) {
      const href = a.getAttribute('href');
      if (href) inHeader.add(href);
    }
    // Four primary links plus the drawer, which lists all eighteen.
    for (const href of ['/discover', '/library', '/search', '/write']) {
      expect(inHeader.has(href)).toBe(true);
    }
  });

  it('offers the concierge queue in the navigation', async () => {
    // Mutation M3 in the wiring proof: removing the nav entry left all 45 tests
    // green, because a page can be reachable by URL and still be unreachable by a
    // reader — the resolver test passes, the render test passes, and nobody is
    // told the queue exists. `NAV` is a plain array in this file, so the check has
    // to be that the link is actually rendered.
    mockShell(false);
    render(App);

    // The queue sits in the Library menu now, so the link is not rendered until
    // the menu opens. Asserting the link exists without opening the menu would
    // pass on the old header and fail here for the right reason.
    // By label, not by index: an index silently points at a different menu the
    // moment the groups are reordered, and the test would then pass for the wrong
    // reason.
    const shelf = screen
      .getAllByTestId('menu-trigger')
      .find((b) => b.textContent?.trim() === 'Shelf')!;
    await fireEvent.click(shelf);
    const link = screen.getAllByRole('link', { name: /your queue/i });
    expect(link.length).toBeGreaterThan(0);
    expect(link[0].getAttribute('href')).toBe('/concierge');
  });

  it('routes /concierge to the concierge page, not the 404 (spec §54)', async () => {
    // The gap this closes. `Concierge.svelte` shipped with 11 passing component
    // tests and no route, and every one of them stayed green — a component test
    // renders the component directly, so none of them ever asks whether
    // `/concierge` resolves or what the shell does with it. "The component works"
    // and "a reader can reach it" are different claims and only this is the second.
    //
    // So this renders the SHELL at that path, with the concierge's own API
    // answered. Two things can go wrong and this catches both: the resolver falls
    // through to not-found (no entry in `ROUTES`), or it matches but no
    // `{:else if route.id === 'concierge'}` branch exists so the shell renders
    // nothing. `router.test.ts` checks the first; only this checks the second.
    mockShell(true);
    const seen: string[] = [];
    const base = vi.spyOn(globalThis, 'fetch').mockImplementation((input) => {
      const url = String(typeof input === 'string' ? input : (input as Request).url);
      seen.push(url);
      if (url.includes('/health/ready')) return Promise.resolve(json(READY));
      if (url.includes('/auth/me')) return Promise.resolve(json(ME));
      if (url.includes('/me/concierge')) {
        return Promise.resolve(
          json({
            session_id: 'sess-1',
            items: [
              {
                work_id: 'work-1',
                title: 'A Work In The Queue',
                estimated_minutes: 20,
                reason: { kind: 'blend' },
              },
            ],
            estimated_minutes: 20,
            truncated_at: null,
            rate_source: 'default',
            explained_empty: null,
          }),
        );
      }
      return Promise.resolve(json(META));
    });

    window.history.pushState({}, '', '/concierge');
    render(App);

    // The page's own heading, which only `Concierge.svelte` renders. NotFound has
    // no such heading, so this is the assertion that the shell dispatched.
    await waitFor(() =>
      expect(screen.getAllByText('Your queue').length).toBeGreaterThan(0),
    );
    await waitFor(() =>
      expect(screen.getAllByText('A Work In The Queue').length).toBeGreaterThan(0),
    );

    // And it actually asked the concierge endpoint — a shell that rendered the
    // page with no data would still show the heading.
    expect(seen.some((u) => u.includes('/me/concierge'))).toBe(true);
    // §54.7's no-selector case: the first render must send NO mood and NO budget,
    // so a reader who has chosen nothing still sees their discovery feed.
    const call = seen.find((u) => u.includes('/me/concierge'));
    expect(call).toBeDefined();
    expect(call).not.toContain('mood=');
    expect(call).not.toContain('minutes=');

    base.mockRestore();
  });

  it('shows real instance values fetched from the API, not placeholders', async () => {
    mockShell(false);
    render(App);

    await waitFor(() => {
      expect(screen.getByText('0.1.0+abc1234')).toBeInTheDocument();
    });
    // Health is summarised on the landing page now rather than listed check by
    // check, so the assertion is the honest one: when everything passes, the
    // page says so in one line and names no individual check.
    //
    // The earlier version asserted `sqlite reachable` appeared. That was the
    // reason the landing page opened with a status panel, and a front door that
    // greets a visitor with a database connection string is the problem this
    // page was rewritten to fix.
    expect(await screen.findByText(/all services healthy/)).toBeInTheDocument();
    expect(screen.queryByText('sqlite reachable')).not.toBeInTheDocument();
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
