import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Pseuds from './Pseuds.svelte';
import { session } from '../lib/session.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchPseuds: vi.fn(),
    fetchPrivacy: vi.fn(),
    createPseud: vi.fn(),
    updatePseud: vi.fn(),
    patchPrivacy: vi.fn(),
  };
});

import { fetchPseuds, fetchPrivacy } from '../lib/api';

const PSEUDS = [
  {
    id: 'pseud-1',
    handle: 'alice',
    display_name: 'Alice',
    bio: 'Writer',
    discoverability: 'listed',
    version: 1,
    created_at: '2026-01-01T00:00:00Z',
    active: true,
  },
];

const PRIVACY = {
  account: {},
  pseuds: { 'pseud-1': { show_in_directory: 'true', show_reading_history: 'false' } },
  schema: [],
};

beforeEach(() => {
  session.status = 'signed-in';
  session.me = {
    id: 'acc-1',
    email: 'test@example.com',
    age_state: 'declared_adult',
    email_verified: true,
    pseuds: PSEUDS,
    active_pseud_id: 'pseud-1',
    capabilities: { can_read: true, can_write: true, can_message: true, can_be_listed: true, max_rating: 'explicit' },
  } as any;
  vi.clearAllMocks();
  (fetchPseuds as any).mockResolvedValue(PSEUDS);
  (fetchPrivacy as any).mockResolvedValue(PRIVACY);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('Pseuds page', () => {
  it('renders the pseuds heading', async () => {
    render(Pseuds);
    await waitFor(() => expect(screen.getByText('Your pseuds')).toBeInTheDocument());
  });

  it('shows existing pseuds', async () => {
    render(Pseuds);
    await waitFor(() => expect(screen.getByText('Alice')).toBeInTheDocument());
  });

  it('shows a create pseud form', async () => {
    render(Pseuds);
    await waitFor(() => expect(screen.getByText('Your pseuds')).toBeInTheDocument());
    expect(screen.getByText('Another pseud')).toBeInTheDocument();
  });

  it('fetches pseuds on mount', async () => {
    render(Pseuds);
    await waitFor(() => expect(fetchPseuds).toHaveBeenCalled());
  });
});
