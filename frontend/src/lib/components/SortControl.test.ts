import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import SortControl from './SortControl.svelte';
import { fetchSort, setSort, clearSort, SORT_VALUES, SORT_LABELS } from '../api';
import { session } from '../session.svelte';

vi.mock('../api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api')>();
  return {
    ...actual,
    fetchSort: vi.fn(),
    setSort: vi.fn(),
    clearSort: vi.fn(),
  };
});

const ME = {
  id: 'acc-1',
  pseuds: [{ handle: 'reader', is_primary: true }],
  trust_level: 1,
  created_at: '',
} as any;

function signedIn() {
  session.status = 'signed-in';
  session.me = ME;
}

function anonymous() {
  session.status = 'anonymous';
  session.me = null as any;
}

function state(sort: string, source: 'preference' | 'default' = 'default') {
  return { surface: 'discover', sort, source };
}

const control = () => screen.getByTestId('sort-control');
const select = () => screen.getByLabelText('Sort') as HTMLSelectElement;

describe('SortControl', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    anonymous();
    (fetchSort as any).mockResolvedValue(state('for-you'));
    (setSort as any).mockResolvedValue(state('trending', 'preference'));
    (clearSort as any).mockResolvedValue(undefined);
  });

  it('offers the whole vocabulary, labelled for a reader rather than a wire', async () => {
    render(SortControl, { props: { surface: 'discover' } });
    const options = await screen.findAllByRole('option');
    expect(options.map((o) => (o as HTMLOptionElement).value)).toEqual([...SORT_VALUES]);
    // A–Z is prose; `az` is a query string. The reader sees the first.
    expect(screen.getByRole('option', { name: 'A–Z' })).toBeTruthy();
    expect(SORT_LABELS['for-you']).toBe('For you');
  });

  it('shows the order the server resolved, not a local guess', async () => {
    (fetchSort as any).mockResolvedValue(state('top', 'preference'));
    render(SortControl, { props: { surface: 'discover' } });
    await waitFor(() => expect(select().value).toBe('top'));
  });

  it('disables itself until the server has answered', async () => {
    let release: (v: any) => void = () => {};
    (fetchSort as any).mockReturnValue(new Promise((r) => (release = r)));
    render(SortControl, { props: { surface: 'discover' } });
    // A reader must not be offered a choice the server has not confirmed is
    // valid for this surface; an enabled control here would be a guess.
    expect(select().disabled).toBe(true);
    release(state('for-you'));
    await waitFor(() => expect(select().disabled).toBe(false));
  });

  it('tells the page, so the list moves, before it tells the server', async () => {
    signedIn();
    const order: string[] = [];
    (setSort as any).mockImplementation(async () => {
      order.push('stored');
      return state('trending', 'preference');
    });
    render(SortControl, {
      props: { surface: 'discover', onchange: () => order.push('page') },
    });
    await waitFor(() => expect(select().disabled).toBe(false));
    await fireEvent.change(select(), { target: { value: 'trending' } });
    await waitFor(() => expect(order).toEqual(['page', 'stored']));
  });

  it('remembers a signed-in reader’s choice', async () => {
    signedIn();
    render(SortControl, { props: { surface: 'discover' } });
    await waitFor(() => expect(select().disabled).toBe(false));
    await fireEvent.change(select(), { target: { value: 'az' } });
    await waitFor(() => expect(setSort).toHaveBeenCalledWith('discover', 'az'));
  });

  it('does not try to store anything for an anonymous reader', async () => {
    anonymous();
    render(SortControl, { props: { surface: 'discover' } });
    await waitFor(() => expect(select().disabled).toBe(false));
    await fireEvent.change(select(), { target: { value: 'az' } });
    await waitFor(() => expect(select().value).toBe('az'));
    // §43.4 has no preference to write for someone with no pseud.
    expect(setSort).not.toHaveBeenCalled();
    expect(clearSort).not.toHaveBeenCalled();
  });

  it('keeps the new order when storing it fails, and says so', async () => {
    signedIn();
    (setSort as any).mockRejectedValue(new Error('nope'));
    render(SortControl, { props: { surface: 'discover' } });
    await waitFor(() => expect(select().disabled).toBe(false));
    await fireEvent.change(select(), { target: { value: 'trending' } });
    await screen.findByText('This order will not be remembered next time.');
    // The reader is looking at the order they asked for. Undoing it here would
    // be worse than losing the setting.
    expect(select().value).toBe('trending');
  });

  it('offers a way back to the surface default only when a choice is stored', async () => {
    // Signed in: the stored preference this test is undoing could only have
    // been written by a signed-in reader in the first place.
    signedIn();
    (fetchSort as any).mockResolvedValue(state('trending', 'preference'));
    render(SortControl, { props: { surface: 'discover' } });
    const back = await screen.findByRole('button', { name: 'Back to default' });

    await fireEvent.click(back);
    await waitFor(() => expect(clearSort).toHaveBeenCalledWith('discover'));
    await waitFor(() => expect(select().value).toBe('for-you'));
    expect(screen.queryByRole('button', { name: 'Back to default' })).toBeNull();
  });

  it('has no reset affordance when nothing is stored', async () => {
    (fetchSort as any).mockResolvedValue(state('for-you', 'default'));
    render(SortControl, { props: { surface: 'discover' } });
    await waitFor(() => expect(select().disabled).toBe(false));
    expect(screen.queryByRole('button', { name: 'Back to default' })).toBeNull();
  });

  it('stays usable when it cannot read its own state', async () => {
    (fetchSort as any).mockRejectedValue(new Error('offline'));
    render(SortControl, { props: { surface: 'discover' } });
    // Falls back to the surface default and stays operable: this is a control
    // the reader can still use, not a failed one.
    await waitFor(() => expect(select().disabled).toBe(false));
    expect(select().value).toBe('for-you');
    expect(control()).toBeTruthy();
  });

  it('ignores a sort the server has not heard of', async () => {
    // A value outside the vocabulary must not reach the select, or the reader
    // would see a blank control with a value no surface accepts.
    (fetchSort as any).mockResolvedValue(state('randomised', 'preference'));
    render(SortControl, { props: { surface: 'discover' } });
    await waitFor(() => expect(select().disabled).toBe(false));
    expect(SORT_VALUES).toContain(select().value as any);
  });
});
