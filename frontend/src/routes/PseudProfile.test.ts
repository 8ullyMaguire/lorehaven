import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import PseudProfile from './PseudProfile.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchPublicPseud: vi.fn(),
  };
});

import { fetchPublicPseud } from '../lib/api';

const PROFILE = {
  id: 'pseud-1',
  handle: 'alice',
  display_name: 'Alice',
  bio: 'Writer of stories',
  created_at: '2026-01-01T00:00:00Z',
};

beforeEach(() => {
  vi.clearAllMocks();
  (fetchPublicPseud as any).mockResolvedValue(PROFILE);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('PseudProfile page', () => {
  it('renders the pseud handle and display name', async () => {
    render(PseudProfile, { props: { handle: 'alice' } });
    await waitFor(() => expect(screen.getByText('Alice')).toBeInTheDocument());
  });

  it('fetches the public pseud on mount', async () => {
    render(PseudProfile, { props: { handle: 'alice' } });
    await waitFor(() => expect(fetchPublicPseud).toHaveBeenCalledWith('alice'));
  });

  it('renders the bio', async () => {
    render(PseudProfile, { props: { handle: 'alice' } });
    await waitFor(() => expect(screen.getByText('Writer of stories')).toBeInTheDocument());
  });

  it('renders the handle', async () => {
    render(PseudProfile, { props: { handle: 'alice' } });
    await waitFor(() => expect(screen.getByText('Alice')).toBeInTheDocument());
    expect(screen.getByText('@alice')).toBeInTheDocument();
  });
});
