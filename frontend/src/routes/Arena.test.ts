import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Arena from './Arena.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchArenaNext: vi.fn(),
    submitArenaVote: vi.fn(),
    dismissArena: vi.fn(),
    fetchArenaWeights: vi.fn(),
  };
});

import {
  fetchArenaNext,
  submitArenaVote,
  dismissArena,
  fetchArenaWeights,
} from '../lib/api';

const ROUND = {
  round_id: 'round-1',
  cards: [
    {
      work_id: 'work-1',
      title: 'Alpha',
      fandom: 'Good Omens',
      word_count: 5000,
      tags: ['fluff', 'romance'],
      excerpt: 'Once upon a time in a garden...',
    },
    {
      work_id: 'work-2',
      title: 'Beta',
      fandom: 'Good Omens',
      word_count: 3000,
      tags: ['angst'],
      excerpt: 'The world was ending...',
    },
    {
      work_id: 'work-3',
      title: 'Gamma',
      fandom: 'Good Omens',
      word_count: 8000,
      tags: ['humor'],
      excerpt: 'Crowley had never been good at following rules...',
    },
    {
      work_id: 'work-4',
      title: 'Delta',
      fandom: 'Good Omens',
      word_count: 2000,
      tags: ['hurt-comfort'],
      excerpt: 'Aziraphale found him on the floor...',
    },
  ],
};

const WEIGHTS = {
  dimensions: [
    { key: 'prose', label: 'Prose', weight: 0.3, influence: 'High' },
    { key: 'pacing', label: 'Pacing', weight: 0.2, influence: 'Medium' },
    { key: 'characters', label: 'Characters', weight: 0.5, influence: 'High' },
  ],
};

beforeEach(() => {
  vi.clearAllMocks();
  (fetchArenaNext as any).mockResolvedValue({
    round: ROUND,
    dimensions: [
      { key: 'prose', label: 'Prose', matches_played: 5 },
      { key: 'pacing', label: 'Pacing', matches_played: 3 },
    ],
  });
  (submitArenaVote as any).mockResolvedValue({
    status: 'recorded',
    next_round: null,
  });
  (dismissArena as any).mockResolvedValue({ status: 'dismissed' });
  (fetchArenaWeights as any).mockResolvedValue(WEIGHTS);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('Taste Calibration Arena', () => {
  it('renders four cards from the API round', async () => {
    render(Arena);
    await waitFor(() => expect(screen.getByText('Alpha')).toBeInTheDocument());
    expect(screen.getByText('Beta')).toBeInTheDocument();
    expect(screen.getByText('Gamma')).toBeInTheDocument();
    expect(screen.getByText('Delta')).toBeInTheDocument();
  });

  it('disables Submit until best and worst are both picked', async () => {
    render(Arena);
    await waitFor(() => expect(screen.getByText('Alpha')).toBeInTheDocument());
    // Submit hidden until both best and worst are picked.
    expect(screen.queryByText('Submit & Next Round')).not.toBeInTheDocument();
    await screen.getAllByRole('button', { name: 'Best' })[0].click();
    expect(screen.queryByText('Submit & Next Round')).not.toBeInTheDocument();
    await screen.getAllByRole('button', { name: 'Worst' })[1].click();
    const submit = await screen.findByText('Submit & Next Round');
    expect(submit).not.toBeDisabled();
  });

  it('prevents selecting same card as best and worst', async () => {
    render(Arena);
    await waitFor(() => expect(screen.getByText('Alpha')).toBeInTheDocument());
    await screen.getAllByRole('button', { name: 'Best' })[0].click();
    expect(screen.getByText('✓ Best')).toBeInTheDocument();
    await screen.getAllByRole('button', { name: 'Worst' })[0].click();
    // Best should be cleared
    expect(screen.queryByText('✓ Best')).not.toBeInTheDocument();
    expect(screen.getByText('✗ Worst')).toBeInTheDocument();
  });

  it('shows reason tag options after clicking Why?', async () => {
    render(Arena);
    await waitFor(() => expect(screen.getByText('Alpha')).toBeInTheDocument());
    await screen.getAllByRole('button', { name: 'Best' })[0].click();
    await waitFor(() => expect(screen.getByText('✓ Best')).toBeInTheDocument());
    await screen.getAllByRole('button', { name: 'Worst' })[1].click();
    await waitFor(() => expect(screen.getByText('✗ Worst')).toBeInTheDocument());
    // Reason tags not shown before clicking Why?.
    expect(screen.queryByText('Premise')).not.toBeInTheDocument();
    expect(screen.queryByText('Vibe')).not.toBeInTheDocument();
    const whyLink = await screen.findByText(/Why\?/);
    await whyLink.click();
    // Premise and Vibe are unique to reason tags (unlike Prose/Pacing/Characters).
    await waitFor(() =>
      expect(screen.getByText('Premise')).toBeInTheDocument(),
    );
    expect(screen.getByText('Vibe')).toBeInTheDocument();
  });

  it('submits vote when Submit is clicked', async () => {
    render(Arena);
    await waitFor(() => expect(screen.getByText('Alpha')).toBeInTheDocument());
    await screen.getAllByRole('button', { name: 'Best' })[0].click();
    await screen.getAllByRole('button', { name: 'Worst' })[1].click();
    await screen.getByText('Submit & Next Round').click();
    await waitFor(() =>
      expect(submitArenaVote).toHaveBeenCalledWith({
        best_work_id: 'work-1',
        worst_work_id: 'work-2',
        reason_tags: [],
      }),
    );
  });

  it('shows weights summary when available', async () => {
    render(Arena);
    await waitFor(() => expect(screen.getByText('Alpha')).toBeInTheDocument());
    await waitFor(() =>
      expect(screen.getByText('What Matters to You')).toBeInTheDocument(),
    );
    // Characters appears only in weights (weight 0.5, influence High), not in progress.
    expect(screen.getByText('Characters')).toBeInTheDocument();
    expect(screen.getAllByText('High').length).toBeGreaterThan(0);
  });

  it('shows an error summary when the round fetch fails', async () => {
    (fetchArenaNext as any).mockRejectedValue(new Error('nope'));
    render(Arena);
    await waitFor(() =>
      expect(screen.getByText(/Something went wrong/)).toBeInTheDocument(),
    );
  });

  it('dismisses the arena when Skip is clicked', async () => {
    render(Arena);
    await waitFor(() => expect(screen.getByText('Alpha')).toBeInTheDocument());
    await screen.getByText('Skip arena for now').click();
    await waitFor(() => expect(dismissArena).toHaveBeenCalled());
    expect(screen.getByText(/Arena dismissed/)).toBeInTheDocument();
  });
});
