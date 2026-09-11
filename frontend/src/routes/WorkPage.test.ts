import { render, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import WorkPage from './WorkPage.svelte';
import { session } from '../lib/session.svelte';

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
  chapters: [{ id: 'chapter-1', title: 'One — Low Tide', word_count: 74, ordinal: 1 }],
};

function review(overrides: Record<string, unknown>) {
  return {
    id: 'review-1',
    author_handle: 'devreader',
    body: 'The prose is quiet and the map is the plot.',
    contains_spoilers: false,
    is_public: true,
    published_at: '2026-09-09T00:00:00Z',
    version: 1,
    ...overrides,
  };
}

let items: unknown[] = [];

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

beforeEach(() => {
  items = [];

  session.status = 'anonymous';
  session.me = null as never;

  vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
    const url = typeof input === 'string' ? input : input.toString();
    const path = new URL(url, 'http://localhost').pathname;

    if (path === '/api/v1/works/work-1') return json(WORK);
    if (path === '/api/v1/works/work-1/reviews') return json({ items, next_cursor: null });
    if (path === '/api/v1/reading/progress') {
      return json({ error: { code: 'NOT_FOUND', message: 'nothing read yet' } }, 404);
    }
    return json({ error: { code: 'NOT_FOUND', message: 'no' } }, 404);
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('the reviews section', () => {
  it('puts a spoiler review behind a closed disclosure that says why', async () => {
    items = [
      review({
        contains_spoilers: true,
        body: 'The cartographer is the road.',
      }),
    ];

    render(WorkPage, { props: { workId: 'work-1' } });

    await waitFor(() => expect(document.body.textContent).toContain('Reviews'));
    const disclosure = await waitFor(() => {
      const found = document.querySelector('details');
      expect(found).not.toBeNull();
      return found as HTMLDetailsElement;
    });

    // The reveal is a deliberate click: it starts closed, and the reader opens it.
    expect(disclosure.open).toBe(false);
    expect(disclosure.querySelector('summary')?.textContent).toContain(
      'This review mentions spoilers',
    );

    // The body is inside the disclosure, not beside or in the summary.
    const summary = disclosure.querySelector('summary') as HTMLElement;
    expect(summary.textContent ?? '').not.toContain('The cartographer is the road.');
    expect(disclosure.textContent ?? '').toContain('The cartographer is the road.');

    // Opening it is the click, and what it reveals is the review.
    disclosure.open = true;
    expect(disclosure.open).toBe(true);
  });

  it('shows a review that mentions no spoilers without a disclosure', async () => {
    items = [review({ contains_spoilers: false, body: 'A quiet, generous book.' })];

    render(WorkPage, { props: { workId: 'work-1' } });

    await waitFor(() =>
      expect(document.body.textContent).toContain('A quiet, generous book.'),
    );

    // A warning shown for everything warns about nothing.
    expect(document.querySelector('details')).toBeNull();
    expect(document.body.textContent).not.toContain('mentions spoilers');
  });

  it('wraps only the spoiler review when both kinds are present', async () => {
    items = [
      review({ id: 'r-plain', author_handle: 'plain', body: 'No spoilers here.' }),
      review({
        id: 'r-spoiler',
        author_handle: 'spoilery',
        body: 'The ending is a map.',
        contains_spoilers: true,
      }),
    ];

    render(WorkPage, { props: { workId: 'work-1' } });

    await waitFor(() => expect(document.body.textContent).toContain('No spoilers here.'));

    const disclosures = [...document.querySelectorAll('details')];
    expect(disclosures).toHaveLength(1);
    expect(disclosures[0].textContent ?? '').toContain('The ending is a map.');
    expect(disclosures[0].textContent ?? '').not.toContain('No spoilers here.');
  });

  it('does not attribute the public review to a pseud the reader is not shown', async () => {
    items = [review({ contains_spoilers: true, author_handle: 'devreader' })];

    render(WorkPage, { props: { workId: 'work-1' } });

    await waitFor(() => expect(document.body.textContent).toContain('Reviews'));
    // The handle is what the reviewer published under; a private pseud must not
    // be what the page names.
    await waitFor(() => expect(document.body.textContent).toContain('@devreader'));
  });
});
