/*
 * Appearance applied before the first paint.
 *
 * This is a file rather than an inline <script> because the server sends
 * `script-src 'self'`: an inline block is refused by the browser and fails
 * *silently*, which is how the reader's saved typography stopped being applied
 * on load while the settings panel kept claiming it was saved. Anything that
 * belongs in front of the first paint has to live here.
 *
 * It mirrors `applyTypography` and `resolveTheme` in src/lib. The duplication
 * is deliberate — this runs before the bundle exists — and
 * `src/lib/prepaint.test.ts` fails if the two drift apart.
 *
 * The stored typography object is the one the server last reported, so its
 * field names are the server's.
 */
(function () {
  'use strict';

  // --- the site theme -------------------------------------------------------
  try {
    var stored = localStorage.getItem('lorehaven.theme');
    var prefersDark =
      window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches;
    var resolved =
      !stored || stored === 'system'
        ? prefersDark
          ? 'after-hours'
          : 'reading-room'
        : stored;
    document.documentElement.dataset.theme = resolved;
  } catch (error) {
    document.documentElement.dataset.theme = 'reading-room';
  }

  // --- the reading surface --------------------------------------------------
  var READER_THEMES = ['paper', 'white', 'sepia', 'dark'];
  var DEFAULT_READER_THEME = 'sepia';

  try {
    var raw = localStorage.getItem('lorehaven.typography');
    if (!raw) return;
    var prefs = JSON.parse(raw);
    var style = document.documentElement.style;
    if (typeof prefs.font_scale === 'number') {
      style.setProperty('--reader-font-scale', String(prefs.font_scale));
    }
    if (typeof prefs.line_height === 'number') {
      style.setProperty('--reader-line-height', String(prefs.line_height));
    }
    if (typeof prefs.measure === 'number') {
      style.setProperty('--reader-measure', prefs.measure + 'ch');
    }
    // `data-reader`, which is what tokens.css selects on. An unknown value
    // resolves to the default rather than being written through.
    document.documentElement.dataset.reader =
      READER_THEMES.indexOf(prefs.reader_theme) >= 0
        ? prefs.reader_theme
        : DEFAULT_READER_THEME;
    document.documentElement.dataset.distractionFree = prefs.distraction_free
      ? 'true'
      : 'false';
  } catch (error) {
    // A corrupt preference is not worth blocking the page for.
  }
})();
