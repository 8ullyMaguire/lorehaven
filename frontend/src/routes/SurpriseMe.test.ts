import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';
import SurpriseMe from './SurpriseMe.svelte';
import { signInTestReader, resetTestSession } from '../lib/testing/session';
import { fetchSurpriseMe, ApiError } from '../lib/api';

/**
 * Item 7, Surprise Me.
 *
 * The property these tests exist for is the EMPTY STATE. The endpoint returns 200 with
 * `work: null` for two genuinely different situations — an empty public catalogue, and a
 * reader whose taste profile covers everything worth showing — and a page that renders one
 * message for both tells a reader with strong taste that the button is broken.
 *
 * Every assertion below therefore pins the wording, not just the presence of text: two of
 * these tests would still pass with a single "nothing available" if only one branch existed.
 */
vi.mock('../lib/api', async () => {
  const actual = await vi.importActual<typeof import('../lib/api')>('../lib/api');
  return { ...actual, fetchSurpriseMe: vi.fn() };
});

const mockFetch = vi.mocked(fetchSurpriseMe);

const WORK = {
  workId: 'work-abc',
  title: 'The Lighthouse at Kirill Point',
  summary: 'Two lighthouse keepers, one winter, and a letter that arrives forty years late.',
};

beforeEach(() => {
  signInTestReader();
  mockFetch.mockReset();
});

afterEach(() => {
  resetTestSession();
});

describe('SurpriseMe', () => {
  it('shows the picked work with its title and summary', async () => {
    mockFetch.mockResolvedValue({ work: WORK, profileEmpty: false });

    render(SurpriseMe);

    expect(await screen.findByRole('link', { name: WORK.title })).toBeTruthy();
    expect(screen.getByText(WORK.summary)).toBeTruthy();
  });

  it('links to the work page for the served work', async () => {
    mockFetch.mockResolvedValue({ work: WORK, profileEmpty: false });

    render(SurpriseMe);

    const link = await screen.findByRole('link', { name: WORK.title });
    expect(link.getAttribute('href')).toBe(`/works/${encodeURIComponent(WORK.workId)}`);
  });

  /**
   * The words have to distinguish the two empty states, because `profileEmpty` is the only
   * thing that tells them apart. Asserting on the specific sentence rather than on "some
   * text appeared" is what stops one branch from satisfying both tests.
   */
  it('blames the catalogue when the reader has no profile and there is nothing', async () => {
    mockFetch.mockResolvedValue({ work: null, profileEmpty: true });

    render(SurpriseMe);

    expect(
      await screen.findByText(/no public work published on this instance/i),
    ).toBeTruthy();
  });

  it('blames the profile — not the catalogue — when a reader with taste gets nothing', async () => {
    mockFetch.mockResolvedValue({ work: null, profileEmpty: false });

    render(SurpriseMe);

    expect(
      await screen.findByText(/everything here shares a tag with your profile/i),
    ).toBeTruthy();
    // The catalogue claim must NOT appear here: it would be false, and it would hide why
    // the button returns nothing.
    expect(screen.queryByText(/no public work published on this instance/i)).toBeNull();
  });

  it('says the pick was unconstrained when the reader has no profile', async () => {
    mockFetch.mockResolvedValue({ work: WORK, profileEmpty: true });

    render(SurpriseMe);

    expect(await screen.findByText(/no taste profile yet/i)).toBeTruthy();
    expect(screen.queryByText(/outside the tags your reading has weighted/i)).toBeNull();
  });

  it('says the pick came from outside the profile when the reader has one', async () => {
    mockFetch.mockResolvedValue({ work: WORK, profileEmpty: false });

    render(SurpriseMe);

    expect(await screen.findByText(/outside the tags your reading has weighted/i)).toBeTruthy();
  });

  it('shows an error and no work when the request fails', async () => {
    mockFetch.mockRejectedValue(new ApiError(500, 'INTERNAL', 'boom'));
    render(SurpriseMe);

    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
    expect(screen.queryByRole('link', { name: WORK.title })).toBeNull();
  });

  /**
   * The retry control, which is the only way a second request happens.
   *
   * This also pins something worth knowing about the component: `ErrorSummary` renders its
   * retry button INSIDE its error branch, so there is no button on a healthy page. A first
   * draft of this test clicked one there and failed on a missing element — which is the same
   * class of mistake as an absence assertion with nothing to be absent.
   */
  it('offers a retry that re-requests the pick', async () => {
    mockFetch.mockRejectedValueOnce(new ApiError(500, 'INTERNAL', 'boom'));
    render(SurpriseMe);

    const retry = await screen.findByRole('button', { name: /try again/i });
    mockFetch.mockResolvedValueOnce({ work: WORK, profileEmpty: false });
    await fireEvent.click(retry);

    expect(await screen.findByRole('link', { name: WORK.title })).toBeTruthy();
    expect(screen.queryByRole('alert')).toBeNull();
    expect(mockFetch).toHaveBeenCalledTimes(2);
  });

  /**
   * The stale-pick guard, stated as what is actually reachable.
   *
   * `load()` sets `result = null` before it can fail, but within one mounted page a second
   * request only follows an error, so `result` is already null by then. The reachable case
   * is the one the error test covers: a failure shows no work. Asserting a success followed
   * by a failure inside one instance would require a control that does not exist, and a test
   * written for it would pass without exercising anything.
   */
  it('shows no work at all while a request is failing', async () => {
    mockFetch.mockRejectedValue(new ApiError(500, 'INTERNAL', 'boom'));
    render(SurpriseMe);

    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
    expect(screen.queryByRole('link')).toBeNull();
    expect(screen.queryByRole('heading', { level: 2 })).toBeNull();
  });

  it('requests the pick exactly once on mount', async () => {
    mockFetch.mockResolvedValue({ work: WORK, profileEmpty: false });

    render(SurpriseMe);
    await screen.findByRole('link', { name: WORK.title });

    // There is deliberately no reroll control, so a second call would be a bug rather than
    // a feature — the surface is a small deliberate departure, not a random-work generator.
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });
});
