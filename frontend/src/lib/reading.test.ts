import { describe, expect, it, vi } from 'vitest';
import {
  readCachedPosition,
  writeCachedPosition,
  clearCachedPosition,
  readTypographyPrefs,
  writeTypographyPrefs,
  POSITION_STORAGE_KEY,
  TYPOGRAPHY_STORAGE_KEY,
} from './reading';

function createPrefs(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    font_size: 100,
    line_width: 70,
    theme: 'paper',
    expected_version: 1,
    ...overrides,
  };
}

function cachedPosition(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    workId: 'work-1',
    chapterId: 'chapter-1',
    position: {
      revision: null,
      anchor: null,
      position_permille: 500,
      device: 'device-1',
    },
    savedAt: Date.now(),
    ...overrides,
  };
}

describe('reading position cache', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('returns null when nothing is cached', () => {
    expect(readCachedPosition('work-1')).toBeNull();
  });

  it('reads and writes a cached position', () => {
    writeCachedPosition(cachedPosition() as never);
    const read = readCachedPosition('work-1');
    expect(read).not.toBeNull();
    expect(read!.workId).toBe('work-1');
    expect(read!.chapterId).toBe('chapter-1');
    expect(read!.position.position_permille).toBe(500);
  });

  it('replaces a position for the same work', () => {
    writeCachedPosition(cachedPosition() as never);
    writeCachedPosition(cachedPosition({ position: { revision: null, anchor: null, position_permille: 750, device: 'device-1' } }) as never);
    const read = readCachedPosition('work-1');
    expect(read!.position.position_permille).toBe(750);
  });

  it('clears a cached position', () => {
    writeCachedPosition(cachedPosition() as never);
    clearCachedPosition('work-1');
    expect(readCachedPosition('work-1')).toBeNull();
  });

  it('returns null when localStorage throws', () => {
    vi.spyOn(localStorage, 'getItem').mockImplementation(() => {
      throw new Error('QuotaExceeded');
    });
    expect(readCachedPosition('work-1')).toBeNull();
  });
});

describe('typography preferences', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('returns null when nothing is stored', () => {
    expect(readTypographyPrefs()).toBeNull();
  });

  it('reads and writes typography preferences', () => {
    const prefs = createPrefs({ font_size: 125, line_width: 80, theme: 'sepia' });
    writeTypographyPrefs(prefs as never);
    expect(readTypographyPrefs()).toEqual(prefs);
  });

  it('returns null when localStorage throws', () => {
    vi.spyOn(localStorage, 'getItem').mockImplementation(() => {
      throw new Error('QuotaExceeded');
    });
    expect(readTypographyPrefs()).toBeNull();
  });
});

describe('constants', () => {
  it('exports the correct position storage key', () => {
    expect(POSITION_STORAGE_KEY).toBe('lorehaven.reading-position');
  });

  it('exports the correct typography storage key', () => {
    expect(TYPOGRAPHY_STORAGE_KEY).toBe('lorehaven.typography');
  });
});