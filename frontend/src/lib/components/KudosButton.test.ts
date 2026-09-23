import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import KudosButton from './KudosButton.svelte';
import { toggleKudos } from '../api';
import { session } from '../session.svelte';

vi.mock('../api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api')>();
  return {
    ...actual,
    toggleKudos: vi.fn(),
  };
});

describe('KudosButton', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    session.status = 'anonymous';
    session.me = null as any;
  });

  it('shows sign-in prompt for anonymous users', () => {
    render(KudosButton, { props: { workId: 'work-1' } });
    expect(screen.getByText('Sign in to leave kudos')).toBeInTheDocument();
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });

  it('shows kudos button for signed-in users', () => {
    session.status = 'signed-in';
    session.me = { id: 'acc-1', pseuds: [{ handle: 'reader', is_primary: true }], trust_level: 1, created_at: '' } as any;
    render(KudosButton, { props: { workId: 'work-1' } });
    expect(screen.getByText('Kudos')).toBeInTheDocument();
    expect(screen.queryByText('Sign in to leave kudos')).not.toBeInTheDocument();
  });

  it('toggles to kudoed state after click', async () => {
    session.status = 'signed-in';
    session.me = { id: 'acc-1', pseuds: [{ handle: 'reader', is_primary: true }], trust_level: 1, created_at: '' } as any;
    (toggleKudos as any).mockResolvedValue({ kudoed: true });

    render(KudosButton, { props: { workId: 'work-1' } });

    const button = screen.getByRole('button');
    expect(button.textContent).toBe('Kudos');
    expect(button.getAttribute('aria-pressed')).toBe('false');

    await fireEvent.click(button);

    await waitFor(() => {
      expect(screen.getByText('Kudoed ♥')).toBeInTheDocument();
      expect(screen.getByRole('button').getAttribute('aria-pressed')).toBe('true');
    });
  });

  it('does not toggle while busy', async () => {
    session.status = 'signed-in';
    session.me = { id: 'acc-1', pseuds: [{ handle: 'reader', is_primary: true }], trust_level: 1, created_at: '' } as any;
    let resolvePromise: (value: any) => void;
    (toggleKudos as any).mockImplementation(
      () => new Promise((resolve) => { resolvePromise = resolve; }),
    );

    render(KudosButton, { props: { workId: 'work-1' } });

    const button = screen.getByRole('button') as HTMLButtonElement;
    await fireEvent.click(button);
    expect(button.disabled).toBe(true);

    // Click again while busy — should not fire another request
    await fireEvent.click(button);
    expect(toggleKudos).toHaveBeenCalledTimes(1);

    resolvePromise!({ kudoed: true });
  });
});
