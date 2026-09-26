import { expect, test, type Page } from '@playwright/test';

/**
 * The analytics page, in a real browser against the real binary.
 *
 * The unit suite covered this page with twenty tests and never noticed that it
 * was not in the router — a component with no route is invisible, and a test
 * that renders it directly cannot tell the difference. This file is the part
 * that can: every assertion goes through the URL a reader would type.
 *
 * Four things it is really about:
 *
 * * **The route exists.** `/analytics` resolves to the dashboard and not to the
 *   not-found view. That is one assertion and it is the one that was missing.
 * * **A signed-out visitor gets a way in, not an error.** The door is
 *   `RequirePseud`, so an anonymous GET is a 401, and rendered as a failure
 *   string that is a message the reader can do nothing about.
 * * **A refused write leaves no trace.** The last test publishes a work and
 *   then tries to mark it finished, and the honest answer is a 404 followed by
 *   a zero -- because a library item is created only by an import, and the work
 *   this test published is not one. See the requirement row for what that costs
 *   the product.
 */

/**
 * Identities are unique per run, because the E2E server's database is *not*
 * reset between runs: an address a previous run registered is rejected with
 * "An account already uses this address", and the test then fails on a
 * precondition while the page under test is fine.
 *
 * `Date.now()` alone is not enough. It is millisecond-resolution and two runs
 * a few seconds apart while the previous server is still up can collide --
 * more to the point, a rerun against the same long-lived scratch database
 * replays the same suffix. `crypto.randomUUID()` cannot collide with anything,
 * and the identifier is only ever used for a throwaway account, so the extra
 * length costs nothing.
 */
const RUN = `r${crypto.randomUUID().slice(0, 8)}`;

/** A fresh suffix per identity, not per file. */
const uniq = () => crypto.randomUUID().slice(0, 8);

interface Who {
  email: string;
  password: string;
  handle: string;
  displayName: string;
}

const AUTHOR: () => Who = () => ({
  // A per-call suffix, not a per-file one: two tests in this file each need an
  // author, and a module-level constant is evaluated once per worker, so the
  // second test would try to create the account the first one already made.
  // The handle needs the same treatment or the pseud collides instead.
  email: `e2e-analytics-author-${uniq()}@lorehaven.test`,
  password: `e2e-analytics-author-pass-${RUN}-x`,
  handle: `E2EAnalyticsAuthor${uniq()}`,
  displayName: 'E2E Analytics Author',
});

const READER: () => Who = () => ({
  email: `e2e-analytics-reader-${uniq()}@lorehaven.test`,
  password: `e2e-analytics-reader-pass-${RUN}-x`,
  handle: `E2EAnalyticsReader${uniq()}`,
  displayName: 'E2E Analytics Reader',
});

async function register(page: Page, who: Who): Promise<void> {
  await page.goto('/register');
  await page.fill('#register-email', who.email);
  await page.fill('#register-password', who.password);
  await page.fill('#register-handle', who.handle);
  await page.fill('#register-display-name', who.displayName);
  await page.selectOption('#register-age-band', 'adult');
  await page.click('button[type=submit]');
  await expect(page).toHaveURL(/\/account/);
}

/** The CSRF token the application issues, read the way the API client does. */
async function csrfToken(page: Page): Promise<string> {
  return page.evaluate(() => {
    const cookie = document.cookie
      .split('; ')
      .find((c) => c.startsWith('lorehaven_csrf='));
    return cookie?.split('=')[1] ?? '';
  });
}

async function signOut(page: Page): Promise<void> {
  await page.goto('/account');
  await page.click('button:text-is("Sign out")');
  await expect(page.locator('a:text-is("Register")')).toBeVisible();
}

/** The four §9.6 fields, read as text so the assertion is about the render. */
async function readingTotals(page: Page): Promise<Record<string, string>> {
  await page.locator('#main button', { hasText: 'own.reading.basic' }).click();
  await expect(page.locator('#main').getByText('Works finished')).toBeVisible();
  const rows = page.locator('#main dl.reading > div');
  const out: Record<string, string> = {};
  for (let i = 0; i < (await rows.count()); i += 1) {
    const label = (await rows.nth(i).locator('dt').textContent())?.trim() ?? '';
    const value = (await rows.nth(i).locator('dd').textContent())?.trim() ?? '';
    out[label] = value;
  }
  return out;
}

test('the analytics page is reachable, and a new reader sees zeroes not an error', async ({
  page,
}) => {
  await register(page, READER());
  await page.goto('/analytics');

  // The heading is the assertion that matters. Before this route existed the
  // page fell through to NotFound, and the component's twenty unit tests were
  // all still green.
  await expect(
    page.locator('#main').getByRole('heading', { name: 'Your analytics' }),
  ).toBeVisible();

  // The registry is the surface, so the reader is told which trust level and
  // instance preset produced this list.
  await expect(page.locator('#main').getByText(/Showing what trust level \d/)).toBeVisible();

  // A brand new account has read nothing, and a zero must render as a zero --
  // these count the viewer's own behaviour, so `Subject::Self_` means no floor.
  const totals = await readingTotals(page);
  expect(totals['Works finished']).toBe('0');
  expect(totals['Chapters read']).toBe('0');
  expect(totals['Time reading']).toBe('0 min');

  // The estimate travels with its method, so no number is ever unexplained.
  await expect(page.getByText(/estimates/i).first()).toBeVisible();
});

test('a signed-out visitor is offered a way in, not an error', async ({ page }) => {
  await page.goto('/analytics');

  // Scoped to #main: the nav bar carries its own "Sign in" link, and a locator
  // that matches both is a locator that is asserting about the wrong thing.
  // A strict-mode violation here is the selector being too broad, not the page
  // being wrong.
  const main = page.locator('#main');
  await expect(main.getByText(/need an account/i)).toBeVisible();
  await expect(main.getByRole('link', { name: 'Sign in' })).toBeVisible();

  // Crucially, not the error state: a 401 is the answer to "is this page for
  // me?", not a failure to report.
  await expect(page.getByRole('alert')).toHaveCount(0);
});

test('a status written against a work id is refused, and the count stays honest', async ({ page }) => {
  test.setTimeout(180_000);

  // The author publishes a work the reader can find.
  await register(page, AUTHOR());
  await page.goto('/write');
  await page.fill('input[id^=field-]', 'The Analytics Odyssey');
  await page.click('button:text-is("Start a draft")');
  await expect(page).toHaveURL(/\/write\//);
  await page.click('button:text-is("Add chapter")');
  const editor = page.locator('.tiptap.ProseMirror');
  await editor.click();
  await page.keyboard.type('One chapter, read by a test so the count moves.');
  const chapterSaved = page.waitForResponse(
    (r) => r.url().includes('/api/v1/chapters/') && r.request().method() !== 'GET',
  );
  await page.click('button:text-is("Save now")');
  await chapterSaved;
  await page.click('button:text-is("Publish")');
  await expect(page.getByText('Republish')).toBeVisible();
  const workId = page.url().split('/').pop()!;
  await signOut(page);

  // The reader opens it, so the library has an item to set a status on.
  await register(page, READER());
  await page.goto(`/works/${workId}`);
  // Named, because a bare `getByRole('heading')` matches the six headings on a
  // work page and a strict-mode violation there is the selector being too
  // broad, not the page being wrong. Asserting the title also proves the
  // reader found the work the author published.
  await expect(
    page.locator('#main').getByRole('heading', { name: 'The Analytics Odyssey' }),
  ).toBeVisible();

  // Reading status is tracked against a *library item*, and a library item is
  // created only by an import -- `upsert_library_item` has exactly one caller,
  // and it is the import runner. A work published on this instance never gets
  // one, so the reader who just read it has nothing to mark.
  //
  // Asserting that as a fact rather than working around it, because the
  // workaround is what hid it: an earlier version of this test marked the
  // *work* id through the status door, got a 200, and then found a zero on the
  // dashboard with no visible cause. The door was writing a reading status
  // against a subject that did not exist. The route now refuses that, and the
  // 404 below is the assertion that the refusal is real on the shipped binary.
  const marked = await page.evaluate(async ([work, token]) => {
    const res = await fetch(`/api/v1/library/items/${work}/status`, {
      method: 'PUT',
      credentials: 'include',
      headers: { 'content-type': 'application/json', 'x-csrf-token': token },
      body: JSON.stringify({ status: 'finished' }),
    });
    return { status: res.status, body: await res.text() };
  }, [workId, await csrfToken(page)]);

  expect(
    marked.status,
    `a work id is not a library item id, so the status door must refuse it: ${marked.status} ${marked.body}`,
  ).toBe(404);

  // And the dashboard still reads zero, because a refused write leaves nothing
  // behind to count. A zero here is the honest number rather than the symptom
  // of an orphan row.
  await page.goto('/analytics');
  const totals = await readingTotals(page);
  expect(totals['Works finished']).toBe('0');
});

/**
 * The sticky header must not eat the viewport.
 *
 * Found while writing the tests above, and it is a layout bug that predates
 * them: the desktop nav wrapped onto three lines at 1280px, so a `position:
 * sticky` header measured 177px -- a quarter of a 720px screen -- and covered
 * every element scrolled to underneath it. That is WCAG 2.4.7 territory for
 * keyboard users, and it is also why Playwright could not click a control on
 * the write page.
 *
 * Asserted as a measurement rather than a screenshot, because the number is
 * the thing that was wrong: a nav that wraps to a second row whenever somebody
 * adds one more destination is a bug waiting for the next feature.
 */
test('the header stays one row and leaves the viewport to the page', async ({ page }) => {
  await page.goto('/sign-in');

  const measured = await page.evaluate(() => {
    const header = document.querySelector('.site-header') as HTMLElement;
    const nav = document.querySelector('.desktop') as HTMLElement;
    return {
      height: header.getBoundingClientRect().height,
      viewport: window.innerHeight,
      navScrolls: nav.scrollWidth > nav.clientWidth,
    };
  });

  // 52rem is the breakpoint where the desktop nav appears; below it the mobile
  // nav (four items plus More) takes over, which is a different layout.
  expect(measured.height).toBeLessThan(90);
  expect(measured.height / measured.viewport).toBeLessThan(0.15);

  // Sixteen destinations do not fit a 1088px bar at `gap: var(--space-5)`, so
  // the nav scrolls. That is the intended escape hatch, not a defect -- but it
  // has to be reachable, so assert it is a scroll container rather than a
  // silently clipped row.
  expect(measured.navScrolls).toBe(true);
});

test('a keyboard user can scroll a target clear of the header', async ({ page }) => {
  await page.goto('/sign-in');

  // `scroll-padding-top` on `html` is what keeps a focused element out from
  // under the bar. Without it, tabbing to a control below the fold puts the
  // focus ring where the reader cannot see it.
  const pad = await page.evaluate(
    () => getComputedStyle(document.documentElement).scrollPaddingTop,
  );
  const headerHeight = await page.evaluate(
    () => (document.querySelector('.site-header') as HTMLElement).getBoundingClientRect().height,
  );
  expect(parseFloat(pad)).toBeGreaterThanOrEqual(headerHeight);
});
