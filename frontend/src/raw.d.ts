/**
 * Vite's `?raw` import (used by `lib/docs.ts` to bundle the markdown docs)
 * is typed by `vite/client`. The project's tsconfig does not include that
 * ambient types entry (it lists only vitest and jest-dom globals), so the
 * one declaration the docs registry needs lives here instead of adding a
 * dependency the rest of the code never uses.
 */
declare module '*?raw' {
  const content: string;
  export default content;
}
