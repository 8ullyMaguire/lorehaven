import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../lib/api', () => ({
  fetchAuthorMediaHealth: vi.fn(async () => ({ items: [] })),
  fetchMediaPreferences: vi.fn(async () => ({
    account_id: 'acct-1',
    auto_submit_to_archive: true,
    prefer_curator_verified: true,
    broken_link_notifications: 'digest_weekly',
    allow_curator_edits: true,
    minimum_healthy_links: 3,
  })),
  fetchWorkMediaReferences: vi.fn(async () => ({ items: [] })),
  postMediaReference: vi.fn(async () => ({ id: 'ref-1', link_id: 'link-1', status: 'pending' })),
  postTargetedBounty: vi.fn(async () => ({ id: 'bounty-1' })),
  putMediaPreferences: vi.fn(async () => ({ status: 'ok' })),
  reportBrokenLink: vi.fn(async () => ({ status: 'ok' })),
}));

const api = await import('../lib/api');
import { session } from '../lib/session.svelte';

import AuthorMedia from './AuthorMedia.svelte';

describe('AuthorMedia', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    session.status = 'signed-in';
    session.me = {
      id: 'acct-1',
      email: 'author@example.com',
      age_state: 'declared_adult',
      email_verified: true,
      pseuds: [{ id: 'pseud-1', handle: 'author', display_name: 'Author', bio: null }],
      active_pseud_id: 'pseud-1',
      capabilities: { can_read: true, can_write: true, can_message: true, can_be_listed: true, max_rating: 'explicit' },
    } as any;
  });

  afterEach(() => {
    vi.restoreAllMocks();
    session.status = 'unknown';
    session.me = null;
  });

  it('shows loading skeleton initially', () => {
    render(AuthorMedia);
    expect(screen.getByText('Your Works — Media Health Report')).toBeTruthy();
  });

  it('renders empty state when no media exists', async () => {
    render(AuthorMedia);
    await waitFor(() => {
      expect(screen.getByText('No media yet')).toBeTruthy();
    });
  });

  it('renders summary counts and per-work health badges', async () => {
    api.fetchAuthorMediaHealth.mockResolvedValue({
      items: [
        {
          work_id: 'work-1',
          work_title: 'The Long Way Home',
          total_references: 5,
          healthy_references: 3,
          at_risk_references: 1,
          broken_references: 1,
        },
      ],
    });
    render(AuthorMedia);
    await waitFor(() => {
      expect(screen.getByText('The Long Way Home')).toBeTruthy();
      expect(screen.getByText('5 refs')).toBeTruthy();
      expect(screen.getByText('3 healthy')).toBeTruthy();
      expect(screen.getByText('1 at risk')).toBeTruthy();
      expect(screen.getByText('1 broken')).toBeTruthy();
    });
  });

  it('switches to Insert tab and shows the form', async () => {
    render(AuthorMedia);
    await waitFor(() => screen.getByText('Insert Media'));
    fireEvent.click(screen.getByText('Insert Media'));
    expect(screen.getByText('Insert Media Reference')).toBeTruthy();
    expect(screen.getByText('Post a Targeted Bounty')).toBeTruthy();
  });

  it('switches to Preferences tab and loads prefs', async () => {
    render(AuthorMedia);
    await waitFor(() => screen.getByText('Preferences'));
    fireEvent.click(screen.getByText('Preferences'));
    await waitFor(() => {
      expect(screen.getByText('Auto-submit new media to Internet Archive')).toBeTruthy();
      expect(api.fetchMediaPreferences).toHaveBeenCalled();
    });
  });

  it('posts a reference from the insert form', async () => {
    api.postMediaReference.mockResolvedValue({ id: 'ref-1', link_id: 'link-1', status: 'pending' });
    render(AuthorMedia);
    await waitFor(() => screen.getByText('Insert Media'));
    fireEvent.click(screen.getByText('Insert Media'));

    fireEvent.input(screen.getByPlaceholderText('insert-work-uuid'), {
      target: { value: 'work-1' },
    });
    fireEvent.input(screen.getByPlaceholderText('https://...'), {
      target: { value: 'https://example.com/img.png' },
    });
    fireEvent.click(screen.getByText('Add Reference'));

    await waitFor(() => {
      expect(api.postMediaReference).toHaveBeenCalledWith(
        expect.objectContaining({ work_id: 'work-1', url: 'https://example.com/img.png' }),
      );
    });
  });
});
