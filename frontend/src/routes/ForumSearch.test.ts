import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import ForumSearch from './ForumSearch.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    searchForum: vi.fn(),
    fetchForums: vi.fn(),
  };
});

import { fetchForums, searchForum } from '../lib/api';

const mockedSearch = vi.mocked(searchForum);
const mockedForums = vi.mocked(fetchForums);

const RESULTS = [
  {
    id: 'topic-1',
    title: 'Best fics?',
    author_pseud: 'alice',
    excerpt: 'What are the best fics?',
    created_at: '2026-09-21T12:00:00Z',
  },
];

/** The categories a real instance has. The old hardcoded list had none of these. */
const CATEGORIES = [
  { id: 'cat-1', name: 'General' },
  { id: 'cat-2', name: 'Writing Help' },
];

beforeEach(() => {
  vi.clearAllMocks();
  mockedSearch.mockResolvedValue(RESULTS as any);
  mockedForums.mockResolvedValue(CATEGORIES as any);
});

afterEach(() => {
  vi.restoreAllMocks();
});

async function type(label: string, value: string) {
  const field = screen.getByLabelText(label);
  await fireEvent.input(field, { target: { value } });
}

async function submit() {
  await fireEvent.click(screen.getByRole('button', { name: 'Search' }));
}

/** The parameter object the last search was called with. */
function lastParams(): Record<string, any> {
  return mockedSearch.mock.calls[mockedSearch.mock.calls.length - 1][0] as any;
}

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
    await waitFor(() => expect(mockedSearch).toHaveBeenCalled());
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

  it('passes a query-language filter straight through', async () => {
    // The point of the shared language: a reader who has learned `words:>10000`
    // on the works search types it here too. The box must not validate or
    // rewrite it, because only the server knows which fields a forum has.
    render(ForumSearch);
    await type('Search posts and topics', 'replies:>50');
    await submit();
    await waitFor(() => expect(mockedSearch).toHaveBeenCalled());
    expect(lastParams().q).toBe('replies:>50');
  });

  it('sends a minimum reply count as its own parameter', async () => {
    render(ForumSearch);
    await type('Search posts and topics', 'winter');
    await type('Min replies', '50');
    await submit();
    await waitFor(() => expect(mockedSearch).toHaveBeenCalled());
    expect(lastParams().min_replies).toBe(50);
  });

  it('omits an untouched reply bound rather than sending zero', async () => {
    // `min_replies: 0` is a filter nobody asked for, and in the request log it
    // reads as "only threads with no replies".
    render(ForumSearch);
    await type('Search posts and topics', 'winter');
    await submit();
    await waitFor(() => expect(mockedSearch).toHaveBeenCalled());
    expect(lastParams().min_replies).toBeUndefined();
  });

  it('offers the categories the instance actually has', async () => {
    render(ForumSearch);
    await waitFor(() => expect(mockedForums).toHaveBeenCalled());
    await waitFor(() => {
      const select = screen.getByLabelText('Category') as HTMLSelectElement;
      const values = Array.from(select.options).map((o) => o.value);
      expect(values).toContain('General');
      expect(values).toContain('Writing Help');
    });
  });

  it('does not offer invented categories', async () => {
    // The old list was hardcoded -- `general`, `fanworks`, `discussion`, `help`
    // -- and none of them is a category a fresh instance creates. A reader who
    // picked "General" and got nothing had no way to tell the dropdown was
    // lying, which is the worst kind of empty result.
    render(ForumSearch);
    await waitFor(() => expect(mockedForums).toHaveBeenCalled());
    const select = screen.getByLabelText('Category') as HTMLSelectElement;
    const values = Array.from(select.options).map((o) => o.value);
    expect(values).not.toContain('general');
    expect(values).not.toContain('fanworks');
    expect(values).not.toContain('discussion');
    expect(values).not.toContain('help');
  });

  it('shows the reason the server gave for a bad query', async () => {
    // A 422 carries why the query was wrong. Replacing that with "Search
    // failed" throws away the only useful part of the response.
    mockedSearch.mockRejectedValue(
      new Error(
        'words is not a forum field (forum fields: category, author, title, kind, replies, active, pinned, locked)',
      ),
    );
    render(ForumSearch);
    await type('Search posts and topics', 'words:>10000');
    await submit();
    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
    expect(screen.getByRole('alert').textContent).toContain('not a forum field');
  });

  it('lists what the search understands', async () => {
    // A reader cannot learn the operators except by being told. One line under
    // the box costs nothing and answers "what am I allowed to type?".
    render(ForumSearch);
    // The whole paragraph, not one <code> inside it -- each operator is its
    // own element, so a text query on any single one proves nothing about the
    // line a reader actually sees.
    const help = document.querySelector('.help') as HTMLElement;
    expect(help).toBeTruthy();
    for (const operator of ['replies:', 'category:', 'author:', 'active:', 'locked:']) {
      expect(help.textContent).toContain(operator);
    }
  });

  it('does not claim there are no results before a search has run', async () => {
    // Silence with an empty list reads as "no results", which is a different
    // and wrong thing.
    render(ForumSearch);
    expect(screen.queryByText(/No results/i)).toBeNull();
  });

  it('will search with only a filter set and no text', async () => {
    // Before the query language, an empty box meant "nothing to search". Now a
    // dropdown alone is a complete query -- `category:meta` -- and refusing to
    // run makes the filters look broken.
    render(ForumSearch);
    await type('Min replies', '10');
    await submit();
    await waitFor(() => expect(mockedSearch).toHaveBeenCalled());
    expect(lastParams().min_replies).toBe(10);
  });
});
