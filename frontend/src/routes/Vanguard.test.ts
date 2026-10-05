import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Vanguard from './Vanguard.svelte';
import {
  resetTestSession,
  signInTestReader,
  signOutTestReader,
} from '../lib/testing/session';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchVanguardStatus: vi.fn(),
    fetchVanguards: vi.fn(),
    fetchPinsForWork: vi.fn(),
    fetchMyStreak: vi.fn(),
  };
});

import { fetchVanguardStatus, fetchVanguards, fetchPinsForWork, fetchMyStreak } from '../lib/api';

beforeEach(() => {
  signInTestReader();
  vi.clearAllMocks();
  (fetchVanguardStatus as any).mockResolvedValue({ is_vanguard: true });
  (fetchVanguards as any).mockResolvedValue({ vanguards: ['alice', 'bob'] });
  (fetchPinsForWork as any).mockResolvedValue({ pins: [] });
  (fetchMyStreak as any).mockResolvedValue({
    current_streak: 5,
    longest_streak: 10,
    last_read_date: '2026-09-22',
  });
});

afterEach(() => {
  resetTestSession();
  vi.restoreAllMocks();
});

describe('Vanguard dashboard', () => {
  it('shows vanguard badge for vanguard users', async () => {
    render(Vanguard);
    await waitFor(() => expect(screen.getByText('You are a Vanguard')).toBeInTheDocument());
  });

  it('shows not-vanguard message for non-vanguards', async () => {
    (fetchVanguardStatus as any).mockResolvedValue({ is_vanguard: false });
    render(Vanguard);
    await waitFor(() => expect(screen.getByText(/You are not a Vanguard/)).toBeInTheDocument());
  });

  it('displays current vanguards list', async () => {
    render(Vanguard);
    await waitFor(() => expect(screen.getByText('alice')).toBeInTheDocument());
    expect(screen.getByText('bob')).toBeInTheDocument();
  });

  it('shows reading streak', async () => {
    render(Vanguard);
    await waitFor(() => expect(screen.getByText('5')).toBeInTheDocument());
    expect(screen.getByText('day streak')).toBeInTheDocument();
  });

  it('shows pin form button for vanguards', async () => {
    render(Vanguard);
    await waitFor(() => expect(screen.getByText('Pin a work')).toBeInTheDocument());
  });

  // -----------------------------------------------------------------------
  // The defect this page had: one operator-only call in a `Promise.all`, and
  // one shared `error` slot, so a 403 from the roster took the whole page down.
  // -----------------------------------------------------------------------

  it('renders for a normal account when the operator-only roster refuses', async () => {
    // 403 `ACCESS_DENIED` — exactly what `GET /api/v1/vanguards` answers to any
    // account that is not the operator (`vanguard.rs:136-145`).
    (fetchVanguards as any).mockRejectedValue(
      Object.assign(new Error('access denied'), { status: 403, code: 'ACCESS_DENIED' }),
    );

    render(Vanguard);

    // The reader's own three panels are all still there...
    await waitFor(() => expect(screen.getByText('You are a Vanguard')).toBeInTheDocument());
    expect(screen.getByText('day streak')).toBeInTheDocument();
    expect(screen.getByText('Pin a work')).toBeInTheDocument();
    // ...the roster is simply absent, with no apology for it...
    expect(screen.queryByText('Current vanguards')).toBeNull();
    // ...and, the load-bearing part, NO red panel anywhere on the page.
    expect(screen.queryByText('That did not work')).toBeNull();
  });

  it('still reports a genuine failure of the roster', async () => {
    // 500 is not "not for you" — it is a fault, and swallowing it would leave the
    // operator with a silently missing roster.
    (fetchVanguards as any).mockRejectedValue(
      Object.assign(new Error('boom'), { status: 500, code: 'INTERNAL' }),
    );

    render(Vanguard);

    await waitFor(() => expect(screen.getByText('That did not work')).toBeInTheDocument());
  });

  it('offers a sign-in note rather than a failure to a signed-out visitor', async () => {
    signOutTestReader();

    render(Vanguard);

    await waitFor(() => expect(screen.getByTestId('signin-note')).toBeInTheDocument());
    expect(screen.queryByText('That did not work')).toBeNull();
    // And nothing was asked of the server, which is the other half of the fix: a
    // 401 the page could have predicted is a 401 the page must not cause.
    expect(fetchVanguardStatus).not.toHaveBeenCalled();
  });
});
