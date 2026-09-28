import { render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import RoadmapCard from './RoadmapCard.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return { ...actual, fetchRoadmapCard: vi.fn() };
});

import { ApiError, fetchRoadmapCard } from '../lib/api';

const BODY = 'A page of prose explaining what this feature is and why it exists.';

const CARD = {
  id: 'card-1',
  title: 'Preservation coverage score',
  body: BODY,
  category: 'preservation',
  stage: 'idea',
  elo_rating: 1612.4,
  matches_played: 7,
  times_best: 2,
  times_worst: 1,
  created_at: '2026-09-28T00:00:00+00:00',
  updated_at: '2026-09-28T00:00:00+00:00',
};

/** The component takes route params, not the id directly. */
function renderWith(cardId: string) {
  return render(RoadmapCard, { props: { params: { cardId } } });
}

describe('RoadmapCard', () => {
  beforeEach(() => {
    vi.mocked(fetchRoadmapCard).mockReset();
  });

  it('shows the title and the body of the card it fetched', async () => {
    vi.mocked(fetchRoadmapCard).mockResolvedValue({ card: CARD });

    renderWith('card-1');

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: CARD.title })).toBeTruthy();
    });
    expect(screen.getByText(BODY)).toBeTruthy();
    // The id is passed through verbatim so the server can find the card.
    expect(fetchRoadmapCard).toHaveBeenCalledWith('card-1');
  });

  it('renders a placeholder, not a blank, when the card has no body', async () => {
    // §44.1: empty is a supported state — a card created through the suggest
    // endpoint is title-only — and it must read as "not written yet" rather
    // than as an empty article.
    vi.mocked(fetchRoadmapCard).mockResolvedValue({ card: { ...CARD, body: '' } });

    renderWith('card-1');

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: CARD.title })).toBeTruthy();
    });
    expect(screen.getByText(/no description yet/i)).toBeTruthy();
  });

  it('treats whitespace-only prose as no body at all', async () => {
    vi.mocked(fetchRoadmapCard).mockResolvedValue({ card: { ...CARD, body: '   \n  ' } });

    renderWith('card-1');

    await waitFor(() => {
      expect(screen.getByText(/no description yet/i)).toBeTruthy();
    });
  });

  it('surfaces a failed fetch instead of rendering an empty card', async () => {
    vi.mocked(fetchRoadmapCard).mockRejectedValue(new Error('not found'));

    renderWith('missing');

    // ErrorSummary deliberately shows only an ApiError's message and renders
    // anything else as a generic line, so an unexpected failure does not leak
    // its text into the page. Assert the alert and the absence of the card —
    // asserting the thrown message here would be asserting a leak.
    await waitFor(() => {
      expect(screen.getByRole('alert')).toBeTruthy();
    });
    expect(screen.getByText('Something went wrong.')).toBeTruthy();
    expect(screen.queryByRole('heading', { name: CARD.title })).toBeNull();
  });

  it('shows the server\'s own message for an ApiError', async () => {
    vi.mocked(fetchRoadmapCard).mockRejectedValue(
      new ApiError(404, 'not_found', 'No such card.'),
    );

    renderWith('missing');

    await waitFor(() => {
      expect(screen.getByText('No such card.')).toBeTruthy();
    });
  });

  it('refuses to fetch when the route named no card', async () => {
    render(RoadmapCard, { props: { params: {} } });

    await waitFor(() => {
      expect(screen.getByRole('alert')).toBeTruthy();
    });
    expect(fetchRoadmapCard).not.toHaveBeenCalled();
  });

  it('does not render the body as HTML', async () => {
    // Bodies are operator-authored today, but the rule is pinned here so that
    // making them member-authored is a deliberate change to this test rather
    // than something discovered after it ships.
    const hostile = '<img src=x onerror="alert(1)">';
    vi.mocked(fetchRoadmapCard).mockResolvedValue({ card: { ...CARD, body: hostile } });

    const { container } = renderWith('card-1');

    await waitFor(() => {
      expect(screen.getByText(hostile)).toBeTruthy();
    });
    expect(container.querySelector('img')).toBeNull();
  });
});
