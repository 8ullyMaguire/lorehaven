import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';
import Draw from './Draw.svelte';
import { signInTestReader, resetTestSession } from '../lib/testing/session';
import { fetchDraw, pickDrawCard, ApiError } from '../lib/api';

/**
 * §16.10a, the draw.
 *
 * The properties these tests exist for are the ones the amendment rests on:
 *
 * 1. **Asking again is a different NONCE, never a re-fetch of the same hand** — the
 *    server deals the same hand for the same (day, nonce), so a page that "refreshed"
 *    with nonce 0 would show a reader the same three cards forever and call it a deal.
 * 2. **The take is recorded against the request the cards came from**, not against the
 *    work alone: two hands can deal the same work, and the slot the server marks is the
 *    one the reader is looking at.
 * 3. **A failed take clears the optimistic mark** — a card left looking "taken" after the
 *    server refused the pick is the page telling the reader it recorded something it did
 *    not.
 *
 * The empty state is deliberately asserted on the SERVER's sentence: the endpoint
 * distinguishes an empty catalogue from an empty pool, and the page must not write its
 * own wording that collapses the two.
 */
vi.mock('../lib/api', async () => {
  const actual = await vi.importActual<typeof import('../lib/api')>('../lib/api');
  return { ...actual, fetchDraw: vi.fn(), pickDrawCard: vi.fn() };
});

const mockFetch = vi.mocked(fetchDraw);
const mockPick = vi.mocked(pickDrawCard);

const HAND = {
  cards: [
    {
      workId: 'work-a',
      title: 'The Salt Road',
      authorHandle: 'quill',
      position: 0,
      slotId: 'slot-a',
    },
    {
      workId: 'work-b',
      title: 'Nine Winters',
      authorHandle: 'ember',
      position: 1,
      slotId: 'slot-b',
    },
    {
      workId: 'work-c',
      title: 'A Ledger of Small Mercies',
      authorHandle: 'moth',
      position: 2,
      slotId: 'slot-c',
    },
  ],
  requestId: 'req-1',
  date: '2026-10-27',
  nonce: 0,
  poolSize: 60,
};

beforeEach(() => {
  signInTestReader();
  mockFetch.mockReset();
  mockPick.mockReset();
});

afterEach(() => {
  resetTestSession();
});

describe('Draw', () => {
  it('deals three cards, each linking to its work', async () => {
    mockFetch.mockResolvedValue(HAND);

    render(Draw);

    const link = await screen.findByRole('link', { name: 'The Salt Road' });
    expect(link.getAttribute('href')).toBe('/works/work-a');
    expect(screen.getByRole('link', { name: 'Nine Winters' })).toBeTruthy();
    expect(screen.getByRole('link', { name: 'A Ledger of Small Mercies' })).toBeTruthy();
    expect(mockFetch).toHaveBeenCalledWith(expect.objectContaining({ nonce: 0 }));
  });

  it('records the take against the request the hand came from', async () => {
    mockFetch.mockResolvedValue(HAND);
    mockPick.mockResolvedValue({ chosenAt: '2026-10-27T15:00:00Z' });

    render(Draw);

    const take = await screen.findByRole('button', { name: 'Take Nine Winters' });
    await fireEvent.click(take);

    // The REQUEST id, not just the work: the slot that gets marked is this hand's.
    await waitFor(() => expect(mockPick).toHaveBeenCalledWith('req-1', 'work-b'));
    expect(await screen.findByText('Taken. It is in your hands now.')).toBeTruthy();
  });

  it('refuses a second take, because the server keeps the first', async () => {
    mockFetch.mockResolvedValue(HAND);
    mockPick.mockResolvedValue({ chosenAt: '2026-10-27T15:00:00Z' });

    render(Draw);

    await fireEvent.click(await screen.findByRole('button', { name: 'Take Nine Winters' }));
    await waitFor(() => expect(mockPick).toHaveBeenCalledTimes(1));

    const other = screen.queryByRole('button', { name: 'Take The Salt Road' });
    // Present but disabled, or absent: either way it cannot be taken.
    if (other) expect((other as HTMLButtonElement).disabled).toBe(true);
    await fireEvent.click(other as HTMLElement);
    expect(mockPick).toHaveBeenCalledTimes(1);
  });

  it('deals a new hand with a new nonce, never the same one twice', async () => {
    mockFetch.mockResolvedValue(HAND);

    render(Draw);

    await screen.findByRole('link', { name: 'The Salt Road' });
    await fireEvent.click(screen.getByRole('button', { name: /Deal another hand/ }));

    await waitFor(() =>
      expect(mockFetch).toHaveBeenLastCalledWith(expect.objectContaining({ nonce: 1 })),
    );
  });

  it('renders the server sentence when there is nothing to deal', async () => {
    mockFetch.mockResolvedValue({
      cards: [],
      requestId: null,
      date: '2026-10-27',
      nonce: 0,
      emptyReason: 'the recommendation pool is empty: nothing is published',
    });

    render(Draw);

    expect(await screen.findByText(/the recommendation pool is empty/)).toBeTruthy();
  });

  it('clears the take when the server refuses it, and says so', async () => {
    mockFetch.mockResolvedValue(HAND);
    mockPick.mockRejectedValue(new ApiError(404, 'NOT_FOUND', 'not found'));

    render(Draw);

    await fireEvent.click(await screen.findByRole('button', { name: 'Take The Salt Road' }));

    await waitFor(() => expect(screen.queryByText('Taken. It is in your hands now.')).toBeNull());
    // The card is takeable again — the reader can retry a different card.
    expect(screen.getByRole('button', { name: 'Take The Salt Road' })).toBeTruthy();
  });
});
