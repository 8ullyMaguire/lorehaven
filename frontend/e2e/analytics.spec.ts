import { expect, test, type Page } from '@playwright/test';

/**
 * The analytics page, in a real browser against the real binary.
 *
 * The unit suite covered this page with twenty tests and never noticed that it
 * was not in the router — a component with no route is invisible, and a test
 * that renders it directly cannot tell the difference. This file is the part
 * that can: every assertion goes through the URL a reader would type.
 *
 * Three things it is really about:
 *
 * * **The route exists.** `/analytics` resolves to the dashboard and not to the
 *   not-found view. That is one assertion and it is the one that was missing.
 * * **A signed-out visitor gets a way in, not an error.** The door is
 *   `RequirePseud`, so an anonymous GET is a 401, and rendered as a failure
 *   string that is a message the reader can do nothing about.
 * * **The number follows the reader's own action.** The last test publishes a
 *   work, opens it, marks it finished through the library's own status door, and
 *   then watches the dashboard's count change. A fixture that inserted the row
 *   behind the UI would pass against a page whose query never runs.
 */

const author = {
  email: 'e2e-analytics-author@lorehaven.test',
  password: 'e2e-analytics-author-pass-5',
  handle: 'E2EAnalyticsAuthor',
  displayName: 'E2E Analytics Author',
};

const reader = {
  email: 'e2e-analytics-reader@lorehaven.test',
  password: 'e2e-analytics-reader-pass-6',
  handle: 'E2EAnalyticsReader',
  displayName: 'E2E Analytics Reader',
};

async function register(page: Page, who: typeof author): Promise<void> {
  await page.goto('/register');
  await page.fill('#register-email', who.email);
  await page.fill('#register-password', who.password);
  await page.fill('#register-handle', who.handle);
  await page.fill('#register-display-name', who.displayName);
  await page.selectOption('#register-age-band', 'adult');
  await page.click('button[type=submit]');
  await expect(page).toHaveURL(/\/account/);
}

async function signOut(page: Page): Promise<void> {
  await page.goto('/account');
  await page.click('button:text-is("Sign out")');
  await expect(page.locator('a:text-is("Register")')).toBeVisible();
}

/** The four §9.6 fields, read as text so the assertion is about the render. */
async function readingTotals(page: Page): Promise<Record<string, string>> {
  await page.locator('button', { hasText: 'own.reading.basic' }).click();
  await expect(page.getByText('Works finished')).toBeVisible();
  const rows = page.locator('dl.reading > div');
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
  await register(page, reader);
  await page.goto('/analytics');

  // The heading is the assertion that matters. Before this route existed the
  // page fell through to NotFound, and the component's twenty unit tests were
  // all still green.
  await expect(page.getByRole('heading', { name: 'Your analytics' })).toBeVisible();

  // The registry is the surface, so the reader is told which trust level and
  // instance preset produced this list.
  await expect(page.getByText(/Showing what trust level \d/)).toBeVisible();

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

  await expect(page.getByText(/need an account/i)).toBeVisible();
  await expect(page.getByRole('link', { name: 'Sign in' })).toBeVisible();

  // Crucially, not the error state: a 401 is the answer to "is this page for
  // me?", not a failure to report.
  await expect(page.getByRole('alert')).toHaveCount(0);
});

test('marking a work finished moves the dashboard count', async ({ page }) => {
  test.setTimeout(180_000);

  // The author publishes a work the reader can find.
  await register(page, author);
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
  await register(page, reader);
  await page.goto(`/works/${workId}`);
  await expect(page.getByRole('heading')).toBeVisible();

  // Mark it finished through the library's own status door, the way the UI does.
  const marked = await page.evaluate(async (id) => {
    const csrf = document.cookie
      .split('; ')
      .find((c) => c.startsWith('lorehaven_csrf='))
      ?.split('=')[1] as string;
    const res = await fetch(`/api/v1/library/items/${id}/status`, {
      method: 'PUT',
      credentials: 'include',
      headers: { 'content-type': 'application/json', 'x-csrf-token': csrf },
      body: JSON.stringify({ status: 'finished' }),
    });
    return { status: res.status, body: await res.text() };
  }, workId);

  // The library item must exist for the status door to have a subject. If the
  // work page did not add one, this 404s and the dashboard count cannot move,
  // so the test reports the real cause rather than a mystery zero.
  expect(
    [200, 201, 404].includes(marked.status),
    `setting the reading status: ${marked.status} ${marked.body}`,
  ).toBe(true);
  expect(marked.status, `the library item for this work is missing: ${marked.body}`).not.toBe(404);

  await page.goto('/analytics');
  const totals = await readingTotals(page);
  expect(totals['Works finished']).toBe('1');
});
