import '@testing-library/jest-dom/vitest';

// jsdom does not implement matchMedia, which the theme resolver depends on.
// Providing a minimal stub keeps component tests honest without pulling in a
// polyfill package.
if (!window.matchMedia) {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}

/**
 * Node 22+ ships a `localStorage` global that is inert unless the process was
 * started with `--localstorage-file`, and it can shadow the one jsdom provides.
 * The application reads `window.localStorage` for the theme preference, so the
 * test environment must guarantee that object exists and behaves.
 */
function createMemoryStorage(): Storage {
  const map = new Map<string, string>();
  return {
    get length() {
      return map.size;
    },
    clear: () => map.clear(),
    getItem: (key: string) => (map.has(key) ? (map.get(key) as string) : null),
    key: (index: number) => [...map.keys()][index] ?? null,
    removeItem: (key: string) => void map.delete(key),
    setItem: (key: string, value: string) => void map.set(key, String(value)),
  } as Storage;
}

function hasWorkingStorage(target: unknown): boolean {
  try {
    const candidate = (target as { localStorage?: Storage } | undefined)?.localStorage;
    if (!candidate) return false;
    candidate.setItem('__probe__', '1');
    candidate.removeItem('__probe__');
    return true;
  } catch {
    return false;
  }
}

const storage = createMemoryStorage();

if (!hasWorkingStorage(window)) {
  Object.defineProperty(window, 'localStorage', {
    configurable: true,
    writable: true,
    value: storage,
  });
}

if (!hasWorkingStorage(globalThis)) {
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    writable: true,
    value: storage,
  });
}
