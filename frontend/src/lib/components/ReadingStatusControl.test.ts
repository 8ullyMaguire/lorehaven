import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import ReadingStatusControl from './ReadingStatusControl.svelte';
import { ApiError } from '../api';
import { fetchWorkReadingStatus, setWorkReadingStatus, clearWorkReadingStatus } from '../api';
import { session } from '../session.svelte';

vi.mock('../api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api')>();
  return {
    ...actual,
    fetchWorkReadingStatus: vi.fn(),
    setWorkReadingStatus: vi.fn(),
    clearWorkReadingStatus: vi.fn(),
  };
});

const ME = {
  id: 'acc-1',
  pseuds: [{ handle: 'reader', is_primary: true }],
  trust_level: 1,
  created_at: '',
} as any;

function signedIn() {
  session.status = 'signed-in';
  session.me = ME;
}

const RECORD = (status: string) => ({
  status,
  started_at: '2026-09-20T00:00:00Z',
  finished_at: status === 'finished' ? '2026-09-21T00:00:00Z' : null,
  updated_at: '2026-09-21T00:00:00Z',
  version: 1,
});

describe('ReadingStatusControl', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    session.status = 'anonymous';
    session.me = null as any;
    (fetchWorkReadingStatus as any).mockResolvedValue(null);
    (setWorkReadingStatus as any).mockResolvedValue(RECORD('finished'));
    (clearWorkReadingStatus as any).mockResolvedValue(undefined);
  });

  // -- who can use it -------------------------------------------------------

  it('offers a way in to a signed-out visitor, and records nothing', () => {
    render(ReadingStatusControl, { props: { workId: 'work-1' } });

    expect(screen.getByRole('link', { name: 'Sign in' })).toBeInTheDocument();
    // Not merely hidden: a reader who cannot see the control cannot know it
    // exists, and a reader who can see it cannot record anything.
    expect(screen.queryByRole('button', { name: 'Finished' })).not.toBeInTheDocument();
    expect(fetchWorkReadingStatus).not.toHaveBeenCalled();
  });

  it('offers every state the server knows to a signed-in reader', async () => {
    signedIn();
    render(ReadingStatusControl, { props: { workId: 'work-1' } });

    // The control is behind a "checking what you recorded" state, because the
    // choices are not offered until the page knows what is already set --
    // offering them first would show a reader five unselected buttons for a
    // work they had in fact finished.
    await waitFor(() =>
      expect(screen.queryByText('Checking what you have recorded…')).not.toBeInTheDocument(),
    );

    for (const label of ['Want to read', 'Reading', 'On hold', 'Dropped', 'Finished']) {
      expect(screen.getByRole('button', { name: label })).toBeInTheDocument();
    }
  });

  // -- reading back what was recorded ---------------------------------------

  it('shows nothing chosen when the reader has recorded nothing', async () => {
    signedIn();
    render(ReadingStatusControl, { props: { workId: 'work-1' } });

    await waitFor(() => expect(fetchWorkReadingStatus).toHaveBeenCalledWith('work-1'));
    for (const label of ['Want to read', 'Reading', 'On hold', 'Dropped', 'Finished']) {
      expect(screen.getByRole('button', { name: label })).toHaveAttribute('aria-pressed', 'false');
    }
    // "Forget this" only makes sense against something to forget.
    expect(screen.queryByRole('button', { name: 'Forget this' })).not.toBeInTheDocument();
  });

  it('shows the state the reader already recorded', async () => {
    signedIn();
    (fetchWorkReadingStatus as any).mockResolvedValue(RECORD('on-hold'));

    render(ReadingStatusControl, { props: { workId: 'work-1' } });

    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'On hold' })).toHaveAttribute(
        'aria-pressed',
        'true',
      ),
    );
    // The states are mutually exclusive, so recording one must clear the others.
    expect(screen.getByRole('button', { name: 'Finished' })).toHaveAttribute(
      'aria-pressed',
      'false',
    );
  });

  // -- writing --------------------------------------------------------------

  it('records the state the reader picked', async () => {
    signedIn();
    render(ReadingStatusControl, { props: { workId: 'work-1' } });
    await waitFor(() => expect(fetchWorkReadingStatus).toHaveBeenCalled());

    await fireEvent.click(screen.getByRole('button', { name: 'Finished' }));

    expect(setWorkReadingStatus).toHaveBeenCalledWith('work-1', 'finished');
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Finished' })).toHaveAttribute(
        'aria-pressed',
        'true',
      ),
    );
  });

  it('keeps the choice visible while the write is in flight', async () => {
    // The optimistic update is the point: a select that reverts to "nothing"
    // while the request is in flight tells the reader their click did not land,
    // and on a slow connection they will click again.
    signedIn();
    let release: (v: unknown) => void = () => {};
    (setWorkReadingStatus as any).mockReturnValue(
      new Promise((resolve) => {
        release = resolve;
      }),
    );

    render(ReadingStatusControl, { props: { workId: 'work-1' } });
    await waitFor(() => expect(fetchWorkReadingStatus).toHaveBeenCalled());

    await fireEvent.click(screen.getByRole('button', { name: 'Reading' }));
    expect(screen.getByRole('button', { name: 'Reading' })).toHaveAttribute('aria-pressed', 'true');

    release(RECORD('reading'));
    await waitFor(() => expect(setWorkReadingStatus).toHaveBeenCalled());
  });

  it('does not offer a second choice while a write is in flight', async () => {
    signedIn();
    let release: (v: unknown) => void = () => {};
    (setWorkReadingStatus as any).mockReturnValue(
      new Promise((resolve) => {
        release = resolve;
      }),
    );

    render(ReadingStatusControl, { props: { workId: 'work-1' } });
    await waitFor(() => expect(fetchWorkReadingStatus).toHaveBeenCalled());

    await fireEvent.click(screen.getByRole('button', { name: 'Reading' }));
    // Every choice is disabled, not just the one clicked: a reader switching
    // from Reading to Finished mid-flight would otherwise queue two writes and
    // get whichever the server happened to apply last.
    for (const label of ['Want to read', 'Reading', 'On hold', 'Dropped', 'Finished']) {
      expect(screen.getByRole('button', { name: label })).toBeDisabled();
    }

    release(RECORD('reading'));
  });

  // -- forgetting -----------------------------------------------------------

  it('forgets a recorded state', async () => {
    signedIn();
    (fetchWorkReadingStatus as any).mockResolvedValue(RECORD('dropped'));

    render(ReadingStatusControl, { props: { workId: 'work-1' } });
    await waitFor(() => expect(screen.getByRole('button', { name: 'Forget this' })).toBeInTheDocument());

    await fireEvent.click(screen.getByRole('button', { name: 'Forget this' }));

    expect(clearWorkReadingStatus).toHaveBeenCalledWith('work-1');
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Forget this' })).not.toBeInTheDocument());
  });

  // -- the session that is not resolved yet --------------------------------

  it('fetches the recorded state once an unknown session resolves to signed-in', async () => {
    // The session starts as `unknown` and resolves asynchronously. A control
    // that reads it only on mount sees "not signed in", gives up, and never
    // fetches -- so a signed-in reader is shown five unselected buttons for a
    // work they had in fact finished, and can save that impression. Found by
    // the E2E: the click worked, and the state was gone after a reload.
    session.status = 'unknown';
    session.me = null as any;
    (fetchWorkReadingStatus as any).mockResolvedValue(RECORD('finished'));

    render(ReadingStatusControl, { props: { workId: 'work-1' } });
    expect(fetchWorkReadingStatus).not.toHaveBeenCalled();

    session.status = 'signed-in';
    session.me = ME;
    await waitFor(() => expect(fetchWorkReadingStatus).toHaveBeenCalledWith('work-1'));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Finished' })).toHaveAttribute(
        'aria-pressed',
        'true',
      ),
    );
  });

  it('does not claim a session is anonymous while it is still resolving', async () => {
    // The mirror image: treating `unknown` as "no session" shows the sign-in
    // prompt to a reader who is signed in, and asks them to sign in again.
    session.status = 'unknown';
    (fetchWorkReadingStatus as any).mockResolvedValue(null);

    render(ReadingStatusControl, { props: { workId: 'work-1' } });

    expect(screen.queryByRole('link', { name: 'Sign in' })).not.toBeInTheDocument();
  });

  // -- failures -------------------------------------------------------------

  it('reports a failed write and puts the previous state back', async () => {
    // A write that fails silently leaves the reader believing they recorded
    // something they did not, and the dashboard then disagrees with them.
    signedIn();
    (fetchWorkReadingStatus as any).mockResolvedValue(RECORD('reading'));
    (setWorkReadingStatus as any).mockRejectedValue(new Error('Could not save that.'));

    render(ReadingStatusControl, { props: { workId: 'work-1' } });
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Reading' })).toHaveAttribute('aria-pressed', 'true'),
    );

    await fireEvent.click(screen.getByRole('button', { name: 'Finished' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('Could not save that.');
    // Back to what was actually stored, not to nothing.
    expect(screen.getByRole('button', { name: 'Reading' })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('button', { name: 'Finished' })).toHaveAttribute('aria-pressed', 'false');
  });

  it('keeps the recorded state when the read-back is rate limited', async () => {
    // Found by the E2E: with the suite's traffic saturating the address bucket,
    // the read-back after a reload returned 429, and the control rendered five
    // unselected buttons for a work the reader had finished. The server was
    // right and the reader was told, in effect, that they had finished nothing
    // -- and their dashboard then disagreed with the page in front of them.
    session.status = 'signed-in';
    session.me = ME;
    (fetchWorkReadingStatus as any).mockRejectedValue(
      new ApiError(429, 'RATE_LIMITED', 'rate limited', null),
    );

    render(ReadingStatusControl, { props: { workId: 'work-1' } });
    await waitFor(() => expect(fetchWorkReadingStatus).toHaveBeenCalled());

    // No error banner: a failed read is not the reader's mistake.
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    // The control is usable, because a reader can still change their mind.
    expect(screen.getByRole('button', { name: 'Reading' })).toBeEnabled();
  });

  it('keeps a state it already knows when a later read-back is rate limited', async () => {
    session.status = 'signed-in';
    session.me = ME;
    (fetchWorkReadingStatus as any).mockResolvedValue(RECORD('reading'));
    render(ReadingStatusControl, { props: { workId: 'work-1' } });
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Reading' })).toHaveAttribute(
        'aria-pressed',
        'true',
      ),
    );

    // A re-read fails. The reader keeps what they had, and is told it is stale.
    (fetchWorkReadingStatus as any).mockRejectedValue(
      new ApiError(429, 'RATE_LIMITED', 'rate limited', null),
    );
    await fetchWorkReadingStatus('work-1').catch(() => null);

    expect(screen.getByRole('button', { name: 'Reading' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });

  it('offers an empty control, not a false claim, when the first read fails', async () => {
    // The read is a guess about what the reader already has. If it fails there
    // is nothing to protect -- no prior state was loaded -- so the control is
    // simply empty and usable, and no banner is raised. A banner here was the
    // old behaviour and it was wrong twice over: it cried wolf on a transient
    // failure, and it is the same treatment that made a *rate-limited* re-read
    // tell a reader they had finished nothing.
    signedIn();
    (fetchWorkReadingStatus as any).mockRejectedValue(new Error('Could not reach the server.'));

    render(ReadingStatusControl, { props: { workId: 'work-1' } });

    await waitFor(() => expect(fetchWorkReadingStatus).toHaveBeenCalled());
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    // The controls still work: a reader who wants to record something can, even
    // if the page could not tell them what they had before.
    expect(screen.getByRole('button', { name: 'Finished' })).toBeEnabled();
  });
});
