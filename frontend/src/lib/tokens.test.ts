/**
 * Every custom property a component reads must be defined somewhere.
 *
 * An undefined custom property does not fail loudly: `var(--surface-2, #f6f6f6)`
 * resolves to the *fallback*, so a component written against a name the design
 * system never defines renders its light-theme literal in every theme. That is
 * how `/docs` came to show an invisible table of contents — `#eeefe7` text on a
 * `#f6f6f6` panel, a contrast ratio of 1.07, in the dark default theme — and how
 * sixty-three names came to be in use across thirty files without anything
 * failing.
 *
 * The check reads the sources rather than the rendered page because the fault is
 * a missing declaration, which no rendered assertion can distinguish from a
 * deliberately inherited value.
 *
 * Two allowlists, both deliberate:
 *
 * - `RUNTIME` names are set inline by a component before it is read
 *   (`style="--clamp-lines: 3"`, `el.style.setProperty('--reader-font-scale')`).
 *   They have no static definition and should not have one.
 * - `FALLBACK_OK` names are read with a fallback that is correct in every theme
 *   (a length, a font stack) rather than a colour literal.
 *
 * When this test fails, prefer adding the name to the alias layer in
 * `styles/tokens.css` over changing the component: the alias fixes every use at
 * once, and the component's fallback then never applies.
 */
import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

/** Custom properties set at runtime, never declared in CSS. */
const RUNTIME = new Set([
  '--clamp-lines',
  '--line-index',
  '--reader-font-scale',
  '--reader-line-height',
  '--reader-measure',
  '--reader-progress',
]);

/** Read with a non-colour fallback that is safe in every theme/or not a token at all. */
const FALLBACK_OK = new Set<string>([
  // handled by the same rule as everything else; kept explicit so a future
  // non-colour fallback has an obvious home.
]);

function walk(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      walk(path, out);
    } else if (entry.endsWith('.svelte') || entry.endsWith('.css')) {
      out.push(path);
    }
  }
  return out;
}

// `import.meta.url` is synthetic under vitest's transform (it resolved to /src),
// so the project root comes from the runner's cwd, which is `frontend/`.
const ROOT = process.cwd();

/** Every `var(--name` read, with the files that read it. */
function reads(files: string[]): Map<string, Set<string>> {
  const used = new Map<string, Set<string>>();
  for (const file of files) {
    const source = readFileSync(file, 'utf8');
    for (const match of source.matchAll(/var\(\s*(--[A-Za-z0-9-]+)/g)) {
      const name = match[1];
      if (!used.has(name)) used.set(name, new Set());
      used.get(name)!.add(relative(ROOT, file));
    }
  }
  return used;
}

/**
 * Every custom property DECLARED — the name leads its line, as declarations do.
 *
 * Deliberately not "the name appears before a colon anywhere in the file": that
 * looser pattern also matches `var(--surface, #fff)` inside a value, which makes
 * the set of defined names equal to the set of used ones and the check vacuous.
 * The first version of this file did exactly that.
 */
function definitions(files: string[]): Set<string> {
  const defined = new Set<string>();
  for (const file of files) {
    for (const line of readFileSync(file, 'utf8').split('\n')) {
      const match = line.match(/^\s*(--[A-Za-z0-9-]+)\s*:/);
      if (match) defined.add(match[1]);
    }
  }
  return defined;
}

describe('design tokens', () => {
  const files = walk(join(ROOT, 'src'));
  const used = reads(files);
  const defined = definitions(files);

  it('finds the sources it is meant to check', () => {
    // A glob that matches nothing reports success, and this file's whole value
    // is that it looked at every component.
    expect(files.length).toBeGreaterThan(50);
    expect(used.size).toBeGreaterThan(50);
    expect(defined.size).toBeGreaterThan(50);
  });

  it('defines every custom property a component reads', () => {
    console.log('DEBUG defined', defined.size, 'used', used.size, 'sample', [...defined].slice(0,6));
    const missing: string[] = [];
    for (const [name, where] of [...used].sort()) {
      if (defined.has(name) || RUNTIME.has(name) || FALLBACK_OK.has(name)) continue;
      missing.push(`${name} (${[...where].sort().join(', ')})`);
    }
    console.log('DEBUG missing', missing.length, missing.slice(0,5));
    expect(
      missing,
      `undefined custom properties fall back to their literal, which is usually a light-theme ` +
        `colour and therefore invisible in the dark themes. Add an alias in styles/tokens.css:\n  ` +
        missing.join('\n  '),
    ).toEqual([]);
  });

  it('declares each alias in the token file rather than in a component', () => {
    const tokens = readFileSync(join(ROOT, 'src/styles/tokens.css'), 'utf8');
    for (const name of ['--surface', '--text-muted', '--border', '--accent', '--danger', '--radius']) {
      expect(tokens, `${name} must be declared in styles/tokens.css`).toContain(`${name}:`);
    }
  });
});
