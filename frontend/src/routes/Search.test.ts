import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import Search from './Search.svelte';

const searchWorks = vi.fn();

vi.mock('../lib/api', () => ({
  searchWorks: (...args: unknown[]) => searchWorks(...args),
}));

beforeEach(() => {
  searchWorks.mockReset();
  searchWorks.mockResolvedValue({ items: [] });
});

describe('Search page', () => {
  it('renders search input and filter options', () => {
    render(Search);
    expect(
      screen.getByPlaceholderText('e.g. fandom:"Harry Potter" AND tag:draco'),
    ).toBeInTheDocument();
    expect(screen.getByText('Field')).toBeInTheDocument();
  });

  it('shows the empty state before any search', () => {
    render(Search);
    expect(screen.getByText(/search works/i)).toBeInTheDocument();
  });

  it('renders filter field options', () => {
    render(Search);
    expect(screen.getByText('Fandom')).toBeInTheDocument();
    expect(screen.getByText('Tag')).toBeInTheDocument();
    expect(screen.getByText('Rating')).toBeInTheDocument();
  });
});

describe('Search page word-count range', () => {
  async function type(label: string, value: string) {
    await fireEvent.input(screen.getByLabelText(label), { target: { value } });
  }

  async function submit() {
    await fireEvent.click(screen.getByText('Search'));
  }

  it('offers both bounds, each independently optional', () => {
    render(Search);
    // Two inputs rather than a slider: "over 10k, no upper bound" has to be
    // expressible, and a slider cannot leave one end open.
    expect(screen.getByLabelText('Min words')).toBeInTheDocument();
    expect(screen.getByLabelText('Max words')).toBeInTheDocument();
    expect(screen.getByLabelText('Min words')).toHaveAttribute('placeholder', 'Any');
  });

  it('sends a minimum as words:>=', async () => {
    render(Search);
    await type('Min words', '10000');
    await submit();
    await waitFor(() => expect(searchWorks).toHaveBeenCalled());
    expect(searchWorks.mock.calls[0][0]).toBe('words:>=10000');
  });

  it('sends a maximum as words:<=', async () => {
    render(Search);
    await type('Max words', '50000');
    await submit();
    await waitFor(() => expect(searchWorks).toHaveBeenCalled());
    expect(searchWorks.mock.calls[0][0]).toBe('words:<=50000');
  });

  it('combines both bounds', async () => {
    render(Search);
    await type('Min words', '1000');
    await type('Max words', '5000');
    await submit();
    await waitFor(() => expect(searchWorks).toHaveBeenCalled());
    expect(searchWorks.mock.calls[0][0]).toBe('words:>=1000 AND words:<=5000');
  });

  it('ANDs the range with free text', async () => {
    render(Search);
    await type('Search query', 'winter');
    await type('Min words', '10000');
    await submit();
    await waitFor(() => expect(searchWorks).toHaveBeenCalled());
    expect(searchWorks.mock.calls[0][0]).toBe('winter AND words:>=10000');
  });

  it('sends nothing while both bounds are still empty', async () => {
    render(Search);
    // Focusing the fields and leaving them blank must not produce `words:>=`,
    // which the server rejects with a 422. The reader would see an error for a
    // field they never filled in.
    await fireEvent.click(screen.getByLabelText('Min words'));
    await fireEvent.click(screen.getByLabelText('Max words'));
    await submit();
    expect(searchWorks).not.toHaveBeenCalled();
  });

  it('ignores a bound that is not a number', async () => {
    render(Search);
    // `type="number"` is a hint, not a guarantee: a pasted value can still
    // reach the handler, and the query must not carry it either way.
    await type('Min words', 'ten thousand');
    await submit();
    expect(searchWorks).not.toHaveBeenCalled();
  });

  it('clears the range and searches again', async () => {
    render(Search);
    await type('Min words', '10000');
    await submit();
    await waitFor(() => expect(searchWorks).toHaveBeenCalledTimes(1));

    await fireEvent.click(screen.getByText('Clear'));
    await waitFor(() => expect(screen.getByLabelText('Min words')).toHaveValue(null));
    // Cleared means cleared: no dangling `words:>=` left in the next query.
    await submit();
    expect(searchWorks).toHaveBeenCalledTimes(1);
  });
});
