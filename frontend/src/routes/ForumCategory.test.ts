import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import ForumCategory from './ForumCategory.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchTopics: vi.fn(),
  };
});

import { fetchTopics } from '../lib/api';

const TOPICS = [
  {
    id: 'topic-1',
    title: 'Welcome',
    creator: 'alice',
    post_count: 3,
    locked: false,
    created_at: '2026-09-21T12:00:00Z',
  },
];

beforeEach(() => {
  vi.clearAllMocks();
  (fetchTopics as any).mockResolvedValue(TOPICS);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('ForumCategory page', () => {
  it('renders the forum name and topics', async () => {
    render(ForumCategory, { props: { categoryId: 'forum-1' } });
    await waitFor(() => expect(screen.getByText('Welcome')).toBeInTheDocument());
  });

  it('fetches the topics on mount', async () => {
    render(ForumCategory, { props: { categoryId: 'forum-1' } });
    await waitFor(() => expect(fetchTopics).toHaveBeenCalled());
  });

  it('shows topic metadata', async () => {
    render(ForumCategory, { props: { categoryId: 'forum-1' } });
    await waitFor(() => expect(screen.getByText('Welcome')).toBeInTheDocument());
    const link = screen.getByRole('link', { name: 'Welcome' });
    expect(link).toHaveAttribute('href', '/community/topics/topic-1');
  });
});
