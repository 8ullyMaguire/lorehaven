import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';
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
  };
});

beforeEach(() => {
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
});
