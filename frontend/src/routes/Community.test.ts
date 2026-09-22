import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Community from './Community.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchForums: vi.fn(),
    fetchGroups: vi.fn(),
    fetchConversations: vi.fn(),
    fetchBlocks: vi.fn(),
  };
});

import { fetchForums, fetchGroups, fetchConversations, fetchBlocks } from '../lib/api';

const FORUMS = [
  { id: 'forum-1', name: 'General', description: 'General chat', topic_count: 10 },
];

const GROUPS = [
  { id: 'group-1', name: 'Beta Readers', description: 'Find a beta', visibility: 'public' },
];

beforeEach(() => {
  vi.clearAllMocks();
  (fetchForums as any).mockResolvedValue(FORUMS);
  (fetchGroups as any).mockResolvedValue(GROUPS);
  (fetchConversations as any).mockResolvedValue([]);
  (fetchBlocks as any).mockResolvedValue([]);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('Community page', () => {
  it('renders forum tabs', async () => {
    render(Community);
    expect(screen.getByRole('tab', { name: 'Forums' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Groups' })).toBeInTheDocument();
  });

  it('loads forums on mount', async () => {
    render(Community);
    await waitFor(() => expect(fetchForums).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByText('General')).toBeInTheDocument());
  });

  it('switches to groups tab', async () => {
    render(Community);
    await waitFor(() => expect(screen.getByText('General')).toBeInTheDocument());
    await screen.getByRole('tab', { name: 'Groups' }).click();
    await waitFor(() => expect(fetchGroups).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByText('Beta Readers')).toBeInTheDocument());
  });
});
