import { render, screen, fireEvent, waitFor, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Settings from './Settings.svelte';
import { session } from '../lib/session.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchSearchSettings: vi.fn().mockResolvedValue({
      pseud_id: 'test-pseud',
      settings: [
        { key: 'min_words', value: 1000, source: 'account', summary: 'Minimum word count' },
        { key: 'sort', value: 'relevance', source: 'instance', summary: 'Default sort order' },
      ],
      schema: [],
    }),
    fetchContentFilters: vi.fn().mockResolvedValue({
      pseud_id: 'test-pseud',
      filters: [
        { filter_type: 'tag', value: 'romance' },
        { filter_type: 'warning', value: 'Graphic Violence' },
      ],
    }),
    fetchNotificationRoutes: vi.fn().mockResolvedValue({
      account_id: 'test-account',
      routes: [
        { event_type: 'work_published', channel: 'email', enabled: true },
        { event_type: 'comment_received', channel: 'in_app', enabled: false },
      ],
    }),
    patchSearchSettings: vi.fn(),
    deleteSearchSetting: vi.fn(),
    addContentFilter: vi.fn(),
    deleteContentFilter: vi.fn(),
    patchNotificationRoutes: vi.fn(),
    deleteNotificationRoute: vi.fn(),
    exportSettings: vi.fn(),
    importSettings: vi.fn(),
    fetchTokens: vi.fn().mockResolvedValue([]),
    createToken: vi.fn(),
    revokeToken: vi.fn(),
  };
});

/** A token as `GET /me/tokens` returns it. */
const token = (over: Partial<Record<string, unknown>> = {}) => ({
  id: 'tok-1',
  account_id: 'acct-1',
  name: 'nightly build',
  kind: 'user',
  scopes: 'content.read library.read',
  acting_pseud_id: 'pseud-1',
  created_at: '2026-01-02T03:04:05Z',
  last_used_at: '2026-02-03T04:05:06Z',
  expires_at: null,
  revoked_at: null,
  ...over,
});

beforeEach(async () => {
  // `vi.clearAllMocks()` in `afterEach` wipes the *implementation* of the
  // factory-defined `fetchTokens` mock as well as its call count, so every test
  // that mounts Settings would otherwise see `undefined` from it. The panel
  // then renders "no linked applications" — a real, plausible state that makes
  // a broken mock look like an empty account.
  const { fetchTokens, fetchSearchSettings } = await import('../lib/api');
  vi.mocked(fetchTokens).mockResolvedValue([]);
  // Same problem, and pre-existing until now: `handles loading error gracefully`
  // rejects `fetchSearchSettings`, and `vi.clearAllMocks()` does not put the
  // factory's resolved value back. Every test after it mounted a page in the
  // error state. Restored here rather than in that one test, because the
  // invariant is "each test starts from a working API" — which is a property of
  // the suite, not of one test that happens to notice.
  vi.mocked(fetchSearchSettings).mockResolvedValue({
    pseud_id: 'test-pseud',
    settings: [
      { key: 'min_words', value: 1000, source: 'account', summary: 'Minimum word count' },
      { key: 'sort', value: 'relevance', source: 'instance', summary: 'Default sort order' },
    ],
    schema: [],
  } as any);

  session.status = 'signed-in';
  session.me = {
    account: { email: 'test@example.com' },
    pseuds: [],
    active_pseud_id: null,
    capabilities: { can_read: true, can_write: true, can_message: true, can_be_listed: true, max_rating: 'general' },
  } as any;
});

afterEach(() => {
  session.status = 'unknown';
  session.me = null;
  vi.clearAllMocks();
});

describe('Settings', () => {
  it('renders all three tabs', () => {
    render(Settings);
    expect(screen.getByText('Search Defaults')).toBeTruthy();
    expect(screen.getByText('Content Filters')).toBeTruthy();
    expect(screen.getByText('Notifications')).toBeTruthy();
  });

  it('loads and displays search settings after mount', async () => {
    render(Settings);
    await waitFor(() => {
      expect(screen.getByText('min_words')).toBeTruthy();
    });
  });

  it('shows notification routes with channel and status', async () => {
    render(Settings);
    await fireEvent.click(screen.getByText('Notifications'));
    await waitFor(() => {
      expect(screen.getAllByText('work_published').length).toBeGreaterThan(0);
    });
  });

  it('displays content filters with type and value', async () => {
    render(Settings);
    await fireEvent.click(screen.getByText('Content Filters'));
    await waitFor(() => {
      expect(screen.getByText('romance')).toBeTruthy();
    });
  });

  it('supports client-side settings search', async () => {
    render(Settings);
    await waitFor(() => screen.getByText('min_words'));
    const searchInput = screen.getByPlaceholderText('Search settings…');
    await fireEvent.input(searchInput, { target: { value: 'min_words' } });
    expect(screen.getByText('min_words')).toBeTruthy();
    expect(screen.queryByText('sort')).toBeFalsy();
  });

  it('allows adding a new search default', async () => {
    const { patchSearchSettings } = await import('../lib/api');
    vi.mocked(patchSearchSettings).mockResolvedValue({
      pseud_id: 'test-pseud',
      settings: [
        { key: 'max_rating', value: 'teen', source: 'account', summary: 'Max rating' },
      ],
      schema: [],
    });

    render(Settings);
    await waitFor(() => screen.getByText('Add'));

    const keyInput = screen.getByPlaceholderText('Key (e.g. min_words)');
    const valueInput = screen.getByPlaceholderText('Value (JSON)');
    await fireEvent.input(keyInput, { target: { value: 'max_rating' } });
    await fireEvent.input(valueInput, { target: { value: '"teen"' } });
    await fireEvent.click(screen.getByText('Add'));

    expect(patchSearchSettings).toHaveBeenCalledWith([
      { key: 'max_rating', value: 'teen' },
    ]);
  });

  it('handles loading error gracefully', async () => {
    const { fetchSearchSettings } = await import('../lib/api');
    vi.mocked(fetchSearchSettings).mockRejectedValue(new Error('Network error'));

    render(Settings);
    await waitFor(() => {
      expect(screen.getByText('That did not work')).toBeTruthy();
    });
  });

  it('shows export/import controls', () => {
    render(Settings);
    expect(screen.getByText('Export')).toBeTruthy();
    expect(screen.getByText('Import')).toBeTruthy();
  });

  // -------------------------------------------------------------------------
  // Linked applications (spec §23.1)
  // -------------------------------------------------------------------------

  it('lists tokens with their scopes and last use', async () => {
    const { fetchTokens } = await import('../lib/api');
    vi.mocked(fetchTokens).mockResolvedValue([token()] as any);

    render(Settings);
    await fireEvent.click(screen.getByText('Linked applications'));
    await waitFor(() => {
      expect(screen.getByText('nightly build')).toBeTruthy();
    });
    // Scopes are shown as the two they are, not as the stored string — and
    // scoped to the list, because the create form above it offers the same
    // names. A bare `getByText('content.read')` matches both and fails, which
    // is a nuisance; a bare `getAllByText(...).length` would pass without
    // checking either, which is worse.
    const list = screen.getByRole('list');
    expect(within(list).getByText('content.read')).toBeTruthy();
    expect(within(list).getByText('library.read')).toBeTruthy();
    expect(screen.getByText(/last used 2026-02-03/)).toBeTruthy();
  });

  it('says when a token has never been used', async () => {
    const { fetchTokens } = await import('../lib/api');
    vi.mocked(fetchTokens).mockResolvedValue([token({ last_used_at: null })] as any);

    render(Settings);
    await fireEvent.click(screen.getByText('Linked applications'));
    await waitFor(() => {
      expect(screen.getByText(/never used/)).toBeTruthy();
    });
  });

  it('refuses to create a token with no permissions selected', async () => {
    const { createToken } = await import('../lib/api');

    render(Settings);
    await fireEvent.click(screen.getByText('Linked applications'));
    await waitFor(() => screen.getByLabelText('Token name'));

    // Name filled, no scope: the button must stay disabled, and no call made.
    await fireEvent.input(screen.getByLabelText('Token name'), {
      target: { value: 'my script' },
    });
    expect(screen.getByText('Create token').hasAttribute('disabled')).toBe(true);
    expect(screen.getByText('Pick at least one permission.')).toBeTruthy();
    expect(vi.mocked(createToken)).not.toHaveBeenCalled();
  });

  it('creates a token once a permission is picked and shows it once', async () => {
    const { createToken, fetchTokens } = await import('../lib/api');
    vi.mocked(createToken).mockResolvedValue({ token: 'lh-secret-value', id: 'tok-2' });
    vi.mocked(fetchTokens)
      .mockResolvedValueOnce([] as any)
      .mockResolvedValue([token({ id: 'tok-2', name: 'my script' })] as any);

    render(Settings);
    await fireEvent.click(screen.getByText('Linked applications'));
    await waitFor(() => screen.getByLabelText('Token name'));
    await fireEvent.input(screen.getByLabelText('Token name'), {
      target: { value: 'my script' },
    });
    await fireEvent.click(screen.getByLabelText('content.read'));
    await fireEvent.click(screen.getByText('Create token'));

    await waitFor(() => {
      expect(vi.mocked(createToken)).toHaveBeenCalledWith('my script', ['content.read']);
    });
    // The raw value is on screen, and the panel says it is the only time.
    expect(screen.getByText('lh-secret-value')).toBeTruthy();
    expect(screen.getByText(/cannot be shown again/)).toBeTruthy();
  });

  it('warns that a token with no acting pseud will be refused', async () => {
    // Not a hypothetical: A6 refuses such a token, and a reader who wired one
    // up without knowing would debug it against the wrong thing.
    const { fetchTokens } = await import('../lib/api');
    vi.mocked(fetchTokens).mockResolvedValue([token({ acting_pseud_id: null })] as any);

    render(Settings);
    await fireEvent.click(screen.getByText('Linked applications'));
    await waitFor(() => {
      expect(screen.getByText(/no acting pseud/)).toBeTruthy();
    });
  });

  it('marks a token bound to a bot', async () => {
    const { fetchTokens } = await import('../lib/api');
    vi.mocked(fetchTokens).mockResolvedValue([token({ kind: 'bot', name: 'lorebot' })] as any);

    render(Settings);
    await fireEvent.click(screen.getByText('Linked applications'));
    await waitFor(() => {
      expect(screen.getByText('lorebot')).toBeTruthy();
    });
    expect(screen.getByText('bot')).toBeTruthy();
  });

  it('drops a revoked token from the list', async () => {
    const { fetchTokens, revokeToken } = await import('../lib/api');
    vi.mocked(fetchTokens).mockResolvedValue([token()] as any);
    vi.mocked(revokeToken).mockResolvedValue(undefined);

    render(Settings);
    await fireEvent.click(screen.getByText('Linked applications'));
    await waitFor(() => screen.getByText('nightly build'));
    await fireEvent.click(screen.getByText('Revoke'));

    await waitFor(() => {
      expect(vi.mocked(revokeToken)).toHaveBeenCalledWith('tok-1');
    });
    await waitFor(() => {
      expect(screen.queryByText('nightly build')).toBeNull();
    });
  });

  it('reports a token list failure without breaking the other tabs', async () => {
    // The failure is scoped to its own panel on purpose: blanking four working
    // tabs would make a token-list problem look like a session problem.
    const { fetchTokens } = await import('../lib/api');
    vi.mocked(fetchTokens).mockRejectedValue(new Error('token service is down'));

    render(Settings);
    await fireEvent.click(screen.getByText('Linked applications'));
    await waitFor(() => {
      expect(screen.getByText('token service is down')).toBeTruthy();
    });
    await fireEvent.click(screen.getByText('Search Defaults'));
    await waitFor(() => {
      expect(screen.getByText('min_words')).toBeTruthy();
    });
  });
});
