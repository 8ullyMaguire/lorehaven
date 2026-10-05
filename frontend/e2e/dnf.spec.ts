import { expect, test, type Page } from '@playwright/test';
import { expectSignedIn, signOutThroughHeader } from './support/session';

/**
 * Item 11 of the 100-idea audit: "Mark as DNF", end to end.
 *
 * ## Why the whole server side already existed
 *
 * `did_not_finish` (0073), seven store functions, three routes and eight acceptance tests
 * shipped in M45-21. `grep -rl dnf frontend/src` returned nothing. So the risk in this
 * feature is not the query — it is that the panel is never wired, or wired to the wrong
 * route, or sends `is_public: true` when it meant to send false. Every journey below
 * targets one of those.
 *
 * ## The privacy assertions
 *
 * `bookmarks.is_public` and `did_not_finish.is_public` are the same class of field, and
 * the same rule applies: `is_public` defaults to 0 in 0073, so a client that omits the
 * field gets the private answer from the server and a client that sends `true` has
 * published a reader's disposition without being asked. The journeys assert the server's
 * stored row after each write, because "the checkbox looked right" is a claim about the
 * form and not about what was stored.
 *
 * Always run through `scripts/fe.sh e2e`: the binary embeds `frontend/dist` at COMPILE
 * time, so `vite build` alone changes nothing that is served.
 */

const PASSPHRASE = 'dnf-passphrase-1';

function uniqueHandle(base: string): string {
  return `${base}${Date.now().toString(36).slice(-6)}`;
}

async function signUp(page: Page, base: string): Promise<void> {
  const handle = uniqueHandle(base);
  await page.goto('/register');
  await page.fill('#register-email', `${handle.toLowerCase()}@dnf.test`);
  await page.fill('#register-password', PASSPHRASE);
  await page.fill('#register-handle', handle);
  await page.fill('#register-display-name', handle);
  await page.selectOption('#register-age-band', 'adult');
  await page.click('button[type=submit]');

  const landed = await page
    .waitForURL(/\/account/, { timeout: 8000 })
    .then(() => true)
    .catch(() => false);
  if (landed) return;

  await page.goto('/sign-in');
  await page.fill('#sign-in-email', `${handle.toLowerCase()}@dnf.test`);
  await page.fill('#sign-in-password', PASSPHRASE);
  await page.click('button[type=submit]');
  await expectSignedIn(page);
}

/** Publish a one-chapter work so there is a page to mark. Returns its id. */
async function publish(page: Page, title: string): Promise<string> {
  await page.goto('/write');
  await page.fill('input[id^=field-]', title);
  await page.click('button:text-is("Start a draft")');
  await expect(page).toHaveURL(/\/write\//);
  await page.click('button:text-is("Add chapter")');
  await page.locator('.tiptap.ProseMirror').click();
  await page.keyboard.type(`A chapter for ${title}. It has enough words to be a chapter.`);
  const saved = page.waitForResponse(
    (r) => r.url().includes('/api/v1/chapters/') && r.request().method() !== 'GET',
  );
  await page.click('button:text-is("Save now")');
  await saved;
  await page.click('button:text-is("Publish")');
  await expect(page.getByText('Republish')).toBeVisible();
  return page.url().split('/').pop()!;
}

/** The signed-in reader's stored row, straight from the server. */
async function storedDnf(page: Page, workId: string) {
  const res = await page.request.get(`/api/v1/works/${workId}/dnf`);
  expect(res.ok(), `reading back the DNF row failed: ${res.status()}`).toBeTruthy();
  return (await res.json()) as {
    reason: string;
    note: string | null;
    is_public: boolean;
  };
}

const panel = (page: Page) => page.locator('section[aria-labelledby="dnf-heading"]');

// ---------------------------------------------------------------------------
// The wiring: does the panel exist on the page at all?
// ---------------------------------------------------------------------------

test('the DNF panel is reachable from a work page, signed in', async ({ page }) => {
  await signUp(page, 'DnfReader');
  const workId = await publish(page, 'Dnf Reachable Work');

  await page.goto(`/works/${workId}`);
  // THE ASSERTION THAT MATTERS MOST. Everything else assumes this element is on the page.
  // A panel with 20 passing component tests and no mount point is the failure mode this
  // whole feature was born from — the server side shipped and nothing referenced it.
  await expect(panel(page)).toBeVisible();
  await expect(page.getByTestId('dnf-open')).toBeVisible();
});

test('a signed-out reader is offered no DNF control', async ({ page }) => {
  // Sign in, publish, then log out — all in one journey. My first version wrapped
  // `publish` in a `.catch(() => null)` and then asserted `expect(workId).toBeTruthy()`,
  // which is a self-inflicted failure: the catch ate the reason the work could not be
  // published and the assertion reported `null` with no cause. Each Playwright test gets
  // its own database, so a work from another journey is not reachable from here.
  await signUp(page, 'DnfSignedOut');
  const workId = await publish(page, 'Dnf Signed Out Work');

  await signOutThroughHeader(page);

  await page.goto(`/works/${workId}`);
  // Absent, not disabled: a DNF row belongs to a pseud, so a signed-out reader has no row
  // to write. Offering a control that cannot work teaches the site is broken.
  await expect(panel(page)).toHaveCount(0);
});

// ---------------------------------------------------------------------------
// The privacy rule, verified against the STORED row
// ---------------------------------------------------------------------------

test('a DNF mark is stored private unless the reader opts in', async ({ page }) => {
  await signUp(page, 'DnfPrivate');
  const workId = await publish(page, 'Dnf Private By Default');

  await page.goto(`/works/${workId}`);
  await page.getByTestId('dnf-open').click();
  await page.getByRole('radio', { name: /too slow/i }).check();
  await page.getByRole('button', { name: /^save$/i }).click();
  await expect(page.getByTestId('dnf-current')).toBeVisible();

  // Read the row back rather than reading the checkbox. THE LOAD-BEARING ASSERTION: 0073
  // defaults is_public to 0, so a client bug that sends `true` publishes a reader's
  // disposition to the author with no consent, and no DOM assertion on this page can see
  // it.
  const row = await storedDnf(page, workId);
  expect(row.reason).toBe('slow_pacing');
  expect(row.is_public).toBe(false);
  await expect(page.getByTestId('dnf-current')).toContainText('private');
});

test('the reader\'s own note is stored and is never part of the author aggregate', async ({ page }) => {
  await signUp(page, 'DnfNote');
  const workId = await publish(page, 'Dnf With A Note');

  await page.goto(`/works/${workId}`);
  await page.getByTestId('dnf-open').click();
  await page.getByRole('radio', { name: /not my taste/i }).check();
  await page.getByLabel(/note to yourself/i).fill('the prose was lovely and the plot was not');
  await page.getByRole('button', { name: /^save$/i }).click();
  await expect(page.getByTestId('dnf-current')).toBeVisible();

  const row = await storedDnf(page, workId);
  expect(row.note).toBe('the prose was lovely and the plot was not');

  // The aggregate route is the only thing an author can ever see. It is built from
  // `aggregate_dnf_counts`, which counts public rows and structured reasons only, so the
  // note must be absent from it by construction — and it is absent entirely here, because
  // this mark is private.
  const agg = await page.request.get(`/api/v1/works/${workId}/dnf/reasons`);
  expect(agg.ok()).toBeTruthy();
  const body = await agg.json();
  expect(JSON.stringify(body)).not.toContain('prose was lovely');
});

test('opting in shares the REASON, and still not the note', async ({ page }) => {
  await signUp(page, 'DnfShare');
  const workId = await publish(page, 'Dnf Shared Reason');

  await page.goto(`/works/${workId}`);
  await page.getByTestId('dnf-open').click();
  await page.getByRole('radio', { name: /author abandoned it/i }).check();
  await page.getByLabel(/note to yourself/i).fill('gave up in chapter two');
  await page.getByLabel(/share the reason with the author/i).check();
  await page.getByRole('button', { name: /^save$/i }).click();
  await expect(page.getByTestId('dnf-current')).toBeVisible();

  const row = await storedDnf(page, workId);
  expect(row.is_public).toBe(true);
  expect(row.note).toBe('gave up in chapter two');

  // The reason reaches the author's aggregate; the note does not. This is the whole design
  // of 0073: a structured reason is countable, a reader's words are not.
  const agg = await page.request.get(`/api/v1/works/${workId}/dnf/reasons`);
  const body = await agg.json();
  expect(body.reasons).toEqual([{ reason: 'abandoned_by_author', count: 1 }]);
  expect(JSON.stringify(body)).not.toContain('gave up in chapter two');
});

test('the aggregate is rendered with a correct singular', async ({ page }) => {
  await signUp(page, 'DnfAggregate');
  const workId = await publish(page, 'Dnf One Reader Stopped');

  await page.goto(`/works/${workId}`);
  await page.getByTestId('dnf-open').click();
  await page.getByRole('radio', { name: /too slow/i }).check();
  await page.getByLabel(/share the reason with the author/i).check();
  await page.getByRole('button', { name: /^save$/i }).click();
  await expect(page.getByTestId('dnf-current')).toBeVisible();

  // "1 reader stopped here", not "1 readers". The panel is showing this reader their own
  // single mark, which is exactly when a pluralised header is most obviously wrong.
  await expect(panel(page)).toContainText('1 reader stopped here');
  await expect(panel(page)).toContainText('Too slow');
});

// ---------------------------------------------------------------------------
// The edit and clear cycle
// ---------------------------------------------------------------------------

test('changing the reason overwrites rather than duplicating', async ({ page }) => {
  await signUp(page, 'DnfEdit');
  const workId = await publish(page, 'Dnf Changed My Mind');

  await page.goto(`/works/${workId}`);
  await page.getByTestId('dnf-open').click();
  await page.getByRole('radio', { name: /too slow/i }).check();
  await page.getByRole('button', { name: /^save$/i }).click();
  await expect(page.getByTestId('dnf-current')).toContainText('Too slow');

  // 0073 has a PARTIAL unique index on (pseud_id, work_id) WHERE deleted_at IS NULL, so a
  // second insert must fail and the server must upsert instead. A component that posted a
  // new row each time would show the FIRST reason here forever.
  await page.getByRole('button', { name: /clear this mark/i }).click();
  await page.getByTestId('dnf-open').click();
  await page.getByRole('radio', { name: /triggering content/i }).check();
  await page.getByRole('button', { name: /^save$/i }).click();
  await expect(page.getByTestId('dnf-current')).toContainText('Triggering content');

  const list = await page.request.get('/api/v1/me/dnf');
  const mine = (await list.json()) as Array<{ work_id: string }>;
  expect(mine.filter((r) => r.work_id === workId)).toHaveLength(1);
});

test('clearing removes the row entirely', async ({ page }) => {
  await signUp(page, 'DnfClear');
  const workId = await publish(page, 'Dnf Cleared');

  await page.goto(`/works/${workId}`);
  await page.getByTestId('dnf-open').click();
  await page.getByRole('radio', { name: /dropped for another reason/i }).check();
  await page.getByRole('button', { name: /^save$/i }).click();
  await expect(page.getByTestId('dnf-current')).toBeVisible();

  await page.getByRole('button', { name: /clear this mark/i }).click();
  await expect(page.getByTestId('dnf-open')).toBeVisible();
  // `expect(locator.textContent)` (without `await`) passes a PROMISE to expect, which
  // fails as "received is not iterable" — a Playwright API error that says nothing about
  // the page. Assert through the auto-retrying locator instead.
  await expect(page.getByTestId('dnf-message')).toContainText('Cleared');

  // A soft delete is 0073's design (deleted_at), so GET must now answer 404 rather than
  // returning the tombstone. If it returned the row, the panel would render a cleared mark.
  const res = await page.request.get(`/api/v1/works/${workId}/dnf`);
  expect(res.status()).toBe(404);
});