import { render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import WorkPage from './WorkPage.svelte';
import { session } from '../lib/session.svelte';
import { ApiError } from '../lib/api';

// Mock the API module, keeping every real export (ApiError included) and
// overriding only the fetchers this page calls.
vi.mock('../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/api')>();
  return {
    ...actual,
    fetchWork: vi.fn(),
    fetchWorkPricing: vi.fn(),
    fetchReviews: vi.fn().mockResolvedValue({ items: [] }),
    purchaseWork: vi.fn(),
    getProgress: vi.fn().mockResolvedValue(null),
    upsertReview: vi.fn(),
    isAuthorWork: vi.fn().mockReturnValue(false),
  };
});

import { fetchWork, fetchWorkPricing, fetchReviews } from '../lib/api';

describe('WorkPage paywall', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    session.status = 'anonymous';
    session.me = null as any;
  });

  it('shows paywall when 403 CONTENT_RESTRICTED returned', async () => {
    (fetchWork as any).mockRejectedValue(
      new ApiError(403, 'CONTENT_RESTRICTED', 'This work is for purchase.'),
    );
    (fetchWorkPricing as any).mockResolvedValue({
      pricing: [{ model: 'purchase', price_minor: 500, currency: 'USD', public_at_offset: null }],
    });

    render(WorkPage, { props: { workId: 'work-1' } });

    await waitFor(() => {
      expect(screen.getByText('This work is for purchase')).toBeInTheDocument();
    });
    expect(screen.getByText('5 USD')).toBeInTheDocument();
    expect(screen.getByText('Buy to unlock full access.')).toBeInTheDocument();
  });

  it('shows sign-in prompt for anonymous users on paywall', async () => {
    (fetchWork as any).mockRejectedValue(
      new ApiError(403, 'CONTENT_RESTRICTED', 'This work is for purchase.'),
    );
    (fetchWorkPricing as any).mockResolvedValue({
      pricing: [{ model: 'purchase', price_minor: 300, currency: 'EUR', public_at_offset: null }],
    });

    render(WorkPage, { props: { workId: 'work-1' } });

    await waitFor(() => {
      expect(screen.getByText('Sign in to buy')).toBeInTheDocument();
    });
  });
});

/**
 * The work page's reviews section.
 *
 * The rule this test exists for is spec §9.2: a review that mentions spoilers is
 * revealed by a **deliberate click**, never automatically. The page has always
 * done that with a `<details>` element, and the browser journey drove it by hand
 * — but nothing automated covered it, so a refactor that swapped the `<details>`
 * for a `<div>` would have hidden nothing and revealed everything, silently.
 *
 * What is asserted is therefore the shape that makes the behaviour, because that
 * is what a regression would break:
 *
 *   * the body of a spoiler review is inside a `<details>` that is **closed**;
 *   * the summary says why, without repeating the spoiler;
 *   * a review that does not mention spoilers is not wrapped at all — the
 *     warning must stay meaningful, and one shown for everything is noise.
 *
 * jsdom has no layout engine and does not implement the `details` disclosure
 * itself, so this does not assert that the text is hidden on screen; it asserts
 * that the element which hides it is there, is a `details`, and starts closed.
 */

const WORK = {
  id: 'work-1',
  title: 'The Salt Road',
  summary: 'A cartographer walks inland.',
  language: 'en',
  rating: 'teen',
  visibility: 'public',
  completion: 'ongoing',
  published_at: '2026-09-01T00:00:00Z',
  show_public_ratings: true,
  authors: [{ handle: 'devwriter', display_name: 'Dev Writer' }],
  units: [{ id: 'unit-1', title: 'One — Low Tide', word_count: 74, ordinal: 1 }],
};

function review(overrides: Record<string, unknown>) {
  return {
    id: 'r-1',
    author_handle: 'reader',
    body: 'Loved this.',
    is_public: true,
    contains_spoilers: false,
    created_at: '2026-09-05T00:00:00Z',
    ...overrides,
  };
}

/**
 * The withheld-text state (`works.gated_metadata_public`).
 *
 * A gated work answers the record and refuses the text. The page must not turn
 * that into a broken site: the units are listed so a reader can see what exists,
 * and they are **not** links, because `/units/<id>` answers 404 and an
 * unexplained 404 reads as a bug rather than as a gate. Both halves are asserted
 * here — a test for the notice alone would pass on a page that still offered the
 * dead links, and a test for the links alone would pass on a page that said
 * nothing.
 */
describe('WorkPage withheld text', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    session.status = 'anonymous';
    session.me = null as any;
    (fetchWorkPricing as any).mockResolvedValue({ pricing: [] });
    (fetchReviews as any).mockResolvedValue({ items: [] });
  });

  it('lists the units without links, and says why', async () => {
    (fetchWork as any).mockResolvedValue({ ...WORK, content_withheld: true });

    render(WorkPage, { props: { workId: 'work-1' } });

    await waitFor(() => {
      expect(screen.getByTestId('content-withheld')).toBeInTheDocument();
    });
    // The unit is named...
    expect(screen.getByText('One — Low Tide')).toBeInTheDocument();
    // ...and is not a link to text the server will refuse.
    expect(screen.queryByRole('link', { name: 'One — Low Tide' })).toBeNull();
  });

  it('links the units when the text is readable', async () => {
    (fetchWork as any).mockResolvedValue(WORK);

    render(WorkPage, { props: { workId: 'work-1' } });

    await waitFor(() => {
      expect(screen.getByRole('link', { name: 'One — Low Tide' })).toBeInTheDocument();
    });
    expect(screen.queryByTestId('content-withheld')).toBeNull();
  });
});

describe('WorkPage spoiler reveal', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    session.status = 'anonymous';
    session.me = null as any;
  });

  it('wraps a spoiler review in a closed <details> element', async () => {
    const spoilerReview = review({ contains_spoilers: true, body: 'The butler did it.' });
    (fetchWork as any).mockResolvedValue(WORK);
    (fetchReviews as any).mockResolvedValue({ items: [spoilerReview] });

    render(WorkPage, { props: { workId: 'work-1' } });

    await waitFor(() => {
      const details = screen.getByText('This review mentions spoilers').closest('details');
      expect(details).not.toBeNull();
      expect((details as HTMLDetailsElement).open).toBe(false);
    });
  });

  it('does not wrap a non-spoiler review in <details>', async () => {
    const normalReview = review({ contains_spoilers: false });
    (fetchWork as any).mockResolvedValue(WORK);
    (fetchReviews as any).mockResolvedValue({ items: [normalReview] });

    render(WorkPage, { props: { workId: 'work-1' } });

    await waitFor(() => {
      expect(screen.getByText('Loved this.')).toBeInTheDocument();
    });
    // Should NOT be inside a <details> element
    const reviewText = screen.getByText('Loved this.');
    expect(reviewText.closest('details')).toBeNull();
  });
});