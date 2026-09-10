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

beforeEach(() => {
  window.localStorage.clear();
  document.documentElement.removeAttribute('data-theme');

  vi.spyOn(globalThis, 'fetch').mockImplementation((input) => {
    const url = typeof input === 'string' ? input : (input as Request).url;
    const body = url.includes('/health/ready') ? READY : META;
    return Promise.resolve(
      new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    );
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  window.localStorage.clear();
});

describe('application shell', () => {
  it('renders the wordmark and the full desktop navigation', () => {
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
    render(App);

    await waitFor(() => {
      expect(screen.getByText('0.1.0+abc1234')).toBeInTheDocument();
    });
    // The health panel is rendered from the readiness response.
    expect(await screen.findByText('sqlite reachable')).toBeInTheDocument();
    expect(screen.getByText('1 migration(s) applied')).toBeInTheDocument();
  });

  it('applies and persists an appearance choice', async () => {
    render(App);

    const select = screen.getByLabelText('Appearance') as HTMLSelectElement;
    await fireEvent.change(select, { target: { value: 'clear-day' } });

    // Applied immediately…
    expect(document.documentElement.dataset.theme).toBe('clear-day');
    // …and remembered, which is the part that regressed.
    expect(localStorage.getItem('lorehaven.theme')).toBe('clear-day');
  });

  it('honours a stored preference on load', () => {
    window.localStorage.setItem('lorehaven.theme', 'after-hours');
    render(App);
    expect(document.documentElement.dataset.theme).toBe('after-hours');
  });

  it('marks the current destination for assistive technology', async () => {
    window.history.pushState({}, '', '/library');
    render(App);
    window.dispatchEvent(new PopStateEvent('popstate'));

    await waitFor(() => {
      const current = screen
        .getAllByText('Library')
        .filter((node) => node.getAttribute('aria-current') === 'page');
      expect(current.length).toBeGreaterThan(0);
    });

    window.history.pushState({}, '', '/');
  });
});
