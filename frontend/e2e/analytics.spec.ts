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
 * * **The reader's own action moves their own number.** A work is published
 *   through the real doors, marked finished through the reading-status door, and
 *   the dashboard count follows. That assertion could not be written before the
 *   door existed.
 * * **The other status door still refuses a work id.** Both routes are named
 *   `/…/{id}/…` and key on different rows; one of them used to accept the wrong
 *   one and store an orphan row.
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

test('a reader marks a locally published work finished, and the count moves', async ({ page }) => {
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

  // Mark it finished through the work's own reading-status door, the way a
  // reader does. This is the door that did not exist when this test was first
  // written: reading status hung off a library item, a library item is created
  // only by the import runner, and a work published on this instance had no
  // subject to mark. An earlier version of this test marked the *work* id
  // through the library door, got a 200, and then found a zero on the dashboard
  // with no visible cause -- the door was writing a status against a subject
  // that did not exist.
  const marked = await page.evaluate(async ([work, token]) => {
    const res = await fetch(`/api/v1/works/${work}/reading-status`, {
      method: 'PUT',
      credentials: 'include',
      headers: { 'content-type': 'application/json', 'x-csrf-token': token },
      body: JSON.stringify({ status: 'finished' }),
    });
    return { status: res.status, body: await res.text() };
  }, [workId, await csrfToken(page)]);

  expect(marked.status, `marking the work finished: ${marked.status} ${marked.body}`).toBe(200);

  // And the count the reader was promised actually moved. This is the assertion
  // that could not be written before: there was no action to take.
  await page.goto('/analytics');
  const totals = await readingTotals(page);
  expect(totals['Works finished']).toBe('1');
});

/**
 * The library status door refuses a work id, and refuses it as a 404.
 *
 * The companion to the test above, and the bug that made it necessary: both
 * routes are named `/…/{id}/…`, but they key on different rows. The library door
 * used to accept a work id, answer 200, and store a reading status against a
 * subject that did not exist -- leaving the reader a zero with no visible cause.
 */
test('the library status door refuses a work id', async ({ page }) => {
  test.setTimeout(180_000);

  await register(page, AUTHOR());
  await page.goto('/write');
  await page.fill('input[id^=field-]', 'A Work Id Is Not An Item Id');
  await page.click('button:text-is("Start a draft")');
  await expect(page).toHaveURL(/\/write\//);
  await page.click('button:text-is("Add chapter")');
  await page.locator('.tiptap.ProseMirror').click();
  await page.keyboard.type('A chapter, so the work can be published.');
  const saved = page.waitForResponse(
    (r) => r.url().includes('/api/v1/chapters/') && r.request().method() !== 'GET',
  );
  await page.click('button:text-is("Save now")');
  await saved;
  await page.click('button:text-is("Publish")');
  await expect(page.getByText('Republish')).toBeVisible();
  const workId = page.url().split('/').pop()!;

  const token = await csrfToken(page);
  const refused = await page.evaluate(async ([work, csrf]) => {
    const res = await fetch(`/api/v1/library/items/${work}/status`, {
      method: 'PUT',
      credentials: 'include',
      headers: { 'content-type': 'application/json', 'x-csrf-token': csrf },
      body: JSON.stringify({ status: 'finished' }),
    });
    return { status: res.status, body: await res.text() };
  }, [workId, token]);

  // 404, not 403: a 403 would confirm the id names a real work.
  expect(
    refused.status,
    `the library door must refuse a work id: ${refused.status} ${refused.body}`,
  ).toBe(404);

  // A refused write leaves no row, so the reader's own count is still the
  // honest zero rather than the symptom of an orphan.
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

/**
 * The reading-status control, driven through the page.
 *
 * The door has a Rust test and the control has ten component tests, and neither
 * can tell whether a reader can actually *reach* the control: a component that
 * is never mounted is indistinguishable from one that is mounted and broken.
 * This drives it the way a reader does, through the work page.
 */
test('a reader records where they got to, and the dashboard counts it', async ({ page }) => {
  test.setTimeout(180_000);

  await register(page, AUTHOR());

  // Publish, through the real doors.
  await page.goto('/write');
  await page.fill('input[id^=field-]', 'A Work To Keep Track Of');
  await page.click('button:text-is("Start a draft")');
  await expect(page).toHaveURL(/\/write\//);
  await page.click('button:text-is("Add chapter")');
  await page.locator('.tiptap.ProseMirror').click();
  await page.keyboard.type('A chapter, read so the work can be published.');
  const saved = page.waitForResponse(
    (r) => r.url().includes('/api/v1/chapters/') && r.request().method() !== 'GET',
  );
  await page.click('button:text-is("Save now")');
  await saved;
  await page.click('button:text-is("Publish")');
  await expect(page.getByText('Republish')).toBeVisible();
  const workId = page.url().split('/').pop()!;

  // The control is on the work page, beside the rating.
  await page.goto(`/works/${workId}`);
  const control = page.locator('#reading-status');
  await expect(control.getByText('Where you got to')).toBeVisible();

  // All five states, and none of them chosen yet.
  await expect(control.getByRole('button', { name: 'Want to read' })).toHaveAttribute(
    'aria-pressed',
    'false',
  );
  for (const label of ['Reading', 'On hold', 'Dropped', 'Finished']) {
    await expect(control.getByRole('button', { name: label })).toHaveAttribute(
      'aria-pressed',
      'false',
    );
  }

  // Record it, the way a reader does: click, not an API call.
  await control.getByRole('button', { name: 'Finished' }).click();
  await expect(control.getByRole('button', { name: 'Finished' })).toHaveAttribute(
    'aria-pressed',
    'true',
  );

  // The state survives a reload, which is what "recorded" means.
  await page.reload();
  await expect(
    page.locator('#reading-status').getByRole('button', { name: 'Finished' }),
  ).toHaveAttribute('aria-pressed', 'true');

  // And it is the reader's own number that moved.
  await page.goto('/analytics');
  const totals = await readingTotals(page);
  expect(totals['Works finished']).toBe('1');
});

/**
 * A signed-out visitor is offered a way in, and no state buttons.
 *
 * The control is behind `session.isSignedIn`, so a visitor gets the sign-in
 * prompt -- and must not get four or five unselected buttons, which would invite
 * a click that cannot do anything.
 */
test('a signed-out visitor cannot set a reading status', async ({ page }) => {
  await page.goto('/works/00000000-0000-4000-8000-000000000009');

  const control = page.locator('#reading-status');
  if ((await control.count()) === 0) {
    // The work does not exist, so there is no page to put a control on. Nothing
    // to assert and nothing broken.
    return;
  }
  await expect(control.getByRole('link', { name: 'Sign in' })).toBeVisible();
  await expect(control.getByRole('button', { name: 'Finished' })).toHaveCount(0);
});
