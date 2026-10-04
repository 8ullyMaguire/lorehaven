/**
 * Item 9: "Why this?" on a recommended item.
 *
 * The order of these tests matters more than usual. The very first one COUNTS the trigger
 * with a slot id before any test asserts its absence — an absence assertion against a
 * component that renders nothing at all is vacuously true, and that is the failure mode
 * this file has to not repeat.
 */
import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';

/**
 * `vi.mock` is HOISTED, `vi.doMock` is not. The component under test imports
 * `fetchSlotExplanation` at module load, so `doMock` in `beforeEach` registered too late
 * and every click reached the real client — which failed, and the test suite then reported
 * four "cannot find why-reasons" failures that looked like component bugs.
 */
const fetchSlotExplanation = vi.fn();
vi.mock('../api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api')>();
  return { ...actual, fetchSlotExplanation };
});

const { default: WhyRecommended } = await import('./WhyRecommended.svelte');
const { ApiError } = await import('../api');

/** One recorded slot, as the server serialises it. */
function slot(reasons: string[] = ['taste_tags', 'popular']) {
  return {
    slot_id: 'slot-1',
    work_id: 'work-1',
    position: 0,
    reasons,
    blend_score: 0,
    served_at: '2026-10-04T19:41:24Z',
  };
}

beforeEach(() => {
  // A default per test, overridable after render — which is what the failure-state tests
  // need, since they cannot know the outcome until they have mounted the component.
  fetchSlotExplanation.mockReset();
  fetchSlotExplanation.mockImplementation(async () => slot());
});

describe('WhyRecommended', () => {
  it('offers the trigger when the server recorded a slot id', async () => {
    // The COUNT first. Everything below asserts about this node's absence, and an absence
    // assertion is only meaningful once we know the node exists in some state.
    render(WhyRecommended, { props: { slotId: 'slot-1' } });
    expect(screen.getAllByTestId('why-trigger')).toHaveLength(1);
  });

  it('renders nothing at all when the server recorded no slot id', () => {
    // The server warns-and-continues when its slot write fails, so this is a HEALTHY
    // server under some conditions — not an error state.
    const { container } = render(WhyRecommended, { props: {} });
    expect(container.querySelectorAll('[data-testid=why-trigger]')).toHaveLength(0);
    expect(container.textContent?.trim()).toBe('');
  });

  it('asks for the explanation only when clicked, not on render', async () => {
    render(WhyRecommended, { props: { slotId: 'slot-1' } });
    // The load-bearing claim: a disclosure must not cost a request for every reader who
    // never asks. If this fetched eagerly, the feed would make N extra requests a paint.
    expect(fetchSlotExplanation).not.toHaveBeenCalled();
  });

  it('fetches the explanation on click and shows each reason in plain words', async () => {
    render(WhyRecommended, { props: { slotId: 'slot-1' } });
    await screen.getByTestId('why-trigger').click();

    await waitFor(() => expect(screen.getByTestId('why-reasons')).toBeTruthy());
    const text = screen.getByTestId('why-reasons').textContent ?? '';
    // The wire form is `taste_tags`; what a reader needs is the sentence. Asserting the
    // raw code would pass while the UI showed nothing useful.
    expect(text).toContain('matches tags your reading has weighted');
    expect(text).toContain('popular on this instance');
    expect(text).not.toContain('taste_tags');
    expect(fetchSlotExplanation).toHaveBeenCalledWith('slot-1', expect.anything());
  });

  it('does not re-request a slot it has already explained', async () => {
    // The recording is immutable once written, so a second click answers from memory.
    render(WhyRecommended, { props: { slotId: 'slot-1' } });
    const trigger = screen.getByTestId('why-trigger');
    await trigger.click();
    await waitFor(() => expect(screen.getByTestId('why-reasons')).toBeTruthy());
    await trigger.click(); // close
    await trigger.click(); // reopen
    await waitFor(() => expect(screen.getByTestId('why-reasons')).toBeTruthy());
    expect(fetchSlotExplanation).toHaveBeenCalledTimes(1);
  });

  it('retires the trigger when the client reports no such slot', async () => {
    // §3.3: a 404 covers both "no such slot" and "not yours", and the door must not
    // distinguish them. So there is nothing left to ask about.
    //
    // The mock resolves `null`, which is what `fetchSlotExplanation` RETURNS for a 404 —
    // it catches the ApiError itself. Throwing here instead tested the client's own 404
    // mapping rather than the component's handling of "no explanation", and the component's
    // catch-all correctly read the throw as a network failure. Mocking the layer the
    // component actually consumes is the whole difference.
    fetchSlotExplanation.mockImplementation(async () => null);
    render(WhyRecommended, { props: { slotId: 'slot-1' } });
    await screen.getByTestId('why-trigger').click();

    await waitFor(() =>
      expect(screen.queryAllByTestId('why-trigger')).toHaveLength(0),
    );
  });

  it('keeps the trigger on a NETWORK failure, because the slot may well exist', async () => {
    // The distinction that matters: 404 means "no explanation", a 500 means "ask again".
    // Retiring the trigger on a 500 would delete a working feature because of a blip.
    fetchSlotExplanation.mockImplementation(async () => {
      throw new ApiError(500, 'INTERNAL', 'boom');
    });
    render(WhyRecommended, { props: { slotId: 'slot-1' } });
    await screen.getByTestId('why-trigger').click();

    await waitFor(() => expect(screen.getByTestId('why-retry')).toBeTruthy());
    expect(screen.getAllByTestId('why-trigger')).toHaveLength(1);
  });

  it('passes an unknown reason code through instead of hiding it', async () => {
    // The vocabulary is closed TODAY. A new server reason must not render as a blank
    // bullet, which is what a lookup miss silently produces.
    fetchSlotExplanation.mockImplementation(async () => slot(['taste_tags', 'future_reason']));
    render(WhyRecommended, { props: { slotId: 'slot-1' } });
    await screen.getByTestId('why-trigger').click();

    await waitFor(() => expect(screen.getByTestId('why-reasons')).toBeTruthy());
    const text = screen.getByTestId('why-reasons').textContent ?? '';
    expect(text).toContain('future_reason');
    // And the count assertion on the reasons themselves: an empty <ul> would satisfy a
    // toBeTruthy on the container.
    expect(screen.getAllByTestId('why-reasons')[0].querySelectorAll('li')).toHaveLength(2);
  });

  it('marks the trigger expanded only while the reasons are showing', async () => {
    render(WhyRecommended, { props: { slotId: 'slot-1' } });
    const trigger = screen.getByTestId('why-trigger');
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    await trigger.click();
    await waitFor(() => expect(screen.getByTestId('why-reasons')).toBeTruthy());
    expect(screen.getAllByTestId('why-trigger')[0].getAttribute('aria-expanded')).toBe('true');
  });
});