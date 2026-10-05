import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * The nine-page sweep (docs/plans/REMAINING-2026-10-05.md §2a), as one test.
 *
 * Nine routes opened with `ErrorSummary` reading "That did not work /
 * authentication required" to a signed-out visitor who had done nothing wrong. The
 * pages differ in what they fetch; the defect was one rule applied nine times, so the
 * guard is one test that walks all nine rather than nine tests that each prove one
 * instance of a rule nobody wrote down.
 *
 * Each entry names the page, the call that used to 401, and — the part that is
 * actually load-bearing — the fetch mock. A guard that only rendered the page would
 * pass against a component that guards its template but still fires the request,
 * which is a 401 in the console and a red panel on any page that renders the error
 * before the session resolves.
 *
 * Why the assertion is the ABSENCE of a heading rather than the presence of a note:
 * the note is a wording choice, and a guard on wording rots the moment someone
 * improves the sentence. "That did not work" is `ErrorSummary`'s `<h3>`, and it is
 * the exact string that was on screen on all nine pages.
 */

/** Every page the sweep found, with the one call each used to make while signed out. */
const PAGES: Array<{ name: string; path: string; fetcher: string }> = [
  { name: 'BlindDate', path: './BlindDate.svelte', fetcher: 'fetchBlindDate' },
  { name: 'SurpriseMe', path: './SurpriseMe.svelte', fetcher: 'fetchSurpriseMe' },
  { name: 'Arena', path: './Arena.svelte', fetcher: 'fetchArenaNext' },
  { name: 'Notifications', path: './Notifications.svelte', fetcher: 'fetchNotifications' },
  { name: 'Concierge', path: './Concierge.svelte', fetcher: 'fetchConciergeQueue' },
  { name: 'Quiz', path: './Quiz.svelte', fetcher: 'fetchQuizWorks' },
  { name: 'Vanguard', path: './Vanguard.svelte', fetcher: 'fetchVanguardStatus' },
  { name: 'Community', path: './Community.svelte', fetcher: 'fetchForums' },
  {
    name: 'AdminEconomyFlows',
    path: './AdminEconomyFlows.svelte',
    fetcher: 'fetchEconomyFlows',
  },
];

/**
 * Mock the whole API module.
 *
 * Every function becomes a `vi.fn()` that rejects, so a page that fetches anyway
 * fails loudly and the assertion can tell "guarded" from "guarded in the template
 * but still fetching". `ApiError` and the types are kept real, because pages branch
 * on `instanceof ApiError` and a fake class would change which branch they take.
 */
vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  /**
   * One stub per export, MEMOISED.
   *
   * The first version of this built a fresh `vi.fn()` on every property read, and
   * that made the "was it called?" assertion below a tautology: the test asked a
   * different mock object than the component had imported, so it could never have
   * been called. Removing the guard from a page's effect left this file green —
   * a guard that cannot fail is worse than no guard, because it reads as evidence.
   *
   * Caught by mutation rather than by reading, which is the only way this shape of
   * mistake ever gets caught.
   */
  const stubs = new Map<string, unknown>();
  return new Proxy(actual, {
    get(target, prop: string) {
      const isDataCall =
        (prop.startsWith('fetch') || prop.startsWith('get') || prop.startsWith('mark')) &&
        prop in target;
      if (!isDataCall) return target[prop as keyof typeof target];
      const existing = stubs.get(prop);
      if (existing) return existing;
      const stub = vi.fn(async () => {
        throw new actual.ApiError(401, 'AUTH_REQUIRED', 'authentication required');
      });
      stubs.set(prop, stub);
      return stub;
    },
  });
});

// `signOutTestReader` sets the store to anonymous BEFORE each render, so the
// components under test see the state a signed-out visitor is in — which is the
// only state this file is about.
import { resetTestSession, signOutTestReader } from '../lib/testing/session';

describe('a signed-out visitor is never told something went wrong', () => {
  beforeEach(() => {
    signOutTestReader();
    vi.clearAllMocks();
  });

  afterEach(() => {
    resetTestSession();
  });

  for (const page of PAGES) {
    it(`${page.name} shows a sign-in note and asks the server nothing`, async () => {
      // The page is imported lazily so each case exercises one module, and so a
      // page that fails to construct fails HERE rather than at collection time
      // taking the other eight with it.
      const mod = (await import(/* @vite-ignore */ page.path)) as Record<string, unknown>;
      const Component = mod.default as Parameters<typeof render>[0];
      if (!Component) {
        throw new Error(`${page.path} has no default export`);
      }

      render(Component as never);

      await waitFor(() => expect(screen.getByTestId('signin-note')).toBeInTheDocument());
      // The defect's exact on-screen form.
      expect(screen.queryByText('That did not work')).toBeNull();

      // The other half: nothing was requested. `signIn` is the one function the
      // shell legitimately uses, and the session probe is `fetchMe` — neither is
      // `${page.fetcher}`, and the mock would have rejected if it had been called.
      const api = (await import('../lib/api')) as unknown as Record<string, unknown>;
      const fetcher = api[page.fetcher] as ReturnType<typeof vi.fn>;
      expect(fetcher, `${page.fetcher} is missing from the mocked api`).toBeDefined();
      // Memoisation in the mock above is what makes this an assertion rather than
      // a comment: it is the same function object the component imported.
      expect(fetcher).not.toHaveBeenCalled();
    });
  }

  it('the sweep covers the nine routes the plan named', () => {
    // An absence test needs its subject counted. Without this, deleting an entry
    // from `PAGES` would silently shrink the guard while it still reported green.
    expect(PAGES).toHaveLength(9);
    expect(new Set(PAGES.map((p) => p.name)).size).toBe(9);
    // And each one names a fetcher, because an entry with none would assert only
    // that the note rendered.
    for (const page of PAGES) {
      expect(page.fetcher, `${page.name} names no fetcher`).toMatch(/^(fetch|get|mark)/);
    }
  });
});
