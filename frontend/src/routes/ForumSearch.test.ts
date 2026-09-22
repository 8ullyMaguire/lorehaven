import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import ForumSearch from './ForumSearch.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    searchForum: vi.fn(),
  };
});

import { searchForum } from '../lib/api';

const RESULTS = [
  {
    id: 'topic-1',
    title: 'Best fics?',
    author_pseud: 'alice',
    excerpt: 'What are the best fics?',
    created_at: '2026-09-21T12:00:00Z',
  },
];

beforeEach(() => {
  vi.clearAllMocks();
  (searchForum as any).mockResolvedValue(RESULTS);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('ForumSearch page', () => {
  it('renders the search form', () => {
    render(ForumSearch);
    expect(screen.getByText('Search Forums')).toBeInTheDocument();
  });

  it('shows results after searching', async () => {
    render(ForumSearch);
    const input = screen.getByPlaceholderText(/Search posts and topics/) as HTMLInputElement;
    await fireEvent.input(input, { target: { value: 'best' } });
    const button = screen.getByRole('button', { name: 'Search' });
    await fireEvent.click(button);
    await waitFor(() => expect(searchForum).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByText('Best fics?')).toBeInTheDocument());
  });

  it('shows the author of each result', async () => {
    render(ForumSearch);
    const input = screen.getByPlaceholderText(/Search posts and topics/) as HTMLInputElement;
    await fireEvent.input(input, { target: { value: 'best' } });
    const button = screen.getByRole('button', { name: 'Search' });
    await fireEvent.click(button);
    await waitFor(() => expect(screen.getByText('Best fics?')).toBeInTheDocument());
    expect(screen.getByText(/alice/)).toBeInTheDocument();
  });
});
