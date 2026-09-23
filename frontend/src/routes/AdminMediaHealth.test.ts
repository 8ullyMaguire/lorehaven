import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import AdminMediaHealth from './AdminMediaHealth.svelte';
import { session } from '../lib/session.svelte';

vi.mock('../lib/api', () => ({
  fetchMediaHealthOverview: vi.fn(async () => ({
    total_references: 42,
    well_mirrored: 30,
    below_threshold: 12,
    health_pct: 71.4,
    min_healthy_threshold: 3,
    recent_rescues_7d: 5,
    one_week_ago: '2026-09-16T00:00:00Z',
  })),
  fetchLinkRotReport: vi.fn(async () => ({
    since: '2026-09-16T00:00:00Z',
    total_rot: 8,
    by_provider: [
      { provider: 'pinterest', dead: 5 },
      { provider: 'tumblr', dead: 3 },
    ],
  })),
  fetchCuratorLeaderboard: vi.fn(async () => ({
    curators: [
      { account_id: 'acc-001', total_rewards: 150, actions: 25 },
      { account_id: 'acc-002', total_rewards: 100, actions: 18 },
    ],
  })),
  fetchBountyStatus: vi.fn(async () => ({
    active_bounties: 7,
    total_available: 2500,
  })),
  fetchStorageStatus: vi.fn(async () => ({
    local_mirrors: 423,
    total_bytes: 1073741824,
    ipfs_pins: 18,
  })),
  fetchProviderReliability: vi.fn(async () => ({
    providers: [
      { provider: 'imgur', healthy: 95, total: 100, health_rate: 0.95 },
      { provider: 'pinterest', healthy: 80, total: 100, health_rate: 0.8 },
    ],
  })),
}));

beforeEach(() => {
  vi.clearAllMocks();
  session.status = 'signed-in';
  session.me = {
    id: 'acc-1',
    email: 'admin@example.com',
    age_state: 'declared_adult',
    email_verified: true,
    pseuds: [{ id: 'pseud-1', handle: 'admin', display_name: 'Admin', bio: null }],
    active_pseud_id: 'pseud-1',
    capabilities: { can_read: true, can_write: true, can_message: true, can_be_listed: true, max_rating: 'explicit' },
  } as any;
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('AdminMediaHealth page', () => {
  it('renders the page heading when signed in', async () => {
    render(AdminMediaHealth);
    await waitFor(() => expect(screen.getByText('Media health')).toBeInTheDocument());
  });

  it('fetches all six metric panels on mount', async () => {
    render(AdminMediaHealth);
    await waitFor(() => expect(screen.getByText('Media health')).toBeInTheDocument());
    await waitFor(() => expect(screen.getByText('Overall health')).toBeInTheDocument());
    await waitFor(() => expect(screen.getByText('Link rot')).toBeInTheDocument());
    await waitFor(() => expect(screen.getByText('Standing bounties')).toBeInTheDocument());
    await waitFor(() => expect(screen.getByText('Storage')).toBeInTheDocument());
    await waitFor(() => expect(screen.getByText('Provider reliability')).toBeInTheDocument());
    await waitFor(() => expect(screen.getByText('Curator leaderboard')).toBeInTheDocument());
  });

  it('renders metric values from the API response', async () => {
    render(AdminMediaHealth);
    await waitFor(() => expect(screen.getByText('Media health')).toBeInTheDocument());
    await waitFor(() => expect(screen.getByText('71.4%')).toBeInTheDocument());
    await waitFor(() => expect(screen.getByText('42')).toBeInTheDocument());
    await waitFor(() => expect(screen.getByText('7')).toBeInTheDocument());
    await waitFor(() => expect(screen.getByText('1.00 GB')).toBeInTheDocument());
  });

  it('shows curator leaderboard entries', async () => {
    render(AdminMediaHealth);
    await waitFor(() => expect(screen.getByText('Media health')).toBeInTheDocument());
    await waitFor(() => expect(screen.getByText('acc-001')).toBeInTheDocument());
    await waitFor(() => expect(screen.getByText('150')).toBeInTheDocument());
  });
});
