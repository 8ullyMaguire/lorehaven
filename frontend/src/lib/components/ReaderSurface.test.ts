import { describe, expect, it } from 'vitest';
import { render } from '@testing-library/svelte';
import type { SurfaceWork } from '../api';
import MostBookmarkedThisWeek from './MostBookmarkedThisWeek.svelte';
import NewInYourFandoms from './NewInYourFandoms.svelte';
import SimilarWorksRail from './SimilarWorksRail.svelte';

/**
 * A minimal surface work. Only the fields the rails actually read are required, which is
 * the point: `SurfaceWork` makes `recent_bookmarks` and `similarity` optional precisely
 * so a fixture cannot accidentally claim a measurement the server never took.
 */
function work(id: string, title: string, extra: Partial<SurfaceWork> = {}): SurfaceWork {
  return {
    id,
    title,
    summary: 'a summary',
    completion: 'complete',
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