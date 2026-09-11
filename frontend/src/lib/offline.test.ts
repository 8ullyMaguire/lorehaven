import { describe, expect, it } from 'vitest';

import {
  evictionPlan,
  planFreesEnough,
  type OfflineCopy,
  OFFLINE_LIMIT_BYTES,
} from './offline';

/**
 * The offline store's contract:
 *
 *  * what a reader saved most recently is what survives;
 *  * a file that cannot fit even in an empty store is refused, not half-stored;
 *  * "keep this" never silently costs the reader something older without saying
 *    which.
 *
 * Only the decision is tested here, because only the decision can be wrong in a
 * way a reader would notice. IndexedDB is the browser's; this is ours.
 */

function copy(id: string, savedAt: string, sizeBytes: number): OfflineCopy {
  return {
    exportId: id,
    subjectId: `work-${id}`,
    title: `Export ${id}`,
    format: 'epub',
    mediaType: 'application/epub+zip',
    blob: new Blob([]),
    sizeBytes,
    savedAt,
  };
}

const MB = 1024 * 1024;

describe('offline eviction', () => {
  it('evicts the oldest first, and only as many as it must', () => {
    const copies = [
      copy('old', '2026-09-01T00:00:00Z', 3 * MB),
      copy('middle', '2026-09-05T00:00:00Z', 3 * MB),
      copy('recent', '2026-09-09T00:00:00Z', 3 * MB),
    ];

    // Room for the new file after dropping the oldest one only.
    const plan = evictionPlan(copies, 3 * MB, 10 * MB);
    expect(plan).toEqual(['old']);
  });

  it('evicts nothing when the file fits', () => {
    const copies = [copy('a', '2026-09-01T00:00:00Z', 1 * MB)];
    expect(evictionPlan(copies, 1 * MB, 10 * MB)).toEqual([]);
  });

  it('evicts as many as it must, oldest first, and states the order', () => {
    const copies = [
      copy('oldest', '2026-09-01T00:00:00Z', 5 * MB),
      copy('newest', '2026-09-09T00:00:00Z', 5 * MB),
    ];
    // 10 MB held, 6 MB wanted, 10 MB allowed: one eviction is not enough, so
    // both go — and the order is stated, because the interface tells the reader
    // which files this cost them.
    expect(evictionPlan(copies, 6 * MB, 10 * MB)).toEqual(['oldest', 'newest']);
    // A file that fits once the store is empty is kept rather than refused, so
    // "make room" means exactly that.
    expect(planFreesEnough(copies, 6 * MB, 10 * MB)).toBe(true);
  });

  it('replaces a copy without counting it twice', () => {
    // Saving a file again must not evict it to make room for itself: the caller
    // removes the existing entry from the list before planning (see `saveCopy`),
    // and this pins the arithmetic that depends on.
    const copies = [copy('a', '2026-09-01T00:00:00Z', 6 * MB)];
    expect(evictionPlan(copies, 0, 6 * MB)).toEqual([]);
    expect(evictionPlan([], 6 * MB, 6 * MB)).toEqual([]);
  });

  it('says a file does not fit rather than pretending it does', () => {
    const copies = [copy('only', '2026-09-01T00:00:00Z', 4 * MB)];
    // A file larger than the whole allowance: even an empty store cannot hold it.
    expect(evictionPlan(copies, OFFLINE_LIMIT_BYTES + 1)).toEqual(['only']);
    expect(planFreesEnough(copies, OFFLINE_LIMIT_BYTES + 1)).toBe(false);

    // The boundary, from both sides.
    expect(planFreesEnough([], OFFLINE_LIMIT_BYTES)).toBe(true);
    expect(planFreesEnough([], OFFLINE_LIMIT_BYTES + 1)).toBe(false);
  });

  it('counts the whole store, not just what it evicts', () => {
    const copies = [
      copy('a', '2026-09-01T00:00:00Z', 2 * MB),
      copy('b', '2026-09-02T00:00:00Z', 2 * MB),
      copy('c', '2026-09-03T00:00:00Z', 2 * MB),
    ];
    // 6 MB held, 2 MB saved, 7 MB allowed: one eviction is enough, and two would
    // be the store throwing away a file it did not have to.
    expect(planFreesEnough(copies, 2 * MB, 7 * MB)).toBe(true);
    expect(evictionPlan(copies, 2 * MB, 7 * MB)).toEqual(['a']);
  });
});
