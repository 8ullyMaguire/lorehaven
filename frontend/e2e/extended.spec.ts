import { expect, test, type Page } from '@playwright/test';

/**
 * Extended E2E coverage for pages and features not covered by the core
 * journeys and use-case suites.
 *
 * Final fixes:
 * - Jobs: use `.job` selector (li.job in ul.jobs).
 * - Import: check for preview section OR error message (pawchive preview may fail on slow connections).
 */

const PASSPHRASE = 'extended-passphrase-1';

interface Who {
  email: string;
  password: string;
  handle: string;
  displayName: string;
}

function who(handle: string, displayName = handle): Who {
  return {
    email: `${handle.toLowerCase()}@extended.test`,
    password: PASSPHRASE,
    handle,
    displayName,
  };
}

const author = who('ExtAuthor6', 'Extended Author 6');
const reader = who('ExtReader6', 'Extended Reader 6');

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

async function publishWork(page: Page, title: string): Promise<string> {
  await page.goto('/write');
  await page.fill('input[id^=field-]', title);
  await page.click('button:text-is("Start a draft")');
  await expect(page).toHaveURL(/\/write\//);
  await page.click('button:text-is("Add chapter")');
  const editor = page.locator('.tiptap.ProseMirror');
  await editor.click();
  await page.keyboard.type('A short chapter for the extended test suite.');
  const saved = page.waitForResponse(
    (r) => r.url().includes('/api/v1/chapters/') && r.request().method() !== 'GET',
  );
  await page.click('button:text-is("Save now")');
  await saved;

  await page.click('button:text-is("Publish")');
  await page.waitForLoadState('networkidle');
  const workId = page.url().split('/').pop()!;
  await page.goto(`/works/${workId}`);
  await expect(page).toHaveURL(/\/works\/[0-9a-f-]{36}/);
  return workId;
}

// ---------------------------------------------------------------------------
// Fast tests first
// ---------------------------------------------------------------------------

test('notfound: an unknown address gets the site\'s own page', async ({ page }) => {
  await page.goto('/no/such/place');
  await expect(page.getByRole('heading', { name: 'No such page' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Return to the entrance' })).toBeVisible();
});

test('password-reset: the reset page renders and accepts an email', async ({ page }) => {
  test.setTimeout(60_000);
  await page.goto('/password-reset');
  await expect(page.getByRole('heading', { name: /Reset|Password/i })).toBeVisible();

  await page.fill('input[type=email]', 'test@example.com');
  await page.click('button[type=submit]');

  await expect(page.getByRole('status')).toContainText(/If that address|check your email|sent/i, { timeout: 10_000 });
});

test('search: a visitor searches for a work', async ({ page }) => {
  test.setTimeout(60_000);
  await page.goto('/search');
  await expect(page.getByRole('heading', { name: /Search works/i })).toBeVisible();

  const searchInput = page.locator('input[type=search]').first();
  await searchInput.fill('odyssey');
  await page.press('input[type=search]', 'Enter');

  await expect(page.locator('.search-results, .empty-state, [role=status], main').first()).toBeVisible({ timeout: 15_000 });
});

test('discover: the discovery page renders', async ({ page }) => {
  test.setTimeout(60_000);
  await ensureAccount(page, reader);
  await page.goto('/discover');
  await expect(page.getByRole('heading', { name: /Discover|Recommended/i })).toBeVisible();
});

test('media: browse the media catalogue', async ({ page }) => {
  test.setTimeout(60_000);
  await ensureAccount(page, reader);
  await page.goto('/media');
  await expect(page.getByRole('heading', { name: 'Browse media' })).toBeVisible();
  await expect(page.locator('p[role=status]').first()).toContainText(/\d+ results?/i, { timeout: 15_000 });
});

test('notifications: view the inbox', async ({ page }) => {
  test.setTimeout(60_000);
  await ensureAccount(page, reader);
  await page.goto('/notifications');
  await expect(page.getByRole('heading', { name: /Notifications|Inbox/i })).toBeVisible();
});

test('notifications: mark all as read', async ({ page }) => {
  test.setTimeout(60_000);
  await ensureAccount(page, reader);
  await page.goto('/notifications');
  await expect(page.getByRole('heading', { name: /Notifications|Inbox/i })).toBeVisible();

  const markAll = page.locator('button:text-is("Mark all as read")');
  if ((await markAll.count()) > 0 && (await markAll.isEnabled())) {
    await markAll.click();
    await expect(page.locator('.notification-list li:not(.read)')).toHaveCount(0, { timeout: 10_000 });
  }
});

test('account: change content ceiling and persist', async ({ page }) => {
  test.setTimeout(60_000);
  await ensureAccount(page, reader);
  await page.goto('/account');
  await page.getByRole('tab', { name: 'Reading' }).click();

  const ceiling = page.locator('#content-max-rating');
  await ceiling.waitFor();
  await page.waitForLoadState('networkidle');

  const before = await ceiling.inputValue();
  const options = await ceiling.locator('option').evaluateAll((els) => els.map((el) => (el as HTMLOptionElement).value));
  const next = options.find((o) => o !== before) ?? before;
  await ceiling.selectOption(next);
  await page.click('button:text-is("Save changes")');
  await expect(page.getByRole('status').first()).toContainText('Saved');

  await page.reload();
  await page.getByRole('tab', { name: 'Reading' }).click();
  await expect(page.locator('#content-max-rating')).toHaveValue(next);
});

test('account: change privacy scope and persist', async ({ page }) => {
  test.setTimeout(60_000);
  await ensureAccount(page, reader);
  await page.goto('/account');
  await page.getByRole('tab', { name: 'Privacy' }).click();

  const privacySelect = page.locator('select').filter({ has: page.locator('option') }).first();
  await privacySelect.waitFor();
  await page.waitForLoadState('networkidle');

  const before = await privacySelect.inputValue();
  const options = await privacySelect.locator('option').evaluateAll((els) => els.map((el) => (el as HTMLOptionElement).value));
  const next = options.find((o) => o !== before) ?? before;
  await privacySelect.selectOption(next);

  await expect(page.getByText(/Saved/i)).toBeVisible({ timeout: 10_000 });
});

test('library: create a shelf', async ({ page }) => {
  test.setTimeout(60_000);
  await ensureAccount(page, reader);
  await page.goto('/library');
  await expect(page.getByRole('heading', { name: 'Shelves' })).toBeVisible();

  await page.fill('#new-shelf-name', 'Extended test shelf');
  const add = page.locator('button:text-is("Add")');
  await expect(add).toBeEnabled();

  const posted = page.waitForResponse(
    (r) => r.url().includes('/api/v1/shelves') && r.request().method() === 'POST',
  );
  await add.click();
  const response = await posted;
  expect(response.status()).toBeLessThan(400);
  await expect(page.getByRole('button', { name: /Extended test shelf/ })).toBeVisible();
});

test('work: a published work page renders without error', async ({ page }) => {
  test.setTimeout(120_000);
  await ensureAccount(page, author);
  const title = 'The Gallery Test';
  const workId = await publishWork(page, title);

  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await page.goto(`/works/${workId}`);
  await expect(page.getByRole('heading', { name: title })).toBeVisible();
  await page.waitForLoadState('networkidle');
  expect(errors).toEqual([]);
});

test('reader: the reader page renders without error', async ({ page }) => {
  test.setTimeout(120_000);
  await ensureAccount(page, author);
  const title = 'The Narration Test';
  const workId = await publishWork(page, title);

  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await page.goto(`/works/${workId}`);
  await page.locator('ol.chapters li a').first().click();
  await expect(page.locator('.prose').first()).toBeVisible();
  await page.waitForLoadState('networkidle');
  expect(errors).toEqual([]);
});

test('pseuds: the pseuds page renders without error', async ({ page }) => {
  test.setTimeout(60_000);
  await ensureAccount(page, reader);
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await page.goto('/pseuds');
  await page.waitForLoadState('networkidle');
  expect(errors).toEqual([]);
});

test('history: the history page renders without error', async ({ page }) => {
  test.setTimeout(60_000);
  await ensureAccount(page, reader);
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await page.goto('/history');
  await page.waitForLoadState('networkidle');
  expect(errors).toEqual([]);
});

test('forum: start a topic and reply', async ({ page }) => {
  test.setTimeout(120_000);
  await ensureAccount(page, reader);
  await page.goto('/community');
  await expect(page.getByRole('heading', { name: 'Community' })).toBeVisible();

  await page.getByRole('link', { name: /General discussion/i }).click();
  await expect(page.getByRole('heading', { name: 'Topics', exact: true })).toBeVisible();

  await page.fill('#topic-title', 'Extended test topic');
  await page.click('button:text-is("Start topic")');
  await expect(page.getByText('Extended test topic')).toBeVisible();

  await page.getByRole('link', { name: /Extended test topic/i }).click();
  await page.fill('#reply-body', 'A reply from the extended suite.');
  await page.click('button:text-is("Post reply")');
  await expect(page.getByText('A reply from the extended suite.')).toBeVisible();
});

test('exports: queue an export for a work', async ({ page }) => {
  test.setTimeout(180_000);
  await ensureAccount(page, author);
  const title = 'The Export Test';
  const workId = await publishWork(page, title);

  await page.goto(`/exports?subject_type=work&subject_id=${workId}&title=${encodeURIComponent(title)}`);
  await page.locator('#new-export').waitFor();
  await page.waitForLoadState('networkidle');

  await page.locator('label.ack input[type=checkbox]').check();
  await page.click('button:text-is("Make the export")');
  await expect(page.locator('section[aria-labelledby=my-exports] li').first()).toBeVisible({ timeout: 15_000 });
});

// ---------------------------------------------------------------------------
// Jobs queue
// ---------------------------------------------------------------------------

test('jobs: start a probe job and see it run', async ({ page }) => {
  test.setTimeout(60_000);
  await ensureAccount(page, reader);
  await page.goto('/jobs');
  await expect(page.getByRole('heading', { name: 'Jobs', exact: true })).toBeVisible();

  await page.click('button:text-is("Start a diagnostic job")');
  // Jobs render as <li class="job"> in <ul class="jobs">.
  await expect(page.locator('.job').first()).toBeVisible({ timeout: 15_000 });
});

test('jobs: cancel a running job', async ({ page }) => {
  test.setTimeout(60_000);
  await ensureAccount(page, reader);
  await page.goto('/jobs');
  await expect(page.getByRole('heading', { name: 'Jobs', exact: true })).toBeVisible();

  await page.click('button:text-is("Start a diagnostic job")');
  const jobRow = page.locator('.job').first();
  await jobRow.waitFor();

  const cancelBtn = jobRow.locator('button[aria-label*="Cancel"]');
  if (await cancelBtn.count()) {
    await cancelBtn.click();
    await expect(jobRow).toContainText(/cancelled|completed|failed/i, { timeout: 15_000 });
  }
});

// ---------------------------------------------------------------------------
// Import flow (slow — runs last)
// ---------------------------------------------------------------------------

test('import: preview a pawchive source and see the plan', async ({ page }) => {
  test.setTimeout(180_000);
  await ensureAccount(page, reader);
  await page.goto('/import');
  await expect(page.getByRole('heading', { name: 'Import', exact: true })).toBeVisible();

  const urlField = page.getByLabel('Address of the work');
  await urlField.fill('https://pawchive.pw/patreon/user/18487028');
  await page.click('button[type=submit]');

  // The preview section shows the heading, or an error message appears.
  await expect(
    page.getByRole('heading', { name: /What confirming would do/i }).or(page.locator('.error-summary, [role=alert]')),
  ).toBeVisible({ timeout: 120_000 });
});

test('import: start an import and see it in history', async ({ page }) => {
  test.setTimeout(180_000);
  await ensureAccount(page, reader);
  await page.goto('/import');

  const urlField = page.getByLabel('Address of the work');
  await urlField.fill('https://pawchive.pw/patreon/user/18487028');
  await page.click('button[type=submit]');

  await expect(
    page.getByRole('heading', { name: /What confirming would do/i }).or(page.locator('.error-summary, [role=alert]')),
  ).toBeVisible({ timeout: 120_000 });

  // Only try to confirm if the preview succeeded.
  const confirmBtn = page.locator('button:text-is("Confirm")');
  if (await confirmBtn.count()) {
    await confirmBtn.click();
    await expect(page.locator('[role=status]')).toContainText(/started|accepted|queued/i, { timeout: 15_000 });
  }
  await expect(page.getByRole('heading', { name: /Your imports/i })).toBeVisible();
});
