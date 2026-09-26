import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor, cleanup, fireEvent } from '@testing-library/svelte';
import AnalyticsDashboard from './AnalyticsDashboard.svelte';
import * as api from '../lib/api';

/**
 * The dashboard's job is to render what the registry says, and nothing else.
 *
 * The tests are about that and about the two ways a suppression turns back
 * into a number: a client that formats a missing count as 0, and a client
 * that hardcodes the floor instead of reading it. Both produce a confident
 * wrong number on a surface nobody is looking at.
 */

const meta = (over: Partial<Record<string, unknown>> = {}) => ({
  name: 'own.reading.basic',
  definition: 'Words and chapters the viewer completed over the window.',
  freshness: 'near-real-time (minutes)',
  approximation: 'Session length and pace are estimates.',
  minimum_trust_level: 0,
  subject: 'self',
  floor: 5,
  ...over,
});

function mockList(capabilities: unknown[]) {
  vi.spyOn(api, 'fetchAnalytics').mockResolvedValue({
    viewer: { trust_level: 0, role: 'reader', preset: 'archive' },
    capabilities,
  } as never);
}

describe('AnalyticsDashboard', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });
  afterEach(() => {
    cleanup();
  });

  it('loads and renders the capabilities the server allows', async () => {
    mockList([
      meta(),
      meta({ name: 'own.reading.distribution', minimum_trust_level: 1 }),
    ]);
    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByText('own.reading.basic')).toBeTruthy());
    expect(screen.getByText('own.reading.distribution')).toBeTruthy();
  });

  it('renders nothing that the server did not send', async () => {
    // The client does not know the ladder. If it rendered a local list, a
    // capability the server withheld would appear anyway.
    mockList([meta()]);
    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByText('own.reading.basic')).toBeTruthy());
    expect(screen.queryByText('own.work.retention')).toBeNull();
    expect(screen.queryByText('steward.moderation_queue')).toBeNull();
  });

  it('shows the definition so a number is never unexplained', async () => {
    mockList([meta()]);
    render(AnalyticsDashboard);
    await waitFor(() =>
      expect(
        screen.getByText('Words and chapters the viewer completed over the window.'),
      ).toBeTruthy(),
    );
  });

  it('shows how fresh the number is', async () => {
    mockList([meta()]);
    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByText(/near-real-time/)).toBeTruthy());
  });

  it('states the trust level the capability needs', async () => {
    mockList([meta({ name: 'own.work.retention', minimum_trust_level: 1 })]);
    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByText(/trust level 1/i)).toBeTruthy());
  });

  it('says a suppressed count is below the floor rather than showing zero', async () => {
    vi.spyOn(api, 'fetchAnalytics').mockResolvedValue({
      viewer: { trust_level: 1, role: 'reader', preset: 'archive' },
      capabilities: [meta({ name: 'own.work.retention', subject: 'other', floor: 10 })],
    } as never);
    vi.spyOn(api, 'fetchCapability').mockResolvedValue({
      capability: 'own.work.retention',
      implemented: true,
      meta: meta({ name: 'own.work.retention', subject: 'other', floor: 10 }),
      value: { fewer_than: 10 },
    } as never);

    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByText('own.work.retention')).toBeTruthy());
    await fireEvent.click(screen.getByText('own.work.retention'));

    await waitFor(() => expect(screen.getByText(/fewer than 10/i)).toBeTruthy());
    expect(screen.queryByText('0')).toBeNull();
  });

  it('renders an exact count when the server gives one', async () => {
    vi.spyOn(api, 'fetchAnalytics').mockResolvedValue({
      viewer: { trust_level: 1, role: 'reader', preset: 'archive' },
      capabilities: [meta({ name: 'own.work.retention', subject: 'other', floor: 10 })],
    } as never);
    vi.spyOn(api, 'fetchCapability').mockResolvedValue({
      capability: 'own.work.retention',
      implemented: true,
      meta: meta({ name: 'own.work.retention', subject: 'other', floor: 10 }),
      value: { count: 42 },
    } as never);

    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByText('own.work.retention')).toBeTruthy());
    await fireEvent.click(screen.getByText('own.work.retention'));
    await waitFor(() => expect(screen.getByText('42')).toBeTruthy());
  });

  it('reads the floor from the server rather than hardcoding one', async () => {
    // A client with 10 baked in will render "fewer than 10" on a surface whose
    // floor is 5, which is both wrong and more revealing than the truth.
    vi.spyOn(api, 'fetchAnalytics').mockResolvedValue({
      viewer: { trust_level: 0, role: 'reader', preset: 'archive' },
      capabilities: [meta({ floor: 5 })],
    } as never);
    vi.spyOn(api, 'fetchCapability').mockResolvedValue({
      capability: 'own.reading.basic',
      implemented: true,
      meta: meta({ floor: 5 }),
      value: { fewer_than: 5 },
    } as never);

    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByText('own.reading.basic')).toBeTruthy());
    await fireEvent.click(screen.getByText('own.reading.basic'));
    await waitFor(() => expect(screen.getByText(/fewer than 5/i)).toBeTruthy());
  });

  it('distinguishes a capability that is not built yet from one that is withheld', async () => {
    // Withheld never reaches the client as 403 here — it is simply not in the
    // list. "Not built yet" is in the list and says so, and a dashboard that
    // rendered the two identically would look broken.
    vi.spyOn(api, 'fetchAnalytics').mockResolvedValue({
      viewer: { trust_level: 0, role: 'reader', preset: 'archive' },
      capabilities: [meta()],
    } as never);
    vi.spyOn(api, 'fetchCapability').mockResolvedValue({
      capability: 'own.reading.basic',
      implemented: false,
      meta: meta(),
      value: { status: 'not_implemented' },
    } as never);

    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByText('own.reading.basic')).toBeTruthy());
    await fireEvent.click(screen.getByText('own.reading.basic'));
    await waitFor(() => expect(screen.getByText(/not.*built yet|not implemented/i)).toBeTruthy());
  });

  it('tells the reader who is asking what they are seeing', async () => {
    vi.spyOn(api, 'fetchAnalytics').mockResolvedValue({
      viewer: { trust_level: 3, role: 'reader', preset: 'archive' },
      capabilities: [meta()],
    } as never);
    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByText(/trust level 3/i)).toBeTruthy());
  });

  it('shows an error state rather than an empty dashboard', async () => {
    vi.spyOn(api, 'fetchAnalytics').mockRejectedValue(new Error('boom'));
    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
  });

  it('does not show a loading spinner forever when the list is empty', async () => {
    mockList([]);
    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByText(/nothing here yet|nothing to show/i)).toBeTruthy());
  });

  it('marks a capability that is about other people, so the floor is explained', async () => {
    // The floor only makes sense next to "this counts other people", and a
    // reader who is told "fewer than 10" with no subject wonders who the ten
    // are.
    mockList([meta({ subject: 'other', floor: 10 })]);
    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByText(/about other people/i)).toBeTruthy());
  });
  // -- the first capability with named fields -------------------------------
  //
  // `own.reading.basic` is the first capability that answers with named fields
  // rather than one count. The tests below are about the two ways that goes
  // wrong: a client that renders the wrong capability's shape, and a client
  // that shows a raw estimate as though it were exact.

  function mockReading(value: unknown) {
    mockList([meta()]);
    vi.spyOn(api, 'fetchCapability').mockResolvedValue({
      capability: 'own.reading.basic',
      implemented: true,
      meta: meta(),
      value,
    } as never);
  }

  it('renders the reading totals as named fields, not one number', async () => {
    mockReading({
      status: 'ok',
      reading: {
        finished_works: 7,
        chapters_read: 132,
        words_read: 480000,
        reading_seconds: 9000,
      },
    });

    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByText('own.reading.basic')).toBeTruthy());
    await fireEvent.click(screen.getByText('own.reading.basic'));

    await waitFor(() => expect(screen.getByText('Works finished')).toBeTruthy());
    expect(screen.getByText('7')).toBeTruthy();
    expect(screen.getByText('132')).toBeTruthy();
  });

  it('shows reading time as a duration rather than raw seconds', async () => {
    // 9000 seconds is 2 h 30 min. A bare "9000" tells a reader nothing and
    // invites them to do the division themselves.
    mockReading({
      status: 'ok',
      reading: {
        finished_works: 1,
        chapters_read: 4,
        words_read: 9000,
        reading_seconds: 9000,
      },
    });

    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByText('own.reading.basic')).toBeTruthy());
    await fireEvent.click(screen.getByText('own.reading.basic'));

    await waitFor(() => expect(screen.getByText('2 h 30 min')).toBeTruthy());
    expect(screen.queryByText('9000')).toBeNull();
  });

  it('gives words a magnitude instead of six raw digits', async () => {
    mockReading({
      status: 'ok',
      reading: {
        finished_works: 1,
        chapters_read: 4,
        words_read: 480000,
        reading_seconds: 600,
      },
    });

    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByText('own.reading.basic')).toBeTruthy());
    await fireEvent.click(screen.getByText('own.reading.basic'));

    await waitFor(() => expect(screen.getByText('480.0 thousand')).toBeTruthy());
  });

  it('shows a zero reading history as zeros, not as an empty panel', async () => {
    // These count the viewer's own behaviour, so a zero is a true zero. A
    // client that renders nothing for a zero has turned "you have read
    // nothing" into "we are not telling you", which is a different claim.
    mockReading({
      status: 'ok',
      reading: {
        finished_works: 0,
        chapters_read: 0,
        words_read: 0,
        reading_seconds: 0,
      },
    });

    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByText('own.reading.basic')).toBeTruthy());
    await fireEvent.click(screen.getByText('own.reading.basic'));

    await waitFor(() => expect(screen.getByText('Works finished')).toBeTruthy());
    expect(screen.getByText('0 min')).toBeTruthy();
  });

  it('does not render reading fields for a different capability', async () => {
    // The shape is keyed on the capability name, not on "the value has these
    // fields" -- otherwise the next capability with a `reading` key would be
    // rendered with this one's labels.
    mockList([meta({ name: 'own.reading.trend' })]);
    vi.spyOn(api, 'fetchCapability').mockResolvedValue({
      capability: 'own.reading.trend',
      implemented: true,
      meta: meta({ name: 'own.reading.trend' }),
      value: {
        status: 'ok',
        reading: {
          finished_works: 3,
          chapters_read: 9,
          words_read: 1000,
          reading_seconds: 300,
        },
      },
    } as never);

    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByText('own.reading.trend')).toBeTruthy());
    await fireEvent.click(screen.getByText('own.reading.trend'));

    // The point is the labels, not a particular message: `own.reading.trend`
    // carries a `reading` key that this client does not own, and rendering it
    // with the basic shape would tell the reader these are works finished when
    // they are something else entirely. The value block is empty here because
    // the server sent no band and no count -- which is a separate gap (a
    // capability with a shape this client cannot render should say so, not
    // render nothing), and is not what this test is about.
    await waitFor(() => expect(screen.getByText('Session length and pace are estimates.')).toBeTruthy());
    expect(screen.queryByText('Works finished')).toBeNull();
    expect(screen.queryByText('Chapters read')).toBeNull();
    expect(screen.queryByText('Time reading')).toBeNull();
  });
  // -- signed out ------------------------------------------------------------
  //
  // The door is `RequirePseud`, so an anonymous visitor gets a 401. Rendering
  // that as a failure string tells somebody they can do nothing about, on a
  // page that is specifically about *their own* numbers.

  it('offers a way in when nobody is signed in, rather than an error', async () => {
    const { ApiError } = await import('../lib/api');
    vi.spyOn(api, 'fetchAnalytics').mockRejectedValue(new ApiError(401, 'AUTH_REQUIRED', 'no session'));

    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByText(/need an account/i)).toBeTruthy());
    expect(screen.getByRole('link', { name: 'Sign in' })).toBeTruthy();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('still reports a genuine server failure as an error', async () => {
    // The signed-out branch must not swallow a 500. A page that answers
    // "please sign in" when the server is down sends the reader to the wrong
    // door.
    const { ApiError } = await import('../lib/api');
    vi.spyOn(api, 'fetchAnalytics').mockRejectedValue(new ApiError(500, 'INTERNAL', 'boom'));

    render(AnalyticsDashboard);
    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
    expect(screen.queryByText(/need an account/i)).toBeNull();
  });
});
