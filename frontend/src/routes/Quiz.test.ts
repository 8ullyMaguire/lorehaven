import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Quiz from './Quiz.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchQuizWorks: vi.fn(),
    saveQuizAnswers: vi.fn(),
    skipQuiz: vi.fn(),
  };
});

import { fetchQuizWorks, saveQuizAnswers, skipQuiz } from '../lib/api';

const WORKS = {
  works: [
    { work_id: 'work-1', title: 'Alpha', author_handle: 'alice', word_count: 1000 },
    { work_id: 'work-2', title: 'Beta', author_handle: 'bob', word_count: 2000 },
  ],
};

beforeEach(() => {
  vi.clearAllMocks();
  (fetchQuizWorks as any).mockResolvedValue(WORKS);
  (saveQuizAnswers as any).mockResolvedValue({ status: 'ok', vector_dimensions: 128 });
  (skipQuiz as any).mockResolvedValue({ status: 'skipped' });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('Quiz onboarding page', () => {
  it('renders quiz works from the API', async () => {
    render(Quiz);
    await waitFor(() => expect(screen.getByText('Alpha')).toBeInTheDocument());
    expect(screen.getByText('Beta')).toBeInTheDocument();
    expect(screen.getByText('1000 words')).toBeInTheDocument();
  });

  it('shows a message when no quiz works are available', async () => {
    (fetchQuizWorks as any).mockResolvedValue({ works: [] });
    render(Quiz);
    await waitFor(() => expect(screen.getByText(/No quiz works/)).toBeInTheDocument());
  });

  it('toggles the Like button on click and shows the count', async () => {
    render(Quiz);
    await waitFor(() => expect(screen.getByText('Alpha')).toBeInTheDocument());
    const likeButton = screen.getAllByText('Like')[0];
    await likeButton.click();
    expect(screen.getByText('1 picked')).toBeInTheDocument();
  });

  it('disables Continue until at least one work is picked', async () => {
    render(Quiz);
    await waitFor(() => expect(screen.getByText('Alpha')).toBeInTheDocument());
    const continueButton = screen.getByText('Continue');
    expect(continueButton).toBeDisabled();
    await screen.getAllByText('Like')[0].click();
    expect(continueButton).not.toBeDisabled();
  });

  it('submits quiz answers when Continue is clicked', async () => {
    render(Quiz);
    await waitFor(() => expect(screen.getByText('Alpha')).toBeInTheDocument());
    await screen.getAllByText('Like')[0].click();
    await screen.getByText('Continue').click();
    await waitFor(() => expect(saveQuizAnswers).toHaveBeenCalled());
  });
});
