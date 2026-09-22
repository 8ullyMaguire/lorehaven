import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Discover from './Discover.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchDiscoveryFeed: vi.fn(),
    clearTasteProfile: vi.fn(),
    recomputeTasteProfile: vi.fn(),
  };
});

import { fetchDiscoveryFeed } from '../lib/api';

const FEED = {
  items: [
    {
      work_id: 'work-1',
      title: 'Discovery Test',
      author_handle: 'alice',
      word_count: 3000,
      rating: 'teen',
      tags: [],
      summary: 'A test work.',
      score: 0.8,
    },
  ],
};

beforeEach(() => {
  vi.clearAllMocks();
  (fetchDiscoveryFeed as any).mockResolvedValue(FEED);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('Discover page', () => {
  it('renders the discovery feed', async () => {
    render(Discover);
    await waitFor(() => expect(screen.getAllByText(/Discovery Test/).length).toBeGreaterThan(0));
  });

  it('fetches the feed on mount', async () => {
    render(Discover);
    await waitFor(() => expect(fetchDiscoveryFeed).toHaveBeenCalled());
  });

  it('shows the author handle', async () => {
    render(Discover);
    await waitFor(() => expect(screen.getByText(/by alice/)).toBeInTheDocument());
  });

  it('links to the work page', async () => {
    render(Discover);
    await waitFor(() => expect(screen.getAllByText(/Discovery Test/).length).toBeGreaterThan(0));
    const link = screen.getByRole('link', { name: /Discovery Test/ });
    expect(link).toHaveAttribute('href', '/works/work-1');
  });
});
