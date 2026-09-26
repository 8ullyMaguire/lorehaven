import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { tick } from 'svelte';

import UserSearch from './UserSearch.svelte';
import type { UserSearchResult } from '../lib/api';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    searchUsers: vi.fn(),
  };
});

import { searchUsers } from '../lib/api';

const mockedSearch = vi.mocked(searchUsers);

const RESULTS = {
  total: 2,
  items: [
    {
      pseud_id: 'pseud-1',
      handle: 'nightowl',
      display_name: 'Owl of the Night',
      joined_at: '2024-03-01T00:00:00Z',
    },
    {
      pseud_id: 'pseud-2',
      handle: 'daybat',
      display_name: 'Bat of the Day',
      joined_at: '2026-06-01T00:00:00Z',
    },
  ],
};

/** The query the component last sent, whatever the shape of the call. */
function lastQuery(): string {
  const call = mockedSearch.mock.calls.at(-1);
  return String(call?.[0] ?? '');
}

beforeEach(() => {
  vi.clearAllMocks();
  mockedSearch.mockResolvedValue(RESULTS as any);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('UserSearch', () => {
  it('sends nothing until the reader asks', () => {
    render(UserSearch);
    expect(mockedSearch).not.toHaveBeenCalled();
  });

  it('sends what the reader typed', async () => {
    render(UserSearch);
    await fireEvent.input(screen.getByLabelText(/search pseudonyms/i), {
      target: { value: 'nightowl' },
    });
    await fireEvent.click(screen.getByRole('button', { name: /search/i }));

    await waitFor(() => expect(mockedSearch).toHaveBeenCalledTimes(1));
    expect(lastQuery()).toBe('nightowl');
  });

  it('lists the handles it found', async () => {
    render(UserSearch);
    await fireEvent.input(screen.getByLabelText(/search pseudonyms/i), {
      target: { value: 'owl' },
    });
    await fireEvent.click(screen.getByRole('button', { name: /search/i }));

    expect(await screen.findByText('nightowl')).toBeInTheDocument();
    // The display name is set inside a span with a separator, so a bare text
    // match would miss it. A function matcher is what survives that markup --
    // and it matches both the span and the list item that contains it, so the
    // assertion is that *something* matched rather than that it matched once.
    expect(
      screen.getAllByText((_, el) => el?.textContent?.includes('Owl of the Night') ?? false)
        .length,
    ).toBeGreaterThan(0);
  });

  it('emits a minimum work count as a comparison, not equality', async () => {
    // `works:10` is equality. A reader who types 10 in a "minimum" box means
    // "at least ten", and the backend agrees -- so the UI has to say which.
    render(UserSearch);
    await fireEvent.input(screen.getByLabelText(/minimum works/i), {
      target: { value: '10' },
    });
    await fireEvent.click(screen.getByRole('button', { name: /search/i }));

    await waitFor(() => expect(mockedSearch).toHaveBeenCalled());
    expect(lastQuery()).toBe('works:>=10');
  });

  it('emits both ends of a work range as one term', async () => {
    render(UserSearch);
    await fireEvent.input(screen.getByLabelText(/minimum works/i), {
      target: { value: '5' },
    });
    await fireEvent.input(screen.getByLabelText(/maximum works/i), {
      target: { value: '50' },
    });
    await fireEvent.click(screen.getByRole('button', { name: /search/i }));

    await waitFor(() => expect(mockedSearch).toHaveBeenCalled());
    expect(lastQuery()).toBe('works:5..50');
  });

  it('lets either end of the range stand alone', async () => {
    render(UserSearch);
    await fireEvent.input(screen.getByLabelText(/maximum works/i), {
      target: { value: '50' },
    });
    await fireEvent.click(screen.getByRole('button', { name: /search/i }));

    await waitFor(() => expect(mockedSearch).toHaveBeenCalled());
    expect(lastQuery()).toBe('works:<=50');
  });

  it('drops a backwards range instead of asking for one', async () => {
    // The two number inputs are bound separately, so this only means anything
    // if each one received its own value -- the assertion below is that no
    // request was made, which a silently-empty box would also satisfy.
    // `works:50..5` is refused by the backend, which would surface as a 422 the
    // reader cannot act on. Catching it here keeps the error out of the way of
    // someone who simply typed the two ends in the wrong order.
    render(UserSearch);
    await fireEvent.input(screen.getByLabelText(/minimum works/i), {
      target: { value: '50' },
    });
    await fireEvent.input(screen.getByLabelText(/maximum works/i), {
      target: { value: '5' },
    });
    await fireEvent.click(screen.getByRole('button', { name: /search/i }));

    // `bind:value` writes to the component on a microtask. Submitting before
    // it flushes reads the previous value, which is why every other test types
    // into one box and this one is the only failure without a tick.
    await tick();

    // Both boxes really did take their value, or this proves nothing.
    expect(
      (screen.getByLabelText(/minimum works/i) as HTMLInputElement).value,
    ).toBe('50');
    expect(
      (screen.getByLabelText(/maximum works/i) as HTMLInputElement).value,
    ).toBe('5');

    await fireEvent.submit(screen.getByRole('button', { name: /search/i }).closest('form')!);
    await waitFor(() => expect(mockedSearch).not.toHaveBeenCalled());
    expect(
      await screen.findByText(/minimum cannot be greater than the maximum/i),
    ).toBeInTheDocument();
  });

  it('emits a fandom as a fielded term', async () => {
    render(UserSearch);
    await fireEvent.input(screen.getByLabelText(/fandom/i), {
      target: { value: 'Good Omens' },
    });
    await fireEvent.click(screen.getByRole('button', { name: /search/i }));

    await waitFor(() => expect(mockedSearch).toHaveBeenCalled());
    expect(lastQuery()).toBe('fandoms:"Good Omens"');
  });

  it('emits a join date as a comparison', async () => {
    render(UserSearch);
    await fireEvent.input(screen.getByLabelText(/joined after/i), {
      target: { value: '2026-01-01' },
    });
    await fireEvent.click(screen.getByRole('button', { name: /search/i }));

    await waitFor(() => expect(mockedSearch).toHaveBeenCalled());
    expect(lastQuery()).toBe('joined:>2026-01-01');
  });

  it('joins free text with the filters rather than replacing it', async () => {
    render(UserSearch);
    await fireEvent.input(screen.getByLabelText(/search pseudonyms/i), {
      target: { value: 'owl' },
    });
    await fireEvent.input(screen.getByLabelText(/minimum works/i), {
      target: { value: '3' },
    });
    await fireEvent.click(screen.getByRole('button', { name: /search/i }));

    await waitFor(() => expect(mockedSearch).toHaveBeenCalled());
    expect(lastQuery()).toBe('owl AND works:>=3');
  });

  it('searches on a filter alone, with no free text', async () => {
    // `works:>=3` is a complete query. A reader who wants prolific authors has
    // not typed a name and should not have to.
    render(UserSearch);
    await fireEvent.input(screen.getByLabelText(/minimum works/i), {
      target: { value: '3' },
    });
    await fireEvent.click(screen.getByRole('button', { name: /search/i }));

    await waitFor(() => expect(mockedSearch).toHaveBeenCalled());
    expect(lastQuery()).toBe('works:>=3');
  });

  it('says a 422 the way the backend wrote it', async () => {
    mockedSearch.mockRejectedValue(
      new Error('replies is not a user field (user fields: user, fandoms, works, joined)'),
    );
    render(UserSearch);
    await fireEvent.input(screen.getByLabelText(/search pseudonyms/i), {
      target: { value: 'owl' },
    });
    await fireEvent.click(screen.getByRole('button', { name: /search/i }));

    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('replies is not a user field');
  });

  it('shows a reader how many were found', async () => {
    render(UserSearch);
    await fireEvent.input(screen.getByLabelText(/search pseudonyms/i), {
      target: { value: 'owl' },
    });
    await fireEvent.click(screen.getByRole('button', { name: /search/i }));

    expect(await screen.findByText(/2 pseudonyms/i)).toBeInTheDocument();
  });

  it('says plainly when a search found nobody', async () => {
    mockedSearch.mockResolvedValue({ total: 0, items: [] } as any);
    render(UserSearch);
    await fireEvent.input(screen.getByLabelText(/search pseudonyms/i), {
      target: { value: 'nobody' },
    });
    await fireEvent.click(screen.getByRole('button', { name: /search/i }));

    expect(await screen.findByText(/no pseudonyms/i)).toBeInTheDocument();
  });

  it('disables the button while a search is running', async () => {
    let release: (v: { total: number; items: UserSearchResult[] }) => void = () => {};
    mockedSearch.mockReturnValue(
      new Promise((resolve) => {
        release = resolve;
      }),
    );
    render(UserSearch);
    await fireEvent.input(screen.getByLabelText(/search pseudonyms/i), {
      target: { value: 'owl' },
    });
    await fireEvent.click(screen.getByRole('button', { name: /search/i }));

    expect(screen.getByRole('button', { name: /search/i })).toBeDisabled();
    release(RESULTS);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /search/i })).not.toBeDisabled(),
    );
  });
});
