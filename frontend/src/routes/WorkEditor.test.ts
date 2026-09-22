import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import WorkEditor from './WorkEditor.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchWork: vi.fn(),
    fetchChapter: vi.fn(),
    updateWork: vi.fn(),
    fetchWorkPricing: vi.fn(),
    setWorkPricing: vi.fn(),
    deleteWorkPricing: vi.fn(),
  };
});

import { fetchWork } from '../lib/api';

const WORK = {
  id: 'work-1',
  title: 'Editable Story',
  summary: 'A test work',
  language: 'en',
  rating: 'teen',
  visibility: 'public',
  lifecycle: 'draft',
  completion: 'in_progress',
  version: 1,
  created_at: '2026-09-21T11:00:00Z',
  updated_at: '2026-09-21T12:00:00Z',
  published_at: null,
  withdrawn_at: null,
  show_public_ratings: true,
  discussion_mode: 'thread_only',
  role: 'owner',
  chapters: [
    {
      id: 'ch-1',
      title: 'Chapter 1',
      order_key: 1,
      word_count: 1000,
      revision_count: 1,
      version: 1,
      updated_at: '2026-09-21T12:00:00Z',
      created_at: '2026-09-21T11:00:00Z',
      has_content: true,
      current_revision_id: 'rev-1',
    },
  ],
  contributors: [
    { pseud_id: 'pseud-1', handle: 'alice', display_name: 'Alice', role: 'owner', public_attribution: true },
  ],
  publication_blockers: [],
};

beforeEach(() => {
  vi.clearAllMocks();
  (fetchWork as any).mockResolvedValue(WORK);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('WorkEditor page', () => {
  it('renders the work title', async () => {
    render(WorkEditor, { props: { workId: 'work-1' } });
    await waitFor(() => expect(screen.getByText('Editable Story')).toBeInTheDocument());
  });

  it('fetches the work on mount', async () => {
    render(WorkEditor, { props: { workId: 'work-1' } });
    await waitFor(() => expect(fetchWork).toHaveBeenCalledWith('work-1'));
  });

  it('shows metadata section', async () => {
    render(WorkEditor, { props: { workId: 'work-1' } });
    await waitFor(() => expect(screen.getByText('Editable Story')).toBeInTheDocument());
    expect(screen.getByText('Details')).toBeInTheDocument();
  });

  it('shows chapters section', async () => {
    render(WorkEditor, { props: { workId: 'work-1' } });
    await waitFor(() => expect(screen.getByText('Editable Story')).toBeInTheDocument());
    expect(screen.getByText('Chapters')).toBeInTheDocument();
    expect(screen.getByText('Chapter 1')).toBeInTheDocument();
  });
});
