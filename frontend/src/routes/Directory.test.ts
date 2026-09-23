import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Directory from './Directory.svelte';

vi.mock('../lib/api', () => ({
  fetchDirectoryEntries: vi.fn(),
  fetchDirectoryCategories: vi.fn(),
  submitDirectoryEntry: vi.fn(),
  voteDirectoryEntry: vi.fn(),
  approveDirectoryEntry: vi.fn(),
  removeDirectoryEntry: vi.fn(),
  fetchModerationQueue: vi.fn(),
}));

// Mock the governance component so the test doesn't need to resolve its
// transitive imports (which pull in session.svelte.ts and other .svelte files
// that fail in the test environment).
vi.mock('../lib/components/DirectoryGovernance.svelte', () => ({
  default: () => ({}),
}));

import {
  fetchDirectoryEntries,
  fetchDirectoryCategories,
  submitDirectoryEntry,
  fetchModerationQueue,
} from '../lib/api';

const CATEGORIES = {
  items: [
    { category: 'Writing', approved_count: 5 },
    { category: 'Research', approved_count: 3 },
  ],
};

const ENTRIES = {
  items: [
    {
      id: 'entry-1',
      title: 'A Writing Tool',
      url: 'https://example.com',
      description: 'Useful for writers',
      category: 'Writing',
      tags: ['writing'],
      score: 10,
      my_vote: null,
      submitted_by: 'alice',
      approved_by: 'operator-1',
    },
  ],
};

const EMPTY = { items: [] };

beforeEach(() => {
  vi.clearAllMocks();
  (fetchDirectoryCategories as any).mockResolvedValue(CATEGORIES);
  (fetchDirectoryEntries as any).mockResolvedValue(ENTRIES);
  (submitDirectoryEntry as any).mockResolvedValue({ entry: ENTRIES.items[0] });
  (fetchModerationQueue as any).mockResolvedValue(EMPTY);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('Resource Directory page', () => {
  it('renders directory entries from the API', async () => {
    render(Directory);
    await waitFor(() => expect(screen.getByText('A Writing Tool')).toBeInTheDocument());
  });

  it('shows category filter options', async () => {
    render(Directory);
    await waitFor(() => expect(screen.getByText('Writing (5)')).toBeInTheDocument());
    expect(screen.getByText('Research (3)')).toBeInTheDocument();
  });

  it('shows empty state when no entries', async () => {
    (fetchDirectoryEntries as any).mockResolvedValue(EMPTY);
    render(Directory);
    await waitFor(() => expect(screen.getByText(/No entries yet/)).toBeInTheDocument());
  });

  it('renders entry score', async () => {
    render(Directory);
    await waitFor(() => expect(screen.getByText('10')).toBeInTheDocument());
  });

  it('shows submitted-by metadata', async () => {
    render(Directory);
    await waitFor(() => expect(screen.getByText(/Submitted by alice/)).toBeInTheDocument());
  });

  it('shows pending badge for unapproved entries', async () => {
    (fetchDirectoryEntries as any).mockResolvedValue({
      items: [{ ...ENTRIES.items[0], approved_by: null }],
    });
    render(Directory);
    await waitFor(() => expect(screen.getByText('Pending review')).toBeInTheDocument());
  });
});