import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
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
  voteDirectoryEntry,
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

  // --- Vote decay and the un-vote control -------------------------------
  //
  // Voting the same direction twice now refreshes the vote rather than
  // toggling it off, so the UI cannot offer "click again to undo". A voter
  // who wants to retract needs a separate, explicit control, and the two
  // states have to be distinguishable by more than colour.

  it('offers an explicit un-vote control when the viewer has voted', async () => {
    (fetchDirectoryEntries as any).mockResolvedValue({
      items: [{ ...ENTRIES.items[0], my_vote: 1 }],
    });
    render(Directory);
    await waitFor(() => expect(screen.getByText('A Writing Tool')).toBeInTheDocument());
    expect(screen.getByRole('button', { name: /remove your vote/i })).toBeInTheDocument();
  });

  it('offers no un-vote control when the viewer has not voted', async () => {
    (fetchDirectoryEntries as any).mockResolvedValue({
      items: [{ ...ENTRIES.items[0], my_vote: null }],
    });
    render(Directory);
    await waitFor(() => expect(screen.getByText('A Writing Tool')).toBeInTheDocument());
    expect(screen.queryByRole('button', { name: /remove your vote/i })).toBeNull();
  });

  it('sends value 0 when the un-vote control is used', async () => {
    (fetchDirectoryEntries as any).mockResolvedValue({
      items: [{ ...ENTRIES.items[0], my_vote: 1 }],
    });
    (voteDirectoryEntry as any).mockResolvedValue({ score: 9, my_vote: null });
    render(Directory);
    await waitFor(() => expect(screen.getByText('A Writing Tool')).toBeInTheDocument());
    const button = screen.getByRole('button', { name: /remove your vote/i });
    await fireEvent.click(button);
    await waitFor(() =>
      expect(voteDirectoryEntry).toHaveBeenCalledWith('entry-1', 0),
    );
  });

  it('says so when a vote on this entry is not on a clock', async () => {
    (fetchDirectoryEntries as any).mockResolvedValue({
      items: [
        {
          ...ENTRIES.items[0],
          my_vote: 1,
          decay: { enabled: true, cutoff_days: 60, applies_to_this_entry: false },
        },
      ],
    });
    render(Directory);
    await waitFor(() => expect(screen.getByText('A Writing Tool')).toBeInTheDocument());
    // Below the activation threshold, so the vote is permanent. Saying so is
    // the difference between "this number is what I think" and "this number
    // counts less every day unless I come back".
    expect(screen.getByText(/this entry does not age votes/i)).toBeInTheDocument();
  });

  it('explains the cutoff when votes on this entry do age', async () => {
    (fetchDirectoryEntries as any).mockResolvedValue({
      items: [
        {
          ...ENTRIES.items[0],
          my_vote: 1,
          decay: { enabled: true, cutoff_days: 60, applies_to_this_entry: true },
        },
      ],
    });
    render(Directory);
    await waitFor(() => expect(screen.getByText('A Writing Tool')).toBeInTheDocument());
    expect(screen.getByText(/vote again to refresh it/i)).toBeInTheDocument();
    expect(screen.queryByText(/this entry does not age votes/i)).toBeNull();
  });

  it('says nothing about decay when the instance has it turned off', async () => {
    (fetchDirectoryEntries as any).mockResolvedValue({
      items: [
        {
          ...ENTRIES.items[0],
          my_vote: 1,
          decay: { enabled: false, cutoff_days: 60, applies_to_this_entry: false },
        },
      ],
    });
    render(Directory);
    await waitFor(() => expect(screen.getByText('A Writing Tool')).toBeInTheDocument());
    expect(screen.queryByText(/does not age votes/i)).toBeNull();
    expect(screen.queryByText(/vote again to refresh it/i)).toBeNull();
  });
});
