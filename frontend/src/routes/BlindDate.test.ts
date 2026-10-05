import { render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import BlindDate from './BlindDate.svelte';
import { signInTestReader, resetTestSession } from '../lib/testing/session';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchBlindDate: vi.fn(),
  };
});

import { fetchBlindDate } from '../lib/api';

const mocked = vi.mocked(fetchBlindDate);

/**
 * What these tests are actually about.
 *
 * Blind Date is a surface whose value is that it shows the reader *nothing*. Two of the
 * four assertions below therefore assert the absence of information, which is the unusual
 * direction: the failure mode is a component that helpfully fetches the work's title,
 * author and tags, at which point the reader can screen by fandom or word count before
 * committing and the mechanism is gone. Nothing here would catch that except a test that
 * looks for the metadata and requires it not to be there.
 */

beforeEach(() => {
  signInTestReader();
});

afterEach(() => {
  resetTestSession();
});

describe('BlindDate', () => {
  beforeEach(() => {
    mocked.mockReset();
  });

  it('offers a single link and reveals nothing about the work', async () => {
    mocked.mockResolvedValue({ date: '2026-10-02', workId: 'work-42' });

    render(BlindDate);

    const link = await screen.findByRole('link', { name: /open today's work/i });
    // `/works/{id}`, the same path Discover links to. A blind date that 404s is the one
    // failure a reader cannot forgive, so this pins the shared convention rather than a
    // path invented for this page.
    expect(link).toHaveAttribute('href', '/works/work-42');

    // The point of the surface: no title, no author, no word count, no fandom. A reader
    // who can see any of that can screen before they commit, and "chosen without looking
    // at your profile" becomes a suggestion with extra steps.
    const text = document.body.textContent ?? '';
    for (const leak of ['work-42 title', 'Author', 'tags', 'words', 'fandom']) {
      expect(text).not.toMatch(new RegExp(leak, 'i'));
    }
  });

  it('offers no way to ask for a different work', async () => {
    mocked.mockResolvedValue({ date: '2026-10-02', workId: 'work-42' });

    render(BlindDate);
    await screen.findByRole('link', { name: /open today's work/i });

    // No "another", "shuffle", "reroll", or "next". The pick is deterministic in
    // (account, day) precisely so it cannot be rerolled, and a button would undo that.
    expect(screen.queryByRole('button')).toBeNull();
    expect(document.body.textContent ?? '').not.toMatch(/shuffle|another|reroll|next/i);
  });

  it('renders an empty state rather than an error when there is no work today', async () => {
    // The endpoint returns 200 with a null id, so this is a normal day, not a failure.
    // An error component here would tell the reader something is broken.
    mocked.mockResolvedValue({ date: '2026-10-02', workId: null });

    render(BlindDate);

    await waitFor(() => {
      expect(screen.getByText(/nothing to offer today/i)).toBeInTheDocument();
    });
    expect(screen.queryByRole('link')).toBeNull();
    expect(document.body.textContent ?? '').not.toMatch(/error|failed|try again/i);
  });

  it('offers a retry when the request actually fails', async () => {
    mocked.mockRejectedValue(new Error('network down'));

    render(BlindDate);

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /try again|retry/i })).toBeInTheDocument();
    });
    // And no stale link: an error beside yesterday's work reads as today's pick.
    expect(screen.queryByRole('link')).toBeNull();
  });
});