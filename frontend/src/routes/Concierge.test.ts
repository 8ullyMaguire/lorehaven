import { render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Concierge from './Concierge.svelte';

vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchConciergeQueue: vi.fn(),
    watchWork: vi.fn(),
    unwatchWork: vi.fn(),
  };
});

import { ApiError, fetchConciergeQueue, unwatchWork, watchWork } from '../lib/api';

const QUEUE = {
  session_id: 'session-1',
  items: [
    {
      work_id: 'work-1',
      title: 'The Long Gate',
      estimated_minutes: 40,
      reason: { kind: 'blend' as const },
    },
    {
      work_id: 'work-2',
      title: 'Small Weather',
      estimated_minutes: 25,
      reason: { kind: 'mood' as const, mood: 'comfort' },
    },
  ],
  estimated_minutes: 65,
  truncated_at: null,
  rate_source: 'default' as const,
  explained_empty: null,
};

beforeEach(() => {
  vi.clearAllMocks();
  (fetchConciergeQueue as any).mockResolvedValue(QUEUE);
  (watchWork as any).mockResolvedValue({ work_id: 'work-1', pending: false, notified: true });
  (unwatchWork as any).mockResolvedValue({ removed: true });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('Concierge page', () => {
  // §54.7. This is the whole feature's claim: a reader who has chosen nothing sees
  // the discovery feed. The first render MUST therefore send no selector at all —
  // a page that preselects "60 min" on mount has quietly replaced the feed with a
  // budgeted queue, and every other test here would still pass.
  it('asks for no selector on first render', async () => {
    render(Concierge);
    await waitFor(() => expect(fetchConciergeQueue).toHaveBeenCalled());
    expect((fetchConciergeQueue as any).mock.calls[0][0]).toEqual({});
  });

  // §54.4: the total shown is the work actually served, and the provenance of the
  // reading rate is stated rather than assumed.
  it('shows the served total and says the rate is a default', async () => {
    render(Concierge);
    await waitFor(() => expect(screen.getByText(/65 min/)).toBeTruthy());
    expect(screen.getByText(/typical reading pace/)).toBeTruthy();
  });

  it('does not claim a personal rate when there is none', async () => {
    (fetchConciergeQueue as any).mockResolvedValue({ ...QUEUE, rate_source: 'observed' });
    render(Concierge);
    await waitFor(() => expect(screen.getByText(/65 min/)).toBeTruthy());
    expect(screen.queryByText(/typical reading pace/)).toBeNull();
  });

  // §54.6's central case, and the one a UI is most likely to get wrong by looking
  // like it works. An empty list has two causes; this page must not merge them.
  it('renders the server explanation for a selector that matched nothing', async () => {
    (fetchConciergeQueue as any).mockResolvedValue({
      session_id: 'session-2',
      items: [],
      estimated_minutes: 0,
      truncated_at: null,
      rate_source: 'default',
      explained_empty: 'no work in this instance carries the mood "grief"',
    });
    render(Concierge);
    await waitFor(() =>
      expect(screen.getByText(/no work in this instance carries the mood/)).toBeTruthy(),
    );
  });

  // The other half of §54.6, and the reason `truncated_at` is a separate field:
  // "nothing fit" is not "nothing matched", and a reader told the first when the
  // second happened cannot act on it.
  it('distinguishes an empty budget from an empty match', async () => {
    (fetchConciergeQueue as any).mockResolvedValue({
      session_id: 'session-3',
      items: [],
      estimated_minutes: 0,
      truncated_at: 0,
      rate_source: 'default',
      explained_empty: null,
    });
    render(Concierge);
    await waitFor(() => expect(screen.getByText(/Nothing fits the time you gave/)).toBeTruthy());
    expect(screen.queryByText(/carries the mood/)).toBeNull();
  });

  // §54.6 again: an empty queue must not be papered over with the feed. There is
  // an explicit affordance to leave the selector, and it is opt-in.
  it('does not silently substitute the feed for an empty queue', async () => {
    (fetchConciergeQueue as any).mockResolvedValue({
      session_id: 'session-4',
      items: [],
      estimated_minutes: 0,
      truncated_at: null,
      rate_source: 'default',
      explained_empty: 'nothing matched',
    });
    render(Concierge);
    await waitFor(() => expect(screen.getByText(/nothing matched/)).toBeTruthy());
    // The queue was asked for exactly once: nothing refetched itself behind the
    // reader's back.
    expect((fetchConciergeQueue as any).mock.calls.length).toBe(1);
  });

  it('sends a chosen mood and a chosen budget together', async () => {
    const { fireEvent } = await import('@testing-library/svelte');
    render(Concierge);
    await waitFor(() => expect(fetchConciergeQueue).toHaveBeenCalled());

    // `getAllByRole` + destructure would depend on how many buttons precede it in
    // the DOM; `getByRole` names the one the test means.
    await fireEvent.click(screen.getByRole('button', { name: '30 min' }));

    await waitFor(() =>
      expect((fetchConciergeQueue as any).mock.calls.at(-1)?.[0]).toEqual({ minutes: 30 }),
    );
    // `mood` is absent, not `''` — a present-but-blank mood is a different request.
    expect((fetchConciergeQueue as any).mock.calls.at(-1)?.[0]).not.toHaveProperty('mood');
  });

  // §54.5: watching is the page's job, and the server decides whether the
  // notification has already fired.
  it('watches a work from the queue and reloads', async () => {
    const { fireEvent } = await import('@testing-library/svelte');
    render(Concierge);
    await waitFor(() => expect(screen.getByText(/The Long Gate/)).toBeTruthy());

    // The FIRST item's button. One per item, so the count is the queue length —
    // asserted rather than indexed, because an index into `getAllByRole` is a
    // silent off-by-one when an item is added to the fixture.
    const buttons = screen.getAllByRole('button', { name: /Tell me when it ends/ });
    expect(buttons).toHaveLength(QUEUE.items.length);
    await fireEvent.click(buttons[0]);

    await waitFor(() => expect(watchWork).toHaveBeenCalledWith('work-1'));
    // And the page reflects the new state rather than assuming it.
    expect((fetchConciergeQueue as any).mock.calls.length).toBeGreaterThan(1);
  });

  // §54.6's transparency, in the item itself: a reader who asked for a mood and is
  // handed something from the general feed is entitled to know.
  it('says why each item is in the queue', async () => {
    render(Concierge);
    await waitFor(() => expect(screen.getByText(/because you asked for comfort/)).toBeTruthy());
    // `blend` is named rather than hidden.
    expect(screen.getByText(/from your feed/)).toBeTruthy();
  });

  // §54.4: an item with no known length says so, rather than rendering a zero or
  // silently vanishing.
  it('labels an item of unknown length instead of hiding it', async () => {
    (fetchConciergeQueue as any).mockResolvedValue({
      ...QUEUE,
      items: [{ work_id: 'work-3', title: 'Draft Thing', estimated_minutes: null, reason: { kind: 'blend' } }],
    });
    render(Concierge);
    await waitFor(() => expect(screen.getByText(/Draft Thing/)).toBeTruthy());
    expect(screen.getByText(/length unknown/)).toBeTruthy();
  });

  // §54.4's cut, marked in place. A reader given less than they asked for should
  // not have to count rows to find out where it stopped.
  it('marks where the budget cut the queue', async () => {
    (fetchConciergeQueue as any).mockResolvedValue({ ...QUEUE, truncated_at: 1 });
    render(Concierge);
    await waitFor(() => expect(screen.getByText(/cut here/)).toBeTruthy());
  });
});

describe('Concierge mood refusal (spec §54.2, §54.6)', () => {
  /**
   * The vocabulary problem.
   *
   * §15.8's moods are author-assigned, so the list of moods a reader can ask for is
   * whatever this instance's writers have used. It is NOT a field on the queue
   * response -- the only place it appears is inside the 422 the server returns when
   * it refuses a mood, which is why this feature reads it out of the error rather
   * than from a separate endpoint.
   *
   * These four tests are the whole feature. Before them, the component declared
   * `knownMoods` and `moodError`, set `moodError = null`, and rendered neither --
   * svelte-check caught `knownMoods` as unused. A mood a reader typed was refused
   * with a red banner and no way forward.
   */

  /** The 422 the route returns for an unknown mood, built the way the server builds it. */
  function refusal(requested: string, available: string[]): ApiError {
    const list = available.length ? available.join(', ') : 'this instance carries no moods yet';
    const message =
      available.length === 0
        ? `no work on this instance carries the mood ${JSON.stringify(requested)}; this instance carries no moods yet`
        : `no work on this instance carries the mood ${JSON.stringify(requested)}; the moods on this instance are: ${list}`;
    return new ApiError(422, 'VALIDATION', message, null, { mood: message });
  }

  it('shows the server\'s own sentence naming the moods that do exist', async () => {
    (fetchConciergeQueue as any).mockRejectedValue(
      refusal('melancholy', ['comfort', 'longing']),
    );
    render(Concierge);

    // Verbatim, not paraphrased: the message is the server's and it names both what
    // was asked for and what exists.
    await waitFor(() =>
      expect(screen.getAllByText(/the moods on this instance are: comfort, longing/i).length)
        .toBeGreaterThan(0),
    );
  });

  it('offers the available moods as one-click buttons', async () => {
    (fetchConciergeQueue as any).mockRejectedValue(
      refusal('melancholy', ['comfort', 'longing']),
    );
    render(Concierge);

    await waitFor(() => expect(screen.getAllByRole('button', { name: 'comfort' }).length).toBeGreaterThan(0));
    expect(screen.getAllByRole('button', { name: 'longing' }).length).toBeGreaterThan(0);

    // Clicking one re-requests WITH that mood. That is the assertion that matters:
    // the list is not decoration, it is the next move.
    await screen.getAllByRole('button', { name: 'comfort' })[0].click();
    await waitFor(() =>
      expect((fetchConciergeQueue as any).mock.calls.at(-1)[0]).toEqual({ mood: 'comfort' }),
    );
  });

  it('does NOT show a red error summary for a refused mood', async () => {
    (fetchConciergeQueue as any).mockRejectedValue(
      refusal('melancholy', ['comfort']),
    );
    render(Concierge);

    await waitFor(() =>
      expect(screen.getAllByText(/moods on this instance/i).length).toBeGreaterThan(0),
    );
    // A refused mood is the system answering a question, not a failure. ErrorSummary
    // would tell the reader something broke and offer a retry that cannot help.
    expect(document.querySelector('[role=alert]')).toBeNull();
  });

  it('still shows ErrorSummary for a real failure', async () => {
    // The complement, and the one that keeps the rule above honest: a 500 on the
    // same request IS a failure and must not be swallowed into the mood path.
    (fetchConciergeQueue as any).mockRejectedValue(
      new ApiError(500, 'INTERNAL', 'the queue could not be built'),
    );
    render(Concierge);

    await waitFor(() =>
      expect(screen.getAllByText(/the queue could not be built/i).length).toBeGreaterThan(0),
    );
    expect(document.querySelector('[role=alert]')).not.toBeNull();
    // And no vocabulary was invented from it.
    expect(screen.queryAllByRole('button', { name: 'comfort' }).length).toBe(0);
  });

  it('treats a 500 that ALSO carries fieldErrors.mood as a failure, not a refusal', async () => {
    // Written because widening the status gate to `>= 400` left all 16 tests green.
    //
    // The 500 in the previous test carries no field errors, so it cannot tell
    // "422 about a mood" from "some other status about a mood" -- the two rules are
    // indistinguishable through it. A server that returns 500 WITH a per-field
    // message is ordinary (a validation that fails mid-handler, a wrapped upstream
    // error), and swallowing that into an inline mood sentence with no retry would
    // hide a real failure behind a helpful-looking message.
    //
    // So the fixture carries `fieldErrors.mood` too, and the only thing separating
    // the two paths is the status.
    (fetchConciergeQueue as any).mockRejectedValue(
      new ApiError(500, 'INTERNAL', 'the queue could not be built', null, {
        mood: 'the mood index is unavailable',
      }),
    );
    render(Concierge);

    await waitFor(() =>
      expect(screen.getAllByText(/the queue could not be built/i).length).toBeGreaterThan(0),
    );
    expect(document.querySelector('[role=alert]')).not.toBeNull();
    // No vocabulary was learned from a failure.
    expect(screen.queryAllByRole('button', { name: 'comfort' }).length).toBe(0);
  });

  it('drops the learned list once a queue succeeds again', async () => {
    (fetchConciergeQueue as any).mockRejectedValueOnce(
      refusal('melancholy', ['comfort']),
    );
    (fetchConciergeQueue as any).mockResolvedValueOnce(QUEUE);
    render(Concierge);

    await waitFor(() => expect(screen.getAllByRole('button', { name: 'comfort' }).length).toBeGreaterThan(0));

    // Ask for something the instance does have; the queue comes back.
    const input = screen.getAllByLabelText(/mood, if you feel like naming one/i)[0] as HTMLInputElement;
    await input.focus();
    await (await import('@testing-library/svelte')).fireEvent.input(input, { target: { value: 'longing' } });
    await (await import('@testing-library/svelte')).fireEvent.keyDown(input, { key: 'Enter' });

    await waitFor(() =>
      expect((fetchConciergeQueue as any).mock.calls.at(-1)[0]).toEqual({ mood: 'longing' }),
    );
    await waitFor(() =>
      expect(screen.getAllByText('The Long Gate').length).toBeGreaterThan(0),
    );
    // The vocabulary was learned from a refusal about a mood the reader no longer
    // has selected. Left in place it would sit under the selector indefinitely,
    // implying the instance offers exactly those two moods.
    await waitFor(() => expect(screen.queryAllByRole('button', { name: 'comfort' }).length).toBe(0));
  });
});
