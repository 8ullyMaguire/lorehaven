import { describe, expect, it } from 'vitest';
import { render } from '@testing-library/svelte';
import WorkCard, { type WorkSummary } from './WorkCard.svelte';

/** A minimal work summary for the metric bar to render from. */
function sampleWork(overrides: Partial<WorkSummary> = {}): WorkSummary {
  return {
    id: 'w-1',
    title: 'A Test Work',
    authorDisplayName: 'Alice',
    completion: 'complete',
    rating: 'general',
    centralRelationships: [],
    wordCount: 5000,
    chapters: 3,
    ...overrides,
  };
}

describe('WorkCard', () => {
  it('shows the metric bar when metrics are present', () => {
    const { getByText } = render(WorkCard, {
      props: { work: sampleWork({ metrics: { views: 1200, kudos: 42, reactions: 5, complete_reads: 10, bookmarks: 3, collection_adds: 2, reviews: 1 } }) },
    });
    expect(getByText('1.2k views')).toBeTruthy();
    expect(getByText('42 kudos')).toBeTruthy();
  });

  it('hides the metric bar when the owner opted out', () => {
    const { queryByText } = render(WorkCard, {
      props: { work: sampleWork({ metrics: null }) },
    });
    expect(queryByText('views')).toBeFalsy();
  });

  it('formats large counts with a k suffix', () => {
    const { getByText } = render(WorkCard, {
      props: {
        work: sampleWork({
          metrics: { views: 12345, kudos: 6789, reactions: 12, complete_reads: 5, bookmarks: 3, collection_adds: 2, reviews: 1 },
        }),
        variant: 'full',
      },
    });
    expect(getByText('12k views')).toBeTruthy();
    expect(getByText('6.8k kudos')).toBeTruthy();
  });
});
