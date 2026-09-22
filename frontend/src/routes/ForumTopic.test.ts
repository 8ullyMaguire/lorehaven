import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import ForumTopic from './ForumTopic.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchTopic: vi.fn(),
    fetchPosts: vi.fn(),
    createReply: vi.fn(),
    setTopicMode: vi.fn(),
  };
});

import { fetchTopic, fetchPosts } from '../lib/api';

const TOPIC = {
  id: 'topic-1',
  title: 'Best Fics of 2026',
  mode: 'plain',
  locked: false,
  tags: [],
  post_count: 3,
  creator: 'alice',
  work_backlink: null,
};

beforeEach(() => {
  vi.clearAllMocks();
  (fetchTopic as any).mockResolvedValue(TOPIC);
  (fetchPosts as any).mockResolvedValue([]);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('ForumTopic page', () => {
  it('loads the topic title', async () => {
    render(ForumTopic, { props: { topicId: 'topic-1' } });
    await waitFor(() =>
      expect(screen.getByText('Best Fics of 2026')).toBeInTheDocument(),
    );
  });

  it('fetches the topic and its posts on mount', async () => {
    render(ForumTopic, { props: { topicId: 'topic-1' } });
    await waitFor(() => expect(fetchTopic).toHaveBeenCalledWith('topic-1'));
    await waitFor(() => expect(fetchPosts).toHaveBeenCalledWith('topic-1'));
  });

  it('shows the topic metadata', async () => {
    render(ForumTopic, { props: { topicId: 'topic-1' } });
    await waitFor(() =>
      expect(screen.getByText(/started by/)).toBeInTheDocument(),
    );
  });
});
