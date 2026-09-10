import { readFileSync } from 'node:fs';

import { afterEach, describe, expect, it } from 'vitest';

import { DEFAULT_TYPOGRAPHY, applyTypography } from './reading';

/**
 * The pre-paint script and the shell it lives in.
 *
 * Two defects are pinned here, both found by driving the reader in a browser
 * rather than by reading the code:
 *
 *  * The server's Content-Security-Policy is `script-src 'self'`, so the inline
 *    `<script>` that used to apply the stored typography was refused by the
 *    browser *silently* — a reader's saved type survived a reload in
 *    `localStorage` and was never applied.
 *  * The script has to keep writing what `tokens.css` and `applyTypography`
 *    write, or the reader's theme silently stops matching the stylesheet.
 *
 * A test that only read the file for the string "data-reader" would pass while
 * the two implementations disagreed about everything else, so the script is
 * executed and its effect compared with `applyTypography`'s.
 */

const shell = readFileSync(`${process.cwd()}/index.html`, 'utf8');
const prepaint = readFileSync(`${process.cwd()}/static/prepaint.js`, 'utf8');


/** Run the pre-paint script as the browser would: in the page, before the app. */
function runPrepaint(): void {
  new Function(prepaint)();
}

function resetDocument(): void {
  const root = document.documentElement;
  delete root.dataset.theme;
  delete root.dataset.reader;
  delete root.dataset.distractionFree;
  root.style.removeProperty('--reader-font-scale');
  root.style.removeProperty('--reader-line-height');
  root.style.removeProperty('--reader-measure');
}

describe('the shell and the content-security-policy', () => {
  afterEach(resetDocument);

  it('ships no inline script, because script-src self refuses one', () => {
    const scripts = [...shell.matchAll(/<script\b[^>]*>/g)].map((match) => match[0]);
    expect(scripts.length).toBeGreaterThan(0);
    for (const tag of scripts) {
      expect(tag, `an inline script is refused by the CSP and fails silently: ${tag}`).toMatch(
        /\ssrc=/,
      );
    }
  });

  it('loads the pre-paint script from this origin', () => {
    expect(shell).toContain('src="/prepaint.js"');
  });

  it('still applies the theme before the first paint', () => {
    localStorage.setItem('lorehaven.theme', 'clear-day');
    runPrepaint();
    expect(document.documentElement.dataset.theme).toBe('clear-day');
  });
});

describe('the pre-paint typography', () => {
  afterEach(() => {
    localStorage.clear();
    resetDocument();
  });

  it('does nothing when nothing is stored', () => {
    runPrepaint();
    expect(document.documentElement.dataset.reader).toBeUndefined();
    expect(document.documentElement.style.getPropertyValue('--reader-font-scale')).toBe('');
  });

  it('applies the stored typography the same way the app does', () => {
    const stored = {
      font_scale: 1.3,
      line_height: 2,
      measure: 60,
      reader_theme: 'dark',
      distraction_free: true,
      version: 4,
    };
    localStorage.setItem('lorehaven.typography', JSON.stringify(stored));

    runPrepaint();
    const fromScript = {
      fontScale: document.documentElement.style.getPropertyValue('--reader-font-scale'),
      lineHeight: document.documentElement.style.getPropertyValue('--reader-line-height'),
      measure: document.documentElement.style.getPropertyValue('--reader-measure'),
      reader: document.documentElement.dataset.reader,
      distractionFree: document.documentElement.dataset.distractionFree,
    };

    resetDocument();
    applyTypography(stored, document.documentElement);
    const fromApp = {
      fontScale: document.documentElement.style.getPropertyValue('--reader-font-scale'),
      lineHeight: document.documentElement.style.getPropertyValue('--reader-line-height'),
      measure: document.documentElement.style.getPropertyValue('--reader-measure'),
      reader: document.documentElement.dataset.reader,
      distractionFree: document.documentElement.dataset.distractionFree,
    };

    expect(fromScript).toEqual(fromApp);
  });

  it('resolves an unknown reader theme to the default rather than writing it through', () => {
    localStorage.setItem(
      'lorehaven.typography',
      JSON.stringify({ ...DEFAULT_TYPOGRAPHY, reader_theme: 'reading-room' }),
    );
    runPrepaint();
    expect(document.documentElement.dataset.reader).toBe(DEFAULT_TYPOGRAPHY.reader_theme);
  });

  it('survives a corrupt preference', () => {
    localStorage.setItem('lorehaven.typography', '{not json');
    expect(() => runPrepaint()).not.toThrow();
    expect(document.documentElement.dataset.reader).toBeUndefined();
  });
});
