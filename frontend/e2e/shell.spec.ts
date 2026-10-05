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

  // -----------------------------------------------------------------------
  // The signed-in row, which is the one nobody spends time in.
  //
  // The test above does `page.goto('/')` with no sign-in, and it was correct
  // about the state it covered: signed out, the row needs 581px and has 581px.
  // Signed in, `.controls` grows to 543px — `Writing as @handle` 154 + `Settings`
  // 57 + `Account` 57 + `Sign out` 83 + the appearance `<select>` 143 — while the
  // bar is a WIDTH-CAPPED container at 1088px. The nav was left 310px of room for
  // 544px of content, so 234px of it — 43% of the row — sat behind the scrollbar.
  // A wider window does not help, which is why this was not a viewport problem.
  //
  // So the fit assertion is repeated signed in. That is the entire change: the
  // defect was invisible to a guard that only ever measured the anonymous row.
  // -----------------------------------------------------------------------
  test.describe('signed in', () => {
    const reader = {
      email: 'shell-signedin@lorehaven.test',
      password: 'shell-signedin-passphrase-1',
      handle: 'ShellSignedIn',
      displayName: 'Shell Signed In',
    };

    /**
     * Get to "signed in, with a pseud speaking, an account of my own".
     *
     * Three wrong versions of this ran before this one, and all three failed as
     * assertion timeouts on a header layout that was already correct — worth recording
     * because the shape of the mistake was identical each time: the fixture assumed a
     * state the instance was not in.
     *
     * 1. **Register in `beforeEach`.** The scratch database is created once per run and
     *    shared by every test in the file (`serve-scratch.sh`), so the first test
     *    registered and the second was refused the email that already existed. It
     *    surfaced as four unrelated-looking failures, because the timeout was on
     *    `toHaveURL`, not on anything to do with the header.
     * 2. **Discriminate on the sign-in FORM instead of its outcome.** The second
     *    version asked "is `#sign-in-password` visible?" to decide whether an account
     *    existed. It is visible on `/sign-in` whether or not the account does, so the
     *    answer was always yes: the first test tried to sign in, was refused, and the
     *    register branch — the only one that could have handled it — never ran. The
     *    run then failed on `writing-as` with a signed-out header, which is what the
     *    `error-context.md` snapshot shows: `Sign in`, `Register` and the appearance
     *    combobox, in a test named "the controls stay inside the bar".
     *
     *    The lesson is the general one: **ask the outcome, not a precondition that
     *    happens to be true in both cases.** A visible form is not evidence of an
     *    account.
     *
     * So: sign in, wait for the identity line, and only if that times out register.
     * The timeout is the branch condition, which makes the slow path the unhappy one
     * rather than the happy one.
     *
     * Each test still gets its own fresh `page`, so this runs five times against one
     * account. That is deliberate: it is the same account every time, which is the only
     * way `Writing as @handle` has the same width in every measurement. Two accounts
     * with different handle lengths would make the one variable the layout depends on
     * the thing the fixture varied.
     */
    test.beforeEach(async ({ page }) => {
      await page.goto('/sign-in');
      await page.fill('#sign-in-email', reader.email);
      await page.fill('#sign-in-password', reader.password);
      await page.click('button[type=submit]');

      // `locator.isVisible({ timeout })` **does not wait** — the `timeout` option is
      // accepted and ignored, and the call is a single instantaneous check. So it
      // read the header microseconds after the sign-in POST, while the session
      // request was still in flight, saw no `writing-as`, and took the register
      // branch on an account that already existed. That is a fourth version of the
      // same bug, and it is why this fixture kept "failing" against a layout that a
      // live browser measured as correct.
      //
      // `locator.count()` is also immediate. The waiting assertion is
      // `expect(locator).toBeVisible()`, which polls for `expect.timeout`. So the
      // branch is decided by waiting for the thing to be true for a moment, and
      // falling back when it never becomes true:
      //
      //     await expect(identity).toBeVisible({ timeout: 8_000 }).catch(() => false)
      //
      // Verified against the live instance before writing this: registering
      // `shell-signedin@lorehaven.test` and landing on `/account` yields
      // `<a class="writing-as" href="/pseud" data-testid="writing-as">Writing as
      // @ShellSignedIn</a>`, and opening the account menu yields all three
      // destinations plus `#theme-select`. The assertions in these tests are true of
      // the running system; what was wrong was the fixture reaching them.
      const identity = page.locator('header .controls a.writing-as');
      const signedIn = await expect(identity)
        .toBeVisible({ timeout: 8_000 })
        .then(() => true)
        .catch(() => false);

      if (!signedIn) {
        // No account yet — this is the first test of the run. Register one.
        //
        // `selectOption` before `click`, because the age band is a `<select>` and not
        // a radio group, and the server rejects an account that has not stated one.
        await page.goto('/register');
        await page.fill('#register-email', reader.email);
        await page.fill('#register-password', reader.password);
        await page.fill('#register-handle', reader.handle);
        await page.fill('#register-display-name', reader.displayName);
        await page.selectOption('#register-age-band', 'adult');
        await page.click('button[type=submit]');
        await expect(page).toHaveURL(/\/account/);
      }

      await page.goto('/');
      await expect(identity).toBeVisible();
    });

    for (const width of [1024, 1440]) {
      test(`every destination fits the row without scrolling, at ${width}px signed in`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 900 });
        await page.goto('/');
        await page.waitForLoadState('networkidle');
        await expect(page.locator('header .controls a.writing-as')).toBeVisible();

        const nav = page.locator('header nav.desktop');
        await expect(nav).toBeVisible();

        const fit = await nav.evaluate((el) => ({
          overBy: el.scrollWidth - el.clientWidth,
          scrollWidth: el.scrollWidth,
          clientWidth: el.clientWidth,
          items: el.children.length,
        }));

        expect(fit.items, 'the nav row rendered no items').toBeGreaterThan(0);
        expect(
          fit.overBy,
          `signed in at ${width}px, the header row needs ${fit.scrollWidth}px but has ` +
            `${fit.clientWidth}px across ${fit.items} items — ${fit.overBy}px is behind ` +
            `the scrollbar`,
        ).toBeLessThanOrEqual(0);
      });
    }

    test('the controls stay inside the bar, not just the nav', async ({ page }) => {
      // The nav fitting is half the claim. If the controls are what pushed it
      // over, they have to be the thing that shrinks — and this is the assertion
      // that says the controls are the ones being paid for, at 1024px.
      await page.setViewportSize({ width: 1024, height: 900 });
      await page.goto('/');
      await page.waitForLoadState('networkidle');

      const controls = await page
        .locator('header .controls')
        .evaluate((el) => Math.round(el.getBoundingClientRect().width));

      // 543px is what it measured at on 2026-10-05, signed in. The budget the
      // plan sets is 300px, which leaves 143 brand + 544 nav + gaps + 300 inside
      // the 1088px content box.
      expect(
        controls,
        `.controls is ${controls}px; the bar is 1088px and the nav needs 544`,
      ).toBeLessThanOrEqual(340);
    });

    test('the row is still one line and the header is still 65px', async ({ page }) => {
      // Both invariants the existing tests defend, asserted signed in as well —
      // the layout change is only allowed to move things sideways.
      await page.setViewportSize(VIEWPORT);
      await page.goto('/');
      await page.waitForLoadState('networkidle');

      const tops = await page
        .locator('header nav.desktop > *')
        .evaluateAll((els) => els.map((el) => Math.round(el.getBoundingClientRect().top)));
      expect(tops.length).toBeGreaterThan(0);
      expect(new Set(tops).size, `items sit at tops ${JSON.stringify(tops)}`).toBe(1);

      const height = await page
        .locator('header')
        .evaluate((el) => Math.round(el.getBoundingClientRect().height));
      // 65px is the measured height with the controls showing. A header that
      // grows breaks `scroll-padding-top`, which App.svelte documents at length.
      expect(Math.abs(height - 65), `header is ${height}px signed in, was 65px`).toBeLessThanOrEqual(1);
    });

    test('every control is still reachable', async ({ page }) => {
      // The fix collapses three links and a select into one account trigger, so
      // this asserts the destinations SURVIVED rather than trusting the layout.
      // `Appearance` in particular: the drawer's `theme-select-mobile` is its
      // only other home, and the drawer is only reachable on a narrow window.
      await page.goto('/account');
      await expect(page).toHaveURL(/\/account/);

      for (const destination of ['/settings', '/account', '/pseud']) {
        const response = await page.goto(destination);
        expect(response?.ok(), `${destination} answered ${response?.status()}`).toBe(true);
      }

      // The theme control itself, which this layout MOVED into the account menu.
      // Asserting it is reachable is the difference between "nothing became
      // unreachable" and "the pages still answer" — the Appearance `<select>` has
      // no route of its own, so if the menu did not open it was simply gone.
      await page.goto('/');
      await page.locator('header .controls [data-testid="menu-trigger"]').click();
      const panel = page.locator('header .controls [data-testid="menu-panel"]');
      await expect(panel).toBeVisible();
      await expect(panel.locator('#theme-select')).toBeVisible();

      // And the menu's own destinations, in the same open panel, so a menu that
      // opened but rendered nothing is not mistaken for a working one.
      for (const [href, name] of [
        ['/account', 'Your account'],
        ['/settings', 'Settings'],
        ['/pseud', 'Your pseuds'],
      ]) {
        await expect(panel.locator(`a[href="${href}"]`), `${name} missing from the menu`).toBeVisible();
      }
      await expect(panel.locator('button:text-is("Sign out")')).toBeVisible();
    });
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