/**
 * The two Node APIs the tests use.
 *
 * The suite runs under Node through vitest, but this project deliberately does
 * not depend on `@types/node` — nothing in the shipped frontend uses a Node
 * API. These declarations exist so a test can read a file that ships beside the
 * bundle (`index.html`, `static/prepaint.js`) and check it, rather than adding
 * a dependency for two functions.
 */
declare module 'node:fs' {
  export function readFileSync(path: string, encoding: 'utf8'): string;
}

declare module 'node:url' {
  export function fileURLToPath(url: string | URL): string;
}

/** vitest runs the suite from the frontend root, so this is how a test finds
 *  a file that ships beside the bundle. */
declare const process: { cwd(): string };
