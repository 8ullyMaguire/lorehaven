import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Roadmap from './Roadmap.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchRoadmapBoard: vi.fn(),
    fetchRoadmapChangelog: vi.fn(),
    fetchArenaBallot: vi.fn(),
    submitRoadmapVote: vi.fn(),
    suggestFeature: vi.fn(),
  };
});

import {
  fetchRoadmapBoard,
  fetchRoadmapChangelog,
  fetchArenaBallot,
  submitRoadmapVote,
  suggestFeature,
} from '../lib/api';

const BOARD = {
  board: {
    idea: [
      { id: 'c1', title: 'Better search', elo_rating: 1512.4, category: 'search' },
      { id: 'c2', title: 'Dark mode', elo_rating: 1490.2, category: 'ui' },
    ],
    up_next: [{ id: 'c3', title: 'Arenas v2', elo_rating: 1520.0, category: 'discovery' }],
    in_progress: [],
    finished: [],
    shipped: [{ id: 'c4', title: 'Roadmap board', elo_rating: 1500.0, category: 'meta' }],
    rejected: [],
  },
};

const CHANGELOG = {
  moves: [
    {
      id: 1,
      card_title: 'Arenas v2',
      from_stage: 'idea',
      to_stage: 'up_next',
      reason: 'community demand',
      created_at: '2026-09-20T10:00:00Z',
    },
  ],
};

const BALLOT = {
  ballot_id: 'b1',
  cards: [
    { id: 'c1', title: 'Better search' },
    { id: 'c2', title: 'Dark mode' },
    { id: 'c5', title: 'Mobile app' },
    { id: 'c6', title: 'API keys' },
  ],
};

beforeEach(() => {
  vi.clearAllMocks();
  (fetchRoadmapBoard as any).mockResolvedValue(BOARD);
  (fetchRoadmapChangelog as any).mockResolvedValue(CHANGELOG);
  (fetchArenaBallot as any).mockResolvedValue(BALLOT);
  (submitRoadmapVote as any).mockResolvedValue({ status: 'ok' });
  (suggestFeature as any).mockResolvedValue({ status: 'ok' });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('Roadmap board page', () => {
  it('renders stage columns with cards from the API', async () => {
    render(Roadmap);
    await waitFor(() => expect(screen.getByText('Better search')).toBeInTheDocument());
    expect(screen.getByText('Dark mode')).toBeInTheDocument();
    expect(screen.getByText('Arenas v2')).toBeInTheDocument();
    expect(screen.getByText('Roadmap board')).toBeInTheDocument();
    // Stage headers present.
    expect(screen.getByText('Ideas')).toBeInTheDocument();
    expect(screen.getByText('Up Next')).toBeInTheDocument();
    expect(screen.getByText('Shipped')).toBeInTheDocument();
  });

  it('shows empty state for a column with no cards', async () => {
    render(Roadmap);
    await waitFor(() => expect(screen.getByText('Better search')).toBeInTheDocument());
    expect(screen.getAllByText(/Nothing here yet/).length).toBeGreaterThan(0);
  });

  it('shows Elo ratings on cards', async () => {
    render(Roadmap);
    await waitFor(() => expect(screen.getByText(/★ 1512/)).toBeInTheDocument());
    expect(screen.getByText(/★ 1490/)).toBeInTheDocument();
  });

  it('switches to the changelog tab and lists moves', async () => {
    render(Roadmap);
    await waitFor(() => expect(screen.getByText('Better search')).toBeInTheDocument());
    await screen.getByRole('button', { name: 'Changelog' }).click();
    await waitFor(() =>
      expect(screen.getByText('Arenas v2', { selector: '.move-card' })).toBeInTheDocument(),
    );
    expect(screen.getByText(/community demand/)).toBeInTheDocument();
  });

  it('switches to the arena tab and renders the ballot when signed in', async () => {
    const { session } = await import('../lib/session.svelte.ts');
    (session as any).status = 'signed-in';
    render(Roadmap);
    await waitFor(() => expect(screen.getByText('Better search')).toBeInTheDocument());
    await screen.getByRole('button', { name: 'Arena' }).click();
    await waitFor(() => expect(screen.getByText('Mobile app')).toBeInTheDocument());
    (session as any).status = 'unknown';
  });

  it('requires sign-in for the arena tab', async () => {
    const { session } = await import('../lib/session.svelte.ts');
    (session as any).status = 'anonymous';
    render(Roadmap);
    await waitFor(() => expect(screen.getByText('Better search')).toBeInTheDocument());
    await screen.getByRole('button', { name: 'Arena' }).click();
    await waitFor(() =>
      expect(screen.getByText('to vote on upcoming features.')).toBeInTheDocument(),
    );
  });

  it('disables submit until both best and worst are picked', async () => {
    const { session } = await import('../lib/session.svelte.ts');
    (session as any).status = 'signed-in';
    render(Roadmap);
    await waitFor(() => expect(screen.getByText('Better search')).toBeInTheDocument());
    await screen.getByRole('button', { name: 'Arena' }).click();
    await waitFor(() => expect(screen.getByText('Mobile app')).toBeInTheDocument());

    const submit = screen.getByRole('button', { name: /Submit Vote/ });
    expect(submit).toBeDisabled();

    // Pick best.
    await screen.getByText('Dark mode').click();
    expect(submit).toBeDisabled();

    // Pick worst.
    await screen.getByText('API keys').click();
    await waitFor(() => expect(submit).not.toBeDisabled());
    (session as any).status = 'unknown';
  });

  it('submits a vote when both picks are made and loads a new ballot', async () => {
    const { session } = await import('../lib/session.svelte.ts');
    (session as any).status = 'signed-in';
    render(Roadmap);
    await waitFor(() => expect(screen.getByText('Better search')).toBeInTheDocument());
    await screen.getByRole('button', { name: 'Arena' }).click();
    await waitFor(() => expect(screen.getByText('Mobile app')).toBeInTheDocument());

    await screen.getByText('Dark mode').click();
    await screen.getByText('API keys').click();
    await screen.getByRole('button', { name: /Submit Vote/ }).click();

    await waitFor(() =>
      expect(submitRoadmapVote).toHaveBeenCalledWith({
        ballot_id: 'b1',
        best_id: 'c2',
        worst_id: 'c6',
      }),
    );
    (session as any).status = 'unknown';
  });

  it('shows the suggestion form when signed in and submits', async () => {
    const { session } = await import('../lib/session.svelte.ts');
    (session as any).status = 'signed-in';
    render(Roadmap);
    await waitFor(() => expect(screen.getByText('Better search')).toBeInTheDocument());
    const input = screen.getByPlaceholderText(/What would you like to see/) as HTMLInputElement;
    input.value = 'Better search';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    const form = screen.getByRole('button', { name: 'Suggest' }).closest('form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await waitFor(() =>
      expect(suggestFeature).toHaveBeenCalledWith({ title: 'Better search' }),
    );
    (session as any).status = 'unknown';
  });
});
