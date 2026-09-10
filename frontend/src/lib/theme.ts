/**
 * Theme resolution.
 *
 * docs/design/THEME.md ships three polished presets and a system preference.
 * The same logic runs in `index.html` before first paint (to avoid a flash) and
 * here (to react to changes), so both are kept deliberately small and parallel.
 */

export const THEMES = ['reading-room', 'after-hours', 'clear-day'] as const;
export type Theme = (typeof THEMES)[number];

/** What the user chose, which may be "follow the system". */
export type ThemePreference = Theme | 'system';

export const STORAGE_KEY = 'lorehaven.theme';

export const THEME_LABELS: Record<ThemePreference, string> = {
  system: 'Match system',
  'reading-room': 'Reading Room',
  'after-hours': 'After Hours',
  'clear-day': 'Clear Day',
};

const DARK: Theme = 'after-hours';
const LIGHT: Theme = 'reading-room';

/** Whether the value is one of the shipped presets. */
export function isTheme(value: unknown): value is Theme {
  return typeof value === 'string' && (THEMES as readonly string[]).includes(value);
}

/** Resolve a preference to a concrete preset. */
export function resolveTheme(
  preference: ThemePreference,
  prefersDark: boolean,
): Theme {
  if (preference === 'system') return prefersDark ? DARK : LIGHT;
  return preference;
}

/** Read the stored preference, defaulting to following the system. */
export function readPreference(storage: Pick<Storage, 'getItem'> | null): ThemePreference {
  if (!storage) return 'system';
  try {
    const raw = storage.getItem(STORAGE_KEY);
    if (raw === 'system') return 'system';
    if (isTheme(raw)) return raw;
  } catch {
    // Storage can throw in private browsing modes; following the system is a
    // perfectly good answer when the preference is unreadable.
  }
  return 'system';
}

/** Persist a preference. Failure to store is not failure to apply. */
export function writePreference(
  storage: Pick<Storage, 'setItem'> | null,
  preference: ThemePreference,
): void {
  if (!storage) return;
  try {
    storage.setItem(STORAGE_KEY, preference);
  } catch {
    // Ignored deliberately.
  }
}

/** Apply a resolved theme to the document. */
export function applyTheme(theme: Theme, root: HTMLElement): void {
  root.dataset.theme = theme;
}
