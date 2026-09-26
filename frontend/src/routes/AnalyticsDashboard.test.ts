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
});
