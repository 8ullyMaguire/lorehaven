import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import AdminJobs from './AdminJobs.svelte';
import { session } from '../lib/session.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchAllJobs: vi.fn(),
    retryJob: vi.fn(),
  };
});

import { fetchAllJobs } from '../lib/api';

const JOBS = {
  items: [
    {
      id: 'job-1',
      kind: 'import',
      state: 'failed',
      payload: 'work-1',
      progress_permille: 0,
      checkpoint: null,
      last_error: 'timeout',
      attempts: 1,
      max_attempts: 3,
      available_at: '2026-09-21T12:00:00Z',
      created_at: '2026-09-21T11:00:00Z',
      updated_at: '2026-09-21T12:00:00Z',
      version: 1,
      requested_by: 'acc-1',
    },
  ],
  next_cursor: null,
};

beforeEach(() => {
  vi.clearAllMocks();
  session.status = 'signed-in';
  session.me = {
    id: 'acc-1',
    email: 'admin@example.com',
    age_state: 'declared_adult',
    email_verified: true,
    pseuds: [{ id: 'pseud-1', handle: 'admin', display_name: 'Admin', bio: null }],
    active_pseud_id: 'pseud-1',
    capabilities: { can_read: true, can_write: true, can_message: true, can_be_listed: true, max_rating: 'explicit' },
  } as any;
  (fetchAllJobs as any).mockResolvedValue(JOBS);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('AdminJobs page', () => {
  it('renders the queue heading', async () => {
    render(AdminJobs);
    await waitFor(() => expect(screen.getByText('Queue')).toBeInTheDocument());
  });

  it('fetches the job list on mount', async () => {
    render(AdminJobs);
    await waitFor(() => expect(fetchAllJobs).toHaveBeenCalled());
  });

  it('shows job states', async () => {
    render(AdminJobs);
    await waitFor(() => expect(screen.getByText('Queue')).toBeInTheDocument());
    await waitFor(() => expect(screen.getAllByText(/failed/).length).toBeGreaterThan(0));
  });
});
