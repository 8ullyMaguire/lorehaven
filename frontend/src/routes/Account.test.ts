import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Account from './Account.svelte';
import { session } from '../lib/session.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchSessions: vi.fn().mockResolvedValue([]),
    fetchPrivacy: vi.fn().mockResolvedValue({
      show_in_directory: true,
      show_reading_history: false,
      show_library: true,
    }),
    fetchContentSettings: vi.fn().mockResolvedValue({
      blur_spoilers: true,
      hide_warnings: false,
      max_rating: 'explicit',
    }),
    fetchFeedbackInbox: vi.fn().mockResolvedValue({ items: [], next_cursor: null }),
    fetchFeedbackPreferences: vi.fn().mockResolvedValue({ allow_from: 'everyone' }),
    fetchMyEarnings: vi.fn().mockResolvedValue({ rows: [], total: 0 }),
    patchContentSettings: vi.fn(),
    patchPrivacy: vi.fn(),
    putFeedbackPreferences: vi.fn(),
    revokeAllSessions: vi.fn(),
    revokeSession: vi.fn(),
  };
});

beforeEach(() => {
  session.status = 'signed-in';
  session.me = {
    id: 'acc-1',
    account: { email: 'test@example.com' },
    age_state: 'declared_adult',
    email_verified: true,
    pseuds: [{ id: 'pseud-1', handle: 'alice', display_name: 'Alice', bio: null }],
    active_pseud_id: 'pseud-1',
    capabilities: { can_read: true, can_write: true, can_message: true, can_be_listed: true, max_rating: 'explicit' },
  } as any;
  vi.clearAllMocks();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('Account page', () => {
  it('renders the account heading when signed in', async () => {
    render(Account);
    await waitFor(() => expect(screen.getAllByText('Your account').length).toBeGreaterThan(0));
  });

  it('shows the sign-in prompt when signed out', async () => {
    session.status = 'anonymous';
    session.me = null;
    render(Account);
    await waitFor(() => expect(screen.getByText('Sign in to see this page')).toBeInTheDocument());
  });
});
