/**
 * Reading position, typography, and progress state.
 *
 * Spec §9. This module owns the local position cache and the flush
 * on scroll-end / visibilitychange / unload. It mirrors the shape of
 * `autosave.ts` but for reading position rather than document edits.
 *
 * One rule the whole module depends on: position is saved to
 * `localStorage` first so a lost request still leaves a position.
 * The server is the source of truth; the local cache is a best-effort
 * acceleration layer.
 */

import type { ServerPosition } from './api';

/** The localStorage key for the reading position cache. */
export const POSITION_STORAGE_KEY = 'lorehaven.reading-position';

/** The localStorage key for typography preferences. */
export const TYPOGRAPHY_STORAGE_KEY = 'lorehaven.typography';

/** A reading position stored locally. */
export interface CachedPosition {
  workId: string;
  chapterId: string;
  position: ServerPosition;
  savedAt: number;
}

/** The reader's own surface presets, matching `[data-reader='…']` in tokens.css. */
export const READER_THEMES = ['paper', 'white', 'sepia', 'dark'] as const;

/** One of the reader's surface presets. */
export type ReaderTheme = (typeof READER_THEMES)[number];

/** The preset a reader gets before choosing one (the server's default too). */
export const DEFAULT_READER_THEME: ReaderTheme = 'sepia';

/** Human labels for the presets. */
export const READER_THEME_LABELS: Record<ReaderTheme, string> = {
  paper: 'Paper',
  white: 'White',
  sepia: 'Sepia',
  dark: 'Dark',
};

/**
 * The typography a reader gets before anything is stored or configured.
 *
 * The server holds the same defaults; this copy is what makes the reading
 * surface look right on the first paint, before any request has answered.
 */
export const DEFAULT_TYPOGRAPHY: TypographyPrefs = {
  font_scale: 1,
  line_height: 1.6,
  measure: 66,
  reader_theme: DEFAULT_READER_THEME,
  distraction_free: false,
  version: 0,
};

/**
 * The stored reader theme, or the default.
 *
 * The server accepts any string for `reader_theme`, so the value can be
 * something this build does not know — a site theme name from an older
 * release, for instance. It is resolved here rather than written to the
 * document, because `data-reader` selects from a closed set in the stylesheet
 * and an unrecognised value would paint nothing at all.
 */
export function resolveReaderTheme(value: unknown): ReaderTheme {
  return typeof value === 'string' && (READER_THEMES as readonly string[]).includes(value)
    ? (value as ReaderTheme)
    : DEFAULT_READER_THEME;
}

/**
 * Typography preferences stored locally.
 *
 * The field names are the server's (`font_scale`, `measure`, `reader_theme`),
 * not the reader's words for them, because the same object is written back to
 * `PATCH /settings/typography` and has to name the columns the server expects.
 */
export interface TypographyPrefs {
  font_scale: number;
  line_height: number;
  /** Line length in characters. */
  measure: number;
  reader_theme: string;
  distraction_free: boolean;
  /** The version the server last reported, for the next stale-write check. */
  version: number;
}

/**
 * Apply a reader's typography to the document.
 *
 * The attribute is `data-reader`, spelled exactly as `tokens.css` selects it
 * (`[data-reader='sepia']`). It is not `data-reader-theme`: that name was
 * written here and read by nothing, so the reader's theme control changed no
 * pixel. `frontend/static/prepaint.js` mirrors this function for the first
 * paint, and a test asserts the two agree.
 */
export function applyTypography(prefs: TypographyPrefs, root: HTMLElement): void {
  const style = root.style;
  style.setProperty('--reader-font-scale', String(prefs.font_scale));
  style.setProperty('--reader-line-height', String(prefs.line_height));
  style.setProperty('--reader-measure', `${prefs.measure}ch`);
  root.dataset.reader = resolveReaderTheme(prefs.reader_theme);
  root.dataset.distractionFree = prefs.distraction_free ? 'true' : 'false';
}

/** A pending position flush. */
interface PendingFlush {
  workId: string;
  chapterId: string;
  revision: string | null;
  anchor: string | null;
  fraction: number;
  device: string | null;
}

const DEBOUNCE_MS = 1000;

/** Read the cached position for a work from localStorage. */
export function readCachedPosition(workId: string): CachedPosition | null {
  try {
    const raw = localStorage.getItem(POSITION_STORAGE_KEY);
    if (!raw) return null;
    const cached: CachedPosition[] = JSON.parse(raw);
    const entry = cached.find((p) => p.workId === workId);
    return entry ?? null;
  } catch {
    return null;
  }
}

/** Write the cached position for a work to localStorage. */
export function writeCachedPosition(pos: CachedPosition): void {
  try {
    const raw = localStorage.getItem(POSITION_STORAGE_KEY);
    const existing: CachedPosition[] = raw ? JSON.parse(raw) : [];
    const filtered = existing.filter((p) => p.workId !== pos.workId);
    filtered.push(pos);
    localStorage.setItem(POSITION_STORAGE_KEY, JSON.stringify(filtered));
  } catch {
    // Quota exceeded or private mode. The in-memory state still holds.
  }
}

/** Clear the cached position for a work. */
export function clearCachedPosition(workId: string): void {
  try {
    const raw = localStorage.getItem(POSITION_STORAGE_KEY);
    if (!raw) return;
    const existing: CachedPosition[] = JSON.parse(raw);
    const filtered = existing.filter((p) => p.workId !== workId);
    if (filtered.length === 0) {
      localStorage.removeItem(POSITION_STORAGE_KEY);
    } else {
      localStorage.setItem(POSITION_STORAGE_KEY, JSON.stringify(filtered));
    }
  } catch {
    // nothing to do
  }
}

/** Read stored typography preferences. */
export function readTypographyPrefs(): TypographyPrefs | null {
  try {
    const raw = localStorage.getItem(TYPOGRAPHY_STORAGE_KEY);
    if (!raw) return null;
    return JSON.parse(raw) as TypographyPrefs;
  } catch {
    return null;
  }
}

/** Write typography preferences to localStorage. */
export function writeTypographyPrefs(prefs: TypographyPrefs): void {
  try {
    localStorage.setItem(TYPOGRAPHY_STORAGE_KEY, JSON.stringify(prefs));
  } catch {
    // nothing to do
  }
}

/** Flush a position to the server, debounced. */
export class PositionFlusher {
  private timer: ReturnType<typeof setTimeout> | null = null;
  private pending: PendingFlush | null = null;
  private readonly flush: (pos: PendingFlush) => Promise<void>;

  constructor(flush: (pos: PendingFlush) => Promise<void>) {
    this.flush = flush;
  }

  /** Schedule a position flush. The previous pending flush is replaced. */
  schedule(pos: PendingFlush): void {
    this.pending = pos;
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = setTimeout(() => this.flushNow(), DEBOUNCE_MS);
  }

  /** Flush immediately, canceling any scheduled flush. */
  async flushNow(): Promise<void> {
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    if (this.pending === null) return;
    const pos = this.pending;
    this.pending = null;
    await this.flush(pos);
  }

  /** Whether a flush is pending. */
  get hasPending(): boolean {
    return this.pending !== null;
  }
}

/** Save a position to the server, with fallback to localStorage. */
export async function savePosition(
  position: PendingFlush,
  fetchFn: (pos: PendingFlush) => Promise<void>,
): Promise<void> {
  // Write to localStorage first so a lost request still leaves a position.
  const cached: CachedPosition = {
    workId: position.workId,
    chapterId: position.chapterId,
    position: {
      revision: position.revision,
      anchor: position.anchor,
      position_permille: position.fraction,
      device: position.device,
    },
    savedAt: Date.now(),
  };
  writeCachedPosition(cached);

  // Try the server.
  try {
    await fetchFn(position);
  } catch {
    // The local cache already has the position. Retry on next load.
  }
}
