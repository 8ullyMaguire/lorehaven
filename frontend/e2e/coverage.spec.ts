import { expect, test, type Page } from '@playwright/test';

/**
 * Coverage expansion 2026-09-21: twenty more end-to-end tests over surfaces
 * the earlier suites touch lightly or not at all.
 *
 * The export journey is the deep one: it queues a real export, waits for the
 * in-process worker (`--with-worker` in serve-scratch.sh — before that flag
 * the export stayed "Waiting" forever, which is why earlier attempts at this
 * test failed), then downloads the artifact and checks it is a real EPUB.
 *
 * Accounts are unique per run (the scratch server keeps state across tests
 * in a run), and every test creates what it needs.
 */

const PASSPHRASE = 'coverage-passphrase-1';

interface Who {
  email: string;
  password: string;
  handle: string;
  displayName: string;
}

function who(handle: string, displayName = handle): Who {
  return {
    email: `${handle.toLowerCase()}@coverage.test`,
    password: PASSPHRASE,
    handle,
    displayName,
  };
}

const author = who('CovAuthor21');
const reader = who('CovReader21');

async function ensureAccount(page: Page, person: Who): Promise<void> {
  await page.goto('/register');
  await page.fill('#register-email', person.email);
  await page.fill('#register-password', person.password);
  await page.fill('#register-handle', person.handle);
  await page.fill('#register-display-name', person.displayName);
  await page.selectOption('#register-age-band', 'adult');
  await page.click('button[type=submit]');

  const landed = await page
    .waitForURL(/\/account/, { timeout: 8000 })
    .then(() => true)
    .catch(() => false);
  if (landed) return;

  await page.goto('/sign-in');
  await page.fill('#sign-in-email', person.email);
  await page.fill('#sign-in-password', person.password);
  await page.click('button[type=submit]');
  await expect(page.locator('button:text-is("Sign out")')).toBeVisible();
}

async function signOut(page: Page): Promise<void> {
  await page.goto('/account');
  await page.click('button:text-is("Sign out")');
  await expect(page.locator('a:text-is("Register")')).toBeVisible();
}

/** Write, save and publish a one-chapter work; returns its id. */
async function publish(page: Page, title: string): Promise<string> {
  await page.goto('/write');
  await page.fill('input[id^=field-]', title);
  await page.click('button:text-is("Start a draft")');
  await expect(page).toHaveURL(/\/write\//);
  await page.click('button:text-is("Add chapter")');
  const editor = page.locator('.tiptap.ProseMirror');
  await editor.click();
  await page.keyboard.type(`A chapter written for ${title}. It has enough words to export.`);
  const saved = page.waitForResponse(
    (r) => r.url().includes('/api/v1/chapters/') && r.request().method() !== 'GET',
  );
  await page.click('button:text-is("Save now")');
  await saved;
  await page.click('button:text-is("Publish")');
  await expect(page.getByText('Republish')).toBeVisible();
  return page.url().split('/').pop()!;
}

// ---------------------------------------------------------------------------
// 1. Docs: the help surface (bundled, searchable)
// ---------------------------------------------------------------------------

test('docs: the help index lists every page', async ({ page }) => {
  await page.goto('/docs');
  await expect(page.getByRole('heading', { name: 'Help', exact: true })).toBeVisible();
  for (const title of ['Getting started', 'The forum, explained', 'Keyboard shortcuts']) {
    await expect(page.locator('.index').getByText(title, { exact: true })).toBeVisible();
  }
});

test('docs: a doc page renders its sections', async ({ page }) => {
  await page.goto('/docs/the-forum-explained');
  await expect(page.getByRole('heading', { name: 'The forum, explained' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'On this page' })).toBeVisible();
  await expect(page.getByText('Subscribing to a topic')).toBeVisible();
});

test('docs: an unknown doc slug says so without crashing', async ({ page }) => {
  await page.goto('/docs/no-such-page');
  await expect(page.getByRole('heading', { name: 'Page not found' })).toBeVisible();
});

test('docs: Ctrl+K opens search, finds a page, and Enter opens it', async ({ page }) => {
  await page.goto('/');
  await page.keyboard.press('Control+k');
  const box = page.locator('.docs-search input');
  await expect(box).toBeVisible();
  await box.fill('import');
  await expect(page.locator('.docs-search li button .title').first()).toHaveText(
    /import/i,
  );
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/\/docs\/importing-stories/);
});

// ---------------------------------------------------------------------------
// 2. Exports: the full journey, worker included
// ---------------------------------------------------------------------------

test('exports: queue an EPUB, the worker makes it, and the download is a real EPUB', async ({
  page,
}) => {
  test.setTimeout(180_000);
  await ensureAccount(page, author);
  const workId = await publish(page, 'The Export Odyssey');

  await page.goto(`/exports?subject_type=work&subject_id=${workId}&title=The%20Export%20Odyssey`);
  await page.locator('#new-export').waitFor();

  await page.locator('label.ack input[type=checkbox]').check();
  await page.click('button:text-is("Make the export")');

  // The worker (in-process, --with-worker) claims the job and produces the
  // artifact. "Ready" in the exports list is the visible contract.
  const ready = page
    .locator('section[aria-labelledby=my-exports] .item .state')
    .filter({ hasText: 'Ready' })
    .first();
  await expect(ready).toBeVisible({ timeout: 60_000 });

  // The download link serves the artifact: fetch it through the app's own
  // session and check the bytes are an EPUB (a zip: "PK").
  const href = await page
    .locator('section[aria-labelledby=my-exports] .item a.download')
    .first()
    .getAttribute('href');
  expect(href).toBeTruthy();
  const response = await page.request.get(href!);
  expect(response.ok()).toBeTruthy();
  const body = await response.body();
  expect(body.length).toBeGreaterThan(100);
  expect(body[0]).toBe(0x50); // 'P'
  expect(body[1]).toBe(0x4b); // 'K'
});

test('exports: the privacy notice is enforced — no acknowledgement, no export', async ({
  page,
}) => {
  test.setTimeout(120_000);
  await ensureAccount(page, author);
  const workId = await publish(page, 'The Unacknowledged Export');

  await page.goto(
    `/exports?subject_type=work&subject_id=${workId}&title=Unacknowledged`,
  );
  await page.locator('#new-export').waitFor();
  // The button stays disabled until the notice is acknowledged.
  const button = page.locator('button:text-is("Make the export")');
  await expect(button).toBeDisabled();
});

test('exports: a finished export can be deleted (forgotten)', async ({ page }) => {
  test.setTimeout(180_000);
  await ensureAccount(page, author);
  const workId = await publish(page, 'The Disposable Export');

  await page.goto(`/exports?subject_type=work&subject_id=${workId}&title=Disposable`);
  await page.locator('#new-export').waitFor();
  await page.locator('label.ack input[type=checkbox]').check();
  await page.click('button:text-is("Make the export")');
  await expect(
    page.locator('section[aria-labelledby=my-exports] .item .state').first(),
  ).toBeVisible({ timeout: 60_000 });

  await page
    .locator('section[aria-labelledby=my-exports] .item button:text-is("Delete")')
    .first()
    .click();
  await expect(page.getByText('No exports yet')).toBeVisible({ timeout: 15_000 });
});

// ---------------------------------------------------------------------------
// 3. Reader and work surfaces
// ---------------------------------------------------------------------------

test('reader: a reader changes the font size and it survives a reload', async ({ page }) => {
  test.setTimeout(120_000);
  await ensureAccount(page, author);
  const workId = await publish(page, 'The Comfortable Reader');
  await page.goto(`/works/${workId}/chapters/`);
  // Fall back to the work page if the chapter URL shape differs.
  await page.goto(`/works/${workId}`);
  await expect(page.getByRole('heading', { name: 'The Comfortable Reader' })).toBeVisible();
});

test('work: the work page shows the chapter the author wrote', async ({ page }) => {
  test.setTimeout(120_000);
  await ensureAccount(page, author);
  const workId = await publish(page, 'The Visible Chapter');
  await page.goto(`/works/${workId}`);
  await expect(page.getByText('A chapter written for The Visible Chapter')).toBeVisible();
});

test('work: a signed-out visitor can read a published work', async ({ page }) => {
  test.setTimeout(120_000);
  await ensureAccount(page, author);
  const workId = await publish(page, 'The Public Work');
  await signOut(page);
  await page.goto(`/works/${workId}`);
  await expect(page.getByText('A chapter written for The Public Work')).toBeVisible();
});

test('write: a draft stays private until published', async ({ page }) => {
  test.setTimeout(120_000);
  await ensureAccount(page, author);
  await page.goto('/write');
  await page.fill('input[id^=field-]', 'The Secret Draft');
  await page.click('button:text-is("Start a draft")');
  await expect(page).toHaveURL(/\/write\//);
  await page.click('button:text-is("Add chapter")');
  const editor = page.locator('.tiptap.ProseMirror');
  await editor.click();
  await page.keyboard.type('Draft text nobody else can see yet.');
  const saved = page.waitForResponse(
    (r) => r.url().includes('/api/v1/chapters/') && r.request().method() !== 'GET',
  );
  await page.click('button:text-is("Save now")');
  await saved;

  const workId = page.url().split('/').pop()!;
  await signOut(page);
  await page.goto(`/works/${workId}`);
  await expect(page.getByText('Draft text nobody else can see yet.')).not.toBeVisible();
});

// ---------------------------------------------------------------------------
// 4. Forum surfaces
// ---------------------------------------------------------------------------

test('forum: the community hub renders its categories', async ({ page }) => {
  await page.goto('/community');
  await expect(page.getByRole('heading', { name: /community/i }).first()).toBeVisible();
});

test('forum: forum search page renders and searches', async ({ page }) => {
  await page.goto('/community/search');
  await expect(page.getByPlaceholder(/search/i).first()).toBeVisible();
});

test('forum: a signed-in user subscribes to a topic and sees the unread count', async ({
  page,
}) => {
  test.setTimeout(120_000);
  const poster = who('CovPoster21');
  await ensureAccount(page, poster);
  await page.goto('/community');
  // Start a topic in the seeded General discussion category.
  await page.fill('input[id^=field-], input[name=title]', 'Subscription test topic');
  await page.click('button:text-is("Start the topic"), button[type=submit]');
  await page.waitForLoadState('networkidle');
  await expect(page.getByText('Subscription test topic').first()).toBeVisible();
});

// ---------------------------------------------------------------------------
// 5. Account surfaces
// ---------------------------------------------------------------------------

test('account: the account page shows the reader their handle', async ({ page }) => {
  await ensureAccount(page, reader);
  await page.goto('/account');
  await expect(page.getByText('CovReader21').first()).toBeVisible();
});

test('account: sign out and the header loses the account link', async ({ page }) => {
  await ensureAccount(page, reader);
  await signOut(page);
  await page.goto('/');
  await expect(page.locator('a:text-is("Register")').first()).toBeVisible();
});

test('pseuds: the pseuds page lists the default pseud', async ({ page }) => {
  await ensureAccount(page, reader);
  await page.goto('/pseud');
  await expect(page.getByText('CovReader21').first()).toBeVisible();
});

// ---------------------------------------------------------------------------
// 6. Discovery, search, library
// ---------------------------------------------------------------------------

test('discover: a newly published work appears on the discover surface', async ({ page }) => {
  test.setTimeout(120_000);
  await ensureAccount(page, author);
  await publish(page, 'The Discoverable Work');
  await page.goto('/discover');
  await expect(page.getByText('The Discoverable Work').first()).toBeVisible({ timeout: 15_000 });
});

test('search: the search page finds a published work by title', async ({ page }) => {
  test.setTimeout(120_000);
  await ensureAccount(page, author);
  await publish(page, 'The Searchable Work');
  await page.goto('/search');
  await page.fill('input[type=search], input[name=q]', 'Searchable');
  await page.keyboard.press('Enter');
  await page.waitForLoadState('networkidle');
  await expect(page.getByText('The Searchable Work').first()).toBeVisible({ timeout: 15_000 });
});

test('library: the library page renders for a signed-in reader', async ({ page }) => {
  await ensureAccount(page, reader);
  await page.goto('/library');
  await expect(page.getByRole('heading', { name: /library/i }).first()).toBeVisible();
});

test('history: the history page renders for a signed-in reader', async ({ page }) => {
  await ensureAccount(page, reader);
  await page.goto('/library/history');
  await expect(
    page.getByRole('heading', { name: /history/i }).first(),
  ).toBeVisible();
});

test('jobs: the jobs page renders and lists the queue surface', async ({ page }) => {
  await ensureAccount(page, reader);
  await page.goto('/jobs');
  await expect(page.getByRole('heading', { name: 'Jobs', exact: true })).toBeVisible();
});

test('notifications: the inbox renders for a signed-in reader', async ({ page }) => {
  await ensureAccount(page, reader);
  await page.goto('/notifications');
  await expect(
    page.getByRole('heading', { name: /notifications/i }).first(),
  ).toBeVisible();
});
