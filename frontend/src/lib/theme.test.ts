import { describe, expect, it } from 'vitest';
import {
  STORAGE_KEY,
  applyTheme,
  isTheme,
  readPreference,
  resolveTheme,
  writePreference,
} from './theme';

describe('theme resolution', () => {
  it('resolves the system preference to a concrete preset', () => {
    expect(resolveTheme('system', true)).toBe('after-hours');
    expect(resolveTheme('system', false)).toBe('reading-room');
  });

  it('honours an explicit choice over the system preference', () => {
    expect(resolveTheme('clear-day', true)).toBe('clear-day');
    expect(resolveTheme('reading-room', true)).toBe('reading-room');
  });

  it('recognises only the shipped presets', () => {
    expect(isTheme('reading-room')).toBe(true);
    expect(isTheme('after-hours')).toBe(true);
    expect(isTheme('clear-day')).toBe(true);
    expect(isTheme('midnight-velvet')).toBe(false);
    expect(isTheme(null)).toBe(false);
  });

  it('reads and writes the stored preference', () => {
    const store = new Map<string, string>();
    const storage = {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => void store.set(key, value),
    };

    expect(readPreference(storage)).toBe('system');

    writePreference(storage, 'after-hours');
    expect(store.get(STORAGE_KEY)).toBe('after-hours');
    expect(readPreference(storage)).toBe('after-hours');

    writePreference(storage, 'system');
    expect(readPreference(storage)).toBe('system');
  });

  it('falls back to the system when storage holds nonsense or throws', () => {
    const hostile = {
      getItem: () => {
        throw new Error('private browsing');
      },
      setItem: () => {
        throw new Error('private browsing');
      },
    };

    expect(readPreference(hostile)).toBe('system');
    // Writing must not throw either: failing to persist is not failing to apply.
    expect(() => writePreference(hostile, 'clear-day')).not.toThrow();

    const garbage = { getItem: () => 'not-a-theme' };
    expect(readPreference(garbage)).toBe('system');
    expect(readPreference(null)).toBe('system');
  });

  it('applies the theme as a data attribute the tokens respond to', () => {
    const root = document.createElement('div');
    applyTheme('clear-day', root);
    expect(root.dataset.theme).toBe('clear-day');
    applyTheme('after-hours', root);
    expect(root.dataset.theme).toBe('after-hours');
  });
});
