import { test, expect } from '@playwright/test';

/**
 * The header must fit its destinations without scrolling, and stay one row tall.
 *
 * ## Why this exists
 *
 * The desktop nav used to be eighteen links in an `overflow-x: auto` row with
 * `scrollbar-width: none`. It scrolled, silently: anything past the fold did not
 * exist as far as a reader could tell. The redesign replaced it with four links
 * and five menus, and this test is what keeps that honest.
 *
 * Both halves matter and they fail differently:
 *
 * - `overBy > 0` means the row scrolls. With the scrollbar now visible that is no
 *   longer silent, but it still means a destination is hidden behind a gesture
 *   nobody knows to make.
 * - a row height above the token means the header grew, and a growing header
 *   breaks `scroll-padding-top`, which is exactly the bug that made the analytics
 *   E2E run unclickable (App.svelte documents it at length).
 *
 * ## Why the widths are fixed rather than "whatever the browser gives"
 *
 * The bar is a width-capped container, so the row's fit does not improve with a
 * wider window: at 1440px the content box is 1088px whatever the viewport. A
 * viewport-driven test would pass at 1920px and hide a row that still overflows
 * in a real window, so the container width is set directly.
 */

const VIEWPORT = { width: 1440, height: 900 };

test.describe('header navigation', () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize(VIEWPORT);
    await page.goto('/');
    await page.waitForLoadState('networkidle');
  });

  test('every destination is reachable without scrolling the row', async ({ page }) => {
    const nav = page.locator('header nav.desktop');
    await expect(nav).toBeVisible();

    const fit = await nav.evaluate((el) => ({
      overBy: el.scrollWidth - el.clientWidth,
      scrollWidth: el.scrollWidth,
      clientWidth: el.clientWidth,
      items: el.children.length,
    }));

    // An empty row trivially does not scroll. Say so, or the test passes for the
    // wrong reason on a shell that failed to render.
    expect(fit.items, 'the nav row rendered no items').toBeGreaterThan(0);

    // Named in the failure message because "expected 0 to be 134" is useless.
    expect(
      fit.overBy,
      `the header row needs ${fit.scrollWidth}px but has ${fit.clientWidth}px across ${fit.items} items`,
    ).toBeLessThanOrEqual(0);
  });

  test('the row stays on one line, so the header keeps a fixed height', async ({ page }) => {
    const tops = await page.locator('header nav.desktop > *').evaluateAll((els) =>
      els.map((el) => Math.round(el.getBoundingClientRect().top)),
    );

    expect(tops.length, 'the nav row rendered no items').toBeGreaterThan(0);
    // Every item shares one `top`. Two distinct values means it wrapped.
    expect(
      new Set(tops).size,
      `items sit at tops ${JSON.stringify(tops)}, so the row wrapped`,
    ).toBe(1);
  });

  test('no destination is named twice in the row', async ({ page }) => {
    // The regression that actually overflowed it: "Write" and "Library" were both
    // a primary link and a menu trigger, so the row read Discover, Library,
    // Search, Write, Read, Write, Library, Community, More.
    //
    // The count is asserted BEFORE the duplicates are, deliberately. A test that
    // only checks for duplicates passes on an EMPTY list, which is what happened
    // on the first version of this test: `header nav.desktop` matched nothing,
    // `[...el.children]` was empty, and "no duplicates" was vacuously true while
    // the row plainly read "Write ... Write". An absence test needs its subject
    // counted.
    const labels = await page.locator('header nav.desktop > *').evaluateAll((els) =>
      els.map((el) => el.textContent?.trim() ?? ''),
    );

    expect(labels.length, 'the nav row rendered no items, so the rest is vacuous')
      .toBeGreaterThan(0);
    expect(labels).toHaveLength(9);

    const duplicates = labels.filter((label, i) => labels.indexOf(label) !== i);
    expect(duplicates, `repeated in the header: ${duplicates.join(', ')}`).toEqual([]);
  });

  test('opening a menu does not change the header height', async ({ page }) => {
    const heightOf = () =>
      page.locator('header').evaluate((el) => el.getBoundingClientRect().height);

    const before = await heightOf();
    await page.getByTestId('menu-trigger').first().click();
    await expect(page.getByTestId('menu-panel')).toBeVisible();
    const after = await heightOf();

    // The panel is absolutely positioned precisely so this holds. If it ever
    // stops, `--header-height` becomes a lie and anchored scrolling breaks.
    expect(
      Math.abs(after - before),
      `header was ${before}px closed and ${after}px with a menu open`,
    ).toBeLessThan(1);
  });

  test('the row is reachable by keyboard, and Escape returns focus', async ({ page }) => {
    const trigger = page.getByTestId('menu-trigger').first();

    await trigger.focus();
    await page.keyboard.press('ArrowDown');
    await expect(page.getByTestId('menu-panel')).toBeVisible();

    // Focus lands INSIDE the menu, so the keyboard path does not have to Tab
    // through the page to reach the panel it just opened.
    const focusedInPanel = await page.evaluate(
      () => !!document.querySelector('[data-testid="menu-panel"]')?.contains(document.activeElement),
    );
    expect(focusedInPanel, 'opening a menu should move focus into it').toBe(true);

    await page.keyboard.press('Escape');
    await expect(page.getByTestId('menu-panel')).toBeHidden();

    const focusReturned = await trigger.evaluate((el) => el === document.activeElement);
    expect(focusReturned, 'Escape must return focus to the trigger').toBe(true);
  });

  test('a sub-path still marks its parent destination as current', async ({ page }) => {
    // Exact-match alone left /library/history highlighting nothing, because no
    // nav item IS /library/history.
    await page.goto('/library/history');
    await page.waitForLoadState('networkidle');

    const current = await page.locator('header nav.desktop > a[aria-current="page"]').allTextContents();
    expect(current.map((t) => t.trim())).toContain('Library');
  });
});

test.describe('landing page', () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize(VIEWPORT);
    await page.goto('/');
    await page.waitForLoadState('networkidle');
  });

  /**
   * The page was a status page: it opened with a build hash and a table of
   * database connection strings. This asserts the ordering that fixes it, in
   * document order, because position IS the claim.
   */
  test('offers what the place is FOR before it offers server health', async ({ page }) => {
    const order = await page.locator('main section > h1, main section > h2').evaluateAll((els) =>
      els.map((el) => el.textContent?.trim() ?? ''),
    );

    const waysIn = order.indexOf('Four ways in');
    const canDo = order.indexOf('What you can do here');
    const instance = order.indexOf('This instance');

    expect(waysIn, `section order was ${JSON.stringify(order)}`).toBeGreaterThanOrEqual(0);
    expect(canDo).toBeGreaterThan(waysIn);
    expect(instance).toBeGreaterThan(canDo);
  });

  test('the four ways in all link somewhere real', async ({ page }) => {
    const hrefs = await page
      .locator('section:has(h2:text("Four ways in")) a')
      .evaluateAll((els) => els.map((el) => el.getAttribute('href')));

    expect(hrefs.sort()).toEqual(['/discover', '/library', '/search', '/write']);
  });

  test('does not greet a visitor with a build hash', async ({ page }) => {
    const text = await page.locator('main').innerText();
    expect(text).not.toMatch(/sqlite reachable|postgres reachable|migration\(s\) applied/);
  });

  test('never overflows horizontally at the documented 320px floor', async ({ page }) => {
    await page.setViewportSize({ width: 320, height: 800 });
    await page.goto('/');
    await page.waitForLoadState('networkidle');

    const over = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(over, `the page scrolls sideways by ${over}px at 320px wide`).toBeLessThanOrEqual(0);
  });
});