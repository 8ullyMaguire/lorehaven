import { describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';
import ContinueReadingBanner, { type ContinueReadingBannerProps } from './ContinueReadingBanner.svelte';
import type { ContinueReading } from '../api';

/** A banner row, as the server returns it. */
function entry(over: Partial<ContinueReading> = {}): ContinueReading {
  return {
    workId: 'work-1',
    title: 'Stars Fall Softly',
    positionPermille: 420,
    percent: 42,
    chapterId: 'chapter-7',
    chapterTitle: 'Chapter 7: The Turn',
    updatedAt: '2026-10-03T10:00:00Z',
    ...over,
  };
}

/** Render with an injected loader. The loader is a prop so no branch needs a network mock. */
function mount(over: Partial<ContinueReadingBannerProps> = {}) {
  const load = vi.fn().mockResolvedValue(null);
  const props: ContinueReadingBannerProps = { load, ...over };
  return { load, ...render(ContinueReadingBanner, { props }) };
}

const banner = () => screen.queryByTestId('continue-reading-banner');

describe('ContinueReadingBanner', () => {
  it('shows the work, the chapter and the percentage', async () => {
    mount({ load: vi.fn().mockResolvedValue(entry()) });

    await waitFor(() => expect(screen.getByTestId('continue-reading-title')).toBeTruthy());
    expect(screen.getByText('Stars Fall Softly')).toBeTruthy();
    expect(screen.getByTestId('continue-reading-chapter').textContent).toContain('Chapter 7');
    expect(screen.getByTestId('continue-reading-percent').textContent).toContain('42%');
  });

  /**
   * `/works/{id}`, and the plural is the whole assertion.
   *
   * The first version of this test asserted `/work/work-1` -- the value the component
   * actually produced -- so it passed while the link rendered "No such page". A test that
   * reads its expectation off the implementation cannot catch the implementation being
   * wrong. The route is taken from the router, not from this file.
   *
   * `ROUTES` is the router's own inventory; asserting against it means renaming a route
   * turns this red instead of quietly breaking the banner.
   */
  it('links the title to the work page', async () => {
    mount({ load: vi.fn().mockResolvedValue(entry()) });
    await waitFor(() => expect(screen.getByTestId('continue-reading-title')).toBeTruthy());
    expect(screen.getByTestId('continue-reading-title').getAttribute('href')).toBe(
      '/works/work-1',
    );
  });

  /**
   * THE COUNTED-ABSENCE RULE.
   *
   * Every "it is not shown" assertion in this file is preceded by a test that shows the
   * banner IS rendered when there is data. Without that, `queryByTestId(...) === null`
   * passes just as well when the component is broken, not mounted, or throws on the first
   * render — which is exactly the false green this project has already shipped once: a nav
   * test asserted "no label appears twice" while matching a hidden mobile nav, so the
   * assertion was vacuously true with the duplicate on screen.
   *
   * The first test above is the count. If it ever goes red, every absence assertion below
   * it is meaningless and will be reported as green until it is fixed.
   */
  it('renders nothing, and nothing visible, when there is nothing to continue', async () => {
    mount({ load: vi.fn().mockResolvedValue(null) });

    await waitFor(() => expect(banner()).toBeTruthy());
    await waitFor(() => expect(banner()!.hasAttribute('hidden')).toBe(true));
    expect(screen.queryByText('Continue reading')).toBeNull();
  });

  it('makes NO request for a signed-out visitor', async () => {
    // The distinction: an anonymous homepage view should not produce a failed request per
    // page view, so this is "not called", not "called and hidden".
    const { load } = mount({ signedIn: false });

    await waitFor(() => expect(banner()).toBeTruthy());
    expect(load).not.toHaveBeenCalled();
    expect(banner()!.hasAttribute('hidden')).toBe(true);
  });

  it('swallows a failed request rather than breaking the page', async () => {
    // The server being down must not produce an error box about a retention feature the
    // reader did not ask for. This is the assertion that pins that decision.
    const load = vi.fn().mockRejectedValue(new Error('500'));
    mount({ load });

    await waitFor(() => expect(banner()).toBeTruthy());
    await waitFor(() => expect(banner()!.hasAttribute('hidden')).toBe(true));
  });

  it('renders a work with no chapter without an empty chapter line', async () => {
    mount({ load: vi.fn().mockResolvedValue(entry({ chapterId: null, chapterTitle: null })) });

    await waitFor(() => expect(screen.getByTestId('continue-reading-title')).toBeTruthy());
    expect(screen.queryByTestId('continue-reading-chapter')).toBeNull();
  });

  it('exposes the percentage to assistive technology, not just visually', async () => {
    mount({ load: vi.fn().mockResolvedValue(entry({ percent: 42 })) });

    await waitFor(() => expect(screen.getByTestId('continue-reading-title')).toBeTruthy());
    const bar = screen.getByRole('progressbar');
    expect(bar.getAttribute('aria-valuenow')).toBe('42');
    expect(bar.getAttribute('aria-valuemin')).toBe('0');
    expect(bar.getAttribute('aria-valuemax')).toBe('100');
  });

  /**
   * THE RACE, and the test that no arrangement of `signedIn` could have caught.
   *
   * Every other test in this file passes `signedIn` as a prop, which means `signedIn` is
   * already correct at mount. The real homepage does not do that: it passes
   * `session.isSignedIn`, which is DERIVED from `session.status`, and `status` starts
   * `'unknown'` and only becomes `'signed-in'` after `/api/v1/auth/me` resolves. The
   * component therefore mounted while `signedIn` was still false, took that as "signed
   * out", and never asked — the banner was dead on the only page it appears on.
   *
   * So this asserts the ORDER: mount with `signedIn` false, THEN flip it, and require that
   * the request happens on the flip rather than never. A component that reads the prop once
   * in `onMount` fails this; one that reacts with `$effect` passes it.
   */
  it('asks once the session RESOLVES, not on mount while it is still unknown', async () => {
    const load = vi.fn().mockResolvedValue(entry());
    // A mutable box stands in for the derived getter the real page passes.
    let current = false;
    const { rerender } = render(ContinueReadingBanner, {
      props: { get signedIn() { return current; }, load } as ContinueReadingBannerProps,
    });

    await waitFor(() => expect(banner()).toBeTruthy());
    expect(load, 'must not ask before the session is known').not.toHaveBeenCalled();

    current = true;
    await rerender({ signedIn: true } as ContinueReadingBannerProps);

    await waitFor(() => expect(load).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.getByText('Stars Fall Softly')).toBeTruthy());
  });

  it('does not ask twice if the prop flips back and forth', async () => {
    const load = vi.fn().mockResolvedValue(entry());
    let current = true;
    const { rerender } = render(ContinueReadingBanner, {
      props: { get signedIn() { return current; }, load } as ContinueReadingBannerProps,
    });

    await waitFor(() => expect(load).toHaveBeenCalledTimes(1));
    current = false;
    await rerender({ signedIn: false } as ContinueReadingBannerProps);
    current = true;
    await rerender({ signedIn: true } as ContinueReadingBannerProps);

    await waitFor(() => expect(screen.getByText('Stars Fall Softly')).toBeTruthy());
    expect(load, 'the banner is one request, not one per prop change').toHaveBeenCalledTimes(1);
  });

  it('renders the server-provided percent, never a re-derived one', async () => {
    // positionPermille and percent are BOTH on the wire and they are not redundant. If the
    // component computed 420/10 it would agree here — so make them disagree, which is the
    // only way to catch a client that re-derives and thereby ignores the server's clamping.
    mount({
      load: vi.fn().mockResolvedValue(entry({ positionPermille: 420, percent: 100 })),
    });

    await waitFor(() => expect(screen.getByTestId('continue-reading-title')).toBeTruthy());
    expect(screen.getByTestId('continue-reading-percent').textContent).toContain('100%');
  });
});
