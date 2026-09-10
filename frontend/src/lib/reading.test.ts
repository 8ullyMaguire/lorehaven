import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  DEFAULT_READER_THEME,
  READER_THEMES,
  applyTypography,
  readCachedPosition,
  writeCachedPosition,
  clearCachedPosition,
  readTypographyPrefs,
  writeTypographyPrefs,
  resolveReaderTheme,
  POSITION_STORAGE_KEY,
  TYPOGRAPHY_STORAGE_KEY,
} from './reading';

// A `getItem` spy installed by one test would otherwise stay installed for
// every test after it in this file, so each test starts from a clean slate.
afterEach(() => {
  vi.restoreAllMocks();
  // The typography tests write to the real document element; leaving the
  // attributes behind would leak into the tests that follow.
  const root = document.documentElement;
  delete root.dataset.reader;
  delete root.dataset.distractionFree;
  root.style.removeProperty('--reader-font-scale');
  root.style.removeProperty('--reader-line-height');
  root.style.removeProperty('--reader-measure');
});

function createPrefs(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    font_scale: 1.0,
    line_height: 1.6,
    measure: 66,
    reader_theme: 'reading-room',
    distraction_free: false,
    version: 0,
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
    const prefs = createPrefs({ font_scale: 1.25, measure: 60, reader_theme: 'after-hours' });
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

describe('applying typography', () => {
  it("writes the reader's preset to data-reader, the attribute tokens.css selects on", () => {
    applyTypography(createPrefs({ reader_theme: 'dark' }) as never, document.documentElement);
    // `tokens.css` defines `[data-reader='dark']`. Writing
    // `data-reader-theme` here — which is what this used to do — left the
    // reader's theme control changing no pixel at all.
    expect(document.documentElement.dataset.reader).toBe('dark');
    expect(document.documentElement.dataset.readerTheme).toBeUndefined();
  });

  it('resolves a value outside the reader presets to the default', () => {
    // The server accepts any string for `reader_theme`, including the site
    // theme names an older build offered.
    expect(resolveReaderTheme('reading-room')).toBe(DEFAULT_READER_THEME);
    expect(resolveReaderTheme(null)).toBe(DEFAULT_READER_THEME);
    expect(resolveReaderTheme(42)).toBe(DEFAULT_READER_THEME);
    for (const theme of READER_THEMES) {
      expect(resolveReaderTheme(theme)).toBe(theme);
    }
  });

  it('sets every custom property the reader consumes, with units', () => {
    applyTypography(
      createPrefs({ font_scale: 1.3, line_height: 2, measure: 60 }) as never,
      document.documentElement,
    );
    const style = document.documentElement.style;
    expect(style.getPropertyValue('--reader-font-scale')).toBe('1.3');
    expect(style.getPropertyValue('--reader-line-height')).toBe('2');
    expect(style.getPropertyValue('--reader-measure')).toBe('60ch');
  });

  it('marks distraction-free on the document element, which is what hides the chrome', () => {
    applyTypography(createPrefs({ distraction_free: true }) as never, document.documentElement);
    expect(document.documentElement.dataset.distractionFree).toBe('true');
    applyTypography(createPrefs({ distraction_free: false }) as never, document.documentElement);
    expect(document.documentElement.dataset.distractionFree).toBe('false');
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