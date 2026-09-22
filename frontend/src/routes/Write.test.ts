import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Write from './Write.svelte';
import { session } from '../lib/session.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchWorks: vi.fn(),
    fetchInvitations: vi.fn(),
    acceptWorkInvitation: vi.fn(),
    declineWorkInvitation: vi.fn(),
  };
});

import { fetchWorks, fetchInvitations } from '../lib/api';

const WORKS = [
  {
    id: 'work-1',
    title: 'My Story',
    lifecycle: 'in_progress',
    visibility: 'public',
    completion: 'in_progress',
    rating: 'teen',
    updated_at: '2026-09-21T12:00:00Z',
    published_at: '2026-09-21T11:00:00Z',
    version: 1,
    chapter_count: 2,
    word_count: 5000,
    role: 'author',
  },
];

const INVITATIONS = [
  {
    id: 'inv-1',
    work_id: 'work-2',
    work_title: 'Collaboration',
    invited_by: 'bob',
    role: 'coauthor',
    role_label: 'Co-author',
  },
];

beforeEach(() => {
  vi.clearAllMocks();
  session.status = 'signed-in';
  session.me = {
    id: 'acc-1',
    email: 'test@example.com',
    age_state: 'declared_adult',
    email_verified: true,
    pseuds: [{ id: 'pseud-1', handle: 'alice', display_name: 'Alice', bio: null }],
    active_pseud_id: 'pseud-1',
    capabilities: { can_read: true, can_write: true, can_message: true, can_be_listed: true, max_rating: 'explicit' },
  } as any;
  (fetchWorks as any).mockResolvedValue(WORKS);
  (fetchInvitations as any).mockResolvedValue(INVITATIONS);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('Write page', () => {
  it('renders the write heading', async () => {
    render(Write);
    await waitFor(() => expect(screen.getByText('Write')).toBeInTheDocument());
  });

  it('shows the user\'s works', async () => {
    render(Write);
    await waitFor(() => expect(screen.getByText('My Story')).toBeInTheDocument());
  });

  it('shows pending invitations', async () => {
    render(Write);
    await waitFor(() => expect(screen.getByText('Invitations')).toBeInTheDocument());
    await waitFor(() => expect(screen.getByText('Collaboration')).toBeInTheDocument());
  });
});
