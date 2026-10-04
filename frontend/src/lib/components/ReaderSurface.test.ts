import { describe, expect, it } from 'vitest';
import { render } from '@testing-library/svelte';
import type { SurfaceWork } from '../api';
import MostBookmarkedThisWeek from './MostBookmarkedThisWeek.svelte';
import NewInYourFandoms from './NewInYourFandoms.svelte';
import SimilarWorksRail from './SimilarWorksRail.svelte';

/**
 * A minimal surface work.
 *
 * `recent_bookmarks` and `similarity` are optional on `SurfaceWork` so a fixture cannot
 * claim a measurement the server never took. `word_count` is NOT, and this fixture
 * compiling is the check: the server COALESCEs the aggregate to 0, so "no chapters" is a
 * true 0 and never an absent field. Making it optional to silence a type error here would
 * reintroduce the `undefined` the store deliberately does not produce.
 */
function work(id: string, title: string, extra: Partial<SurfaceWork> = {}): SurfaceWork {
  return {
    id,
    title,
    summary: 'a summary',
    completion: 'complete',
    word_count: 0,
    ...extra,
  };
}

describe('NewInYourFandoms', () => {
  it('renders nothing when there is nothing', () => {
    // The load-bearing case. A heading over an empty list teaches the reader that the
    // site has nothing new for them, which is a different claim from "you have no
    // fandoms yet" — and one the reader cannot check.
    const { container } = render(NewInYourFandoms, { props: { works: [] } });
    expect(container.textContent?.trim()).toBe('');
  });

  it('names the section when there is something', () => {
    const { getByText } = render(NewInYourFandoms, {
      props: { works: [work('a', 'New HP Fic')] },
    });
    expect(getByText('New in your fandoms')).toBeTruthy();
    expect(getByText('New HP Fic')).toBeTruthy();
  });

  it('says so when the reader has already saved the work', () => {
    // The section's promise is "new to you", and the promise is only credible if the
    // copy states the exclusion rather than leaving the reader to wonder.
    const { getByText } = render(NewInYourFandoms, {
      props: { works: [work('a', 'New HP Fic')] },
    });
    expect(getByText(/already have/i)).toBeTruthy();
  });

  it('shows the section while loading rather than flickering it away', () => {
    const { getByText } = render(NewInYourFandoms, {
      props: { works: [], loading: true },
    });
    expect(getByText('New in your fandoms')).toBeTruthy();
  });

  it('opens a work when asked', () => {
    let opened: string | null = null;
    const { getByRole } = render(NewInYourFandoms, {
      props: { works: [work('a', 'New HP Fic')], onopen: (id: string) => (opened = id) },
    });
    getByRole('button', { name: 'New HP Fic' }).click();
    expect(opened).toBe('a');
  });
});

describe('MostBookmarkedThisWeek', () => {
  it('renders nothing when the week has no public bookmarks', () => {
    const { container } = render(MostBookmarkedThisWeek, { props: { works: [] } });
    expect(container.textContent?.trim()).toBe('');
  });

  it('says the count is of PUBLIC bookmarks', () => {
    // The privacy rule, reader-facing. The server enforces it; the label has to
    // correspond, or the leaderboard's number quietly claims more than it counted.
    const { getByText } = render(MostBookmarkedThisWeek, {
      props: { works: [work('a', 'Stars Fall Softly', { recent_bookmarks: 12 })] },
    });
    expect(getByText(/public bookmarks/i)).toBeTruthy();
  });

  it('names the unit in the accessible text of a bare number', () => {
    const { getByLabelText } = render(MostBookmarkedThisWeek, {
      props: { works: [work('a', 'Stars Fall Softly', { recent_bookmarks: 12 })] },
    });
    expect(getByLabelText('12 public bookmarks')).toBeTruthy();
  });

  it('ranks in the order the server sent', () => {
    const { getAllByRole } = render(MostBookmarkedThisWeek, {
      props: {
        works: [
          work('a', 'First', { recent_bookmarks: 30 }),
          work('b', 'Second', { recent_bookmarks: 20 }),
          work('c', 'Third', { recent_bookmarks: 10 }),
        ],
      },
    });
    const names = getAllByRole('listitem').map((li) => li.textContent ?? '');
    expect(names[0]).toContain('First');
    expect(names[1]).toContain('Second');
    expect(names[2]).toContain('Third');
  });

  it('omits the count when the server did not send one', () => {
    // `recent_bookmarks` is optional on purpose: on this surface the server always
    // sets it, but a row without one must not render "0", which claims a measurement
    // of zero rather than an absent one.
    const { queryByText } = render(MostBookmarkedThisWeek, {
      props: { works: [work('a', 'Uncounted')] },
    });
    expect(queryByText('0')).toBeFalsy();
  });
});

describe('SimilarWorksRail', () => {
  it('renders nothing when nothing clears the honesty floor', () => {
    const { container } = render(SimilarWorksRail, { props: { works: [] } });
    expect(container.textContent?.trim()).toBe('');
  });

  it('names the subject work when the caller knows it', () => {
    const { getByText } = render(SimilarWorksRail, {
      props: { works: [work('a', 'A Match')], subjectTitle: 'Stars Fall Softly' },
    });
    expect(getByText('Similar to “Stars Fall Softly”')).toBeTruthy();
  });

  it('falls back to a plain heading with no subject title', () => {
    const { getByText } = render(SimilarWorksRail, { props: { works: [work('a', 'A Match')] } });
    expect(getByText('Similar works')).toBeTruthy();
  });

  it('shows the score rather than asserting similarity', () => {
    // 0.62 is what weighted Jaccard produced. Rendering it lets the reader weigh the
    // evidence; a bare "similar" is a claim with nothing behind it.
    const { getByLabelText } = render(SimilarWorksRail, {
      props: { works: [work('a', 'A Match', { similarity: 0.62 })] },
    });
    expect(getByLabelText('62% tag overlap')).toBeTruthy();
  });

  it('omits the score for a work the server did not score', () => {
    // A missing `similarity` rendered as 0% would read as "no overlap found" instead of
    // "not scored", which is a different statement about the work.
    const { queryByText } = render(SimilarWorksRail, {
      props: { works: [work('a', 'Unscored')] },
    });
    expect(queryByText('0%')).toBeFalsy();
  });

  it('renders at most the five works the server returned', () => {
    const works = Array.from({ length: 5 }, (_, i) =>
      work(`w-${i}`, `Match ${i}`, { similarity: 0.9 - i * 0.1 }),
    );
    const { getAllByRole } = render(SimilarWorksRail, { props: { works } });
    expect(getAllByRole('listitem')).toHaveLength(5);
  });
});

// ---------------------------------------------------------------------------
// Item 4: reading time on every rail
// ---------------------------------------------------------------------------

describe('reading length (item 4)', () => {
  /**
   * The count assertion comes FIRST, in the very first test below. Without it, every
   * "renders no length" assertion here would pass just as well against a component that
   * renders nothing at all — which is the false green this project has already shipped.
   */
  it('shows words and minutes on each rail that renders works', () => {
    const long = work('w1', 'Long Serial', { word_count: 12_000 });
    const mb = render(MostBookmarkedThisWeek, { props: { works: [long] } });
    // "12k", not "12.0k": the formatter keeps one decimal only below 10,000, and a
    // count that reads "12.0k" implies a precision the number does not have.
    expect(mb.getAllByTestId('work-length')[0].textContent).toContain('12k words');
    expect(mb.getAllByTestId('work-length')[0].textContent).toContain('~48 min');

    const feed = render(NewInYourFandoms, { props: { works: [long] } });
    expect(feed.getAllByTestId('work-length')[0].textContent).toContain('~48 min');

    const rail = render(SimilarWorksRail, { props: { works: [long] } });
    expect(rail.getAllByTestId('work-length')[0].textContent).toContain('~48 min');
  });

  it('rounds UP, so a 5.04-minute work does not claim five', () => {
    // The audit's formula is count / 250. 1260 words is 5.04 minutes: rounding to
    // nearest says "5 min", which is the direction a reader is hurt in.
    const { container } = render(MostBookmarkedThisWeek, {
      props: { works: [work('w1', 'Just Over', { word_count: 1260 })] },
    });
    expect(container.textContent).toContain('~6 min');
    expect(container.textContent).not.toContain('~5 min');
  });

  /**
   * 200 words, not 300. 300 / 250 is 1.2 and `ceil(1.2)` is 2, so a 300-word fixture
   * correctly says "~2 min" — my first version of this test asserted "about a minute"
   * against it and the code was right and the fixture was wrong. The boundary that
   * produces one minute is anything up to 250 words.
   */
  it('says "about a minute" rather than "~1 min", which reads like a measurement', () => {
    const { container } = render(MostBookmarkedThisWeek, {
      props: { works: [work('w1', 'A Drabble', { word_count: 200 })] },
    });
    expect(container.textContent).toContain('about a minute');
    expect(container.textContent).toContain('200 words');
    expect(container.textContent).not.toContain('~1 min');
  });

  it('shows "no words yet" for a zero count instead of hiding the length', () => {
    // 0 is a real value the server computed, not a missing field. Hiding it would make an
    // unwritten work indistinguishable from one the server failed to measure.
    const { container } = render(MostBookmarkedThisWeek, {
      props: { works: [work('w1', 'Unwritten', { word_count: 0 })] },
    });
    expect(container.textContent).toContain('no words yet');
  });

  it('renders no length line at all when the count is absent', () => {
    // The only case that produces nothing — and it needs an explicit `undefined`, because
    // the type does not allow a missing `word_count`. This is what a server rollback or an
    // older instance would look like.
    const w = work('w1', 'Unmeasured');
    (w as { word_count?: number }).word_count = undefined;
    const { container } = render(MostBookmarkedThisWeek, { props: { works: [w] } });
    expect(container.querySelectorAll('[data-testid=work-length]')).toHaveLength(0);
    expect(container.textContent).toContain('Unmeasured');
  });
});
