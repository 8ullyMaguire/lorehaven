import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Vanguard from './Vanguard.svelte';

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
});
