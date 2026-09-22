import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import History from './History.svelte';
import { session } from '../lib/session.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchHistory: vi.fn(),
    deleteHistoryEntry: vi.fn(),
    clearHistory: vi.fn(),
  };
});

import { fetchHistory } from '../lib/api';

const HISTORY = {
  items: [
    {
      id: 'h-1',
      subject_type: 'work',
      subject_id: 'work-1',
      last_read_at: '2026-09-21T12:00:00Z',
      title: 'Read Yesterday',
      authors: ['alice'],
    },
  ],
  next_cursor: null,
};

const EMPTY = { items: [], next_cursor: null };

beforeEach(() => {
  vi.clearAllMocks();
  session.status = 'signed-in';
  session.me = {
    id: 'acc-1',
    email: 'test@example.com',
    age_state: 'declared_adult',
    email_verified: true,
    pseuds: [{ id: 'pseud-1', handle: 'alice', display_name: 'Alice', bio: null }],
    active_pseud_id: 'pseud-1',
    capabilities: { can_read: true, can_write: true, can_message: true, can_be_listed: true, max_rating: 'explicit' },
  } as any;
  (fetchHistory as any).mockResolvedValue(HISTORY);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('History page', () => {
  it('renders reading history entries', async () => {
    render(History);
    await waitFor(() => expect(screen.getByText('Read Yesterday')).toBeInTheDocument());
  });

  it('shows the empty state when there is no history', async () => {
    (fetchHistory as any).mockResolvedValue(EMPTY);
    render(History);
    await waitFor(() => expect(screen.getByText(/You haven't read anything yet/)).toBeInTheDocument());
  });

  it('fetches the history on mount', async () => {
    render(History);
    await waitFor(() => expect(fetchHistory).toHaveBeenCalled());
  });
});
