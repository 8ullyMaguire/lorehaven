import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Notifications from './Notifications.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchNotifications: vi.fn(),
    markAllNotificationsRead: vi.fn(),
    markNotificationRead: vi.fn(),
  };
});

import {
  fetchNotifications,
  markAllNotificationsRead,
  markNotificationRead,
} from '../lib/api';

const NOTIFICATIONS = {
  items: [
    {
      id: 'notif-1',
      kind: 'library_update',
      title: 'A work you follow was updated',
      body: 'New chapter',
      work_id: 'work-1',
      read: false,
      created_at: '2026-09-21T12:00:00Z',
    },
  ],
  unread_count: 1,
};

beforeEach(() => {
  vi.clearAllMocks();
  (fetchNotifications as any).mockResolvedValue(NOTIFICATIONS);
  (markAllNotificationsRead as any).mockResolvedValue({});
  (markNotificationRead as any).mockResolvedValue({});
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('Notifications page', () => {
  it('renders notifications and unread count', async () => {
    render(Notifications);
    await waitFor(() =>
      expect(screen.getByText('A work you follow was updated')).toBeInTheDocument(),
    );
    expect(screen.getByText('1 new')).toBeInTheDocument();
  });

  it('shows the mark-all-as-read button', async () => {
    render(Notifications);
    await waitFor(() =>
      expect(screen.getByText('A work you follow was updated')).toBeInTheDocument(),
    );
    expect(screen.getByText('Mark all as read')).toBeInTheDocument();
  });

  it('shows the empty state when there are no notifications', async () => {
    (fetchNotifications as any).mockResolvedValue({ items: [], unread_count: 0 });
    render(Notifications);
    await waitFor(() => expect(screen.getByText('No notifications yet.')).toBeInTheDocument());
  });

  it('marks all as read when the button is clicked', async () => {
    render(Notifications);
    await waitFor(() =>
      expect(screen.getByText('A work you follow was updated')).toBeInTheDocument(),
    );
    await screen.getByText('Mark all as read').click();
    await waitFor(() => expect(markAllNotificationsRead).toHaveBeenCalled());
  });
});
