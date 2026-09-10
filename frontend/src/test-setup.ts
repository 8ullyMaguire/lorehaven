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
