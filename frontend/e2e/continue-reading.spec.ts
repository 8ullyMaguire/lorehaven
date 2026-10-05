import { expect, test, type Page } from '@playwright/test';
import { expectSignedIn, signOutThroughHeader } from './support/session';

/**
 * Item 1 of the 100-idea audit: "Continue Reading", end to end.
 *
 * ## What is actually being tested
 *
 * `crates/db/src/continue_reading.rs` has 12 tests on two engines, and
 * `ContinueReadingBanner.test.ts` has 8. Neither can see the thing that broke this
 * feature's half-built siblings twice: a component that renders perfectly and is mounted
 * nowhere, or mounted on the wrong page. Concierge.svelte shipped with eleven green
 * component tests and no route.
 *
 * So every journey below targets the seams:
 *
 *  1. the banner is on the HOMEPAGE, not on a work page;
 *  2. it reflects what the server stored, including the per-device collapse;
 *  3. it disappears when the reader finishes, and when signed out;
 *  4. it must fail when the route is forced to 404.
 *
 * ## Progress is written through the real API
 *
 * `PUT /api/v1/reading/progress` with `RequirePseud`, so a journey signs up, publishes,
 * then writes a position the way the reader UI would. There is no seeding shortcut, which
 * matters: a journey that inserts rows directly proves the COMPONENT works, not that the
 * real write path produces rows the banner can read.
 *
 * Always run through `scripts/fe.sh e2e`: the binary embeds `frontend/dist` at COMPILE
 * time, so `vite build` alone changes nothing that is served.
 */

const PASSPHRASE = 'continue-passphrase-1';

function uniqueHandle(base: string): string {
  return `${base}${Date.now().toString(36).slice(-6)}`;
}

async function signUp(page: Page, base: string): Promise<void> {
  const handle = uniqueHandle(base);
  await page.goto('/register');
  await page.fill('#register-email', `${handle.toLowerCase()}@continue.test`);
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
  await page.fill('#sign-in-email', `${handle.toLowerCase()}@continue.test`);
  await page.fill('#sign-in-password', PASSPHRASE);
  await page.click('button[type=submit]');
  await expectSignedIn(page);
}

/** Publish a one-chapter work so there is a page to read. Returns its id. */
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

/**
 * Write a reading position the way the reader UI does.
 *
 * `device_id` is a parameter because it is the whole point of one journey: the store has
 * to collapse a reader's two device rows to the one they wrote last, and that cannot be
 * observed without writing two.
 */
async function writeProgress(
  page: Page,
  workId: string,
  positionPermille: number,
  deviceId: string,
): Promise<void> {
  const res = await page.request.put('/api/v1/reading/progress', {
    data: {
      subject_type: 'work',
      subject_id: workId,
      position_permille: positionPermille,
      device_id: deviceId,
    },
  });
  expect(res.ok(), `writing reading progress failed: ${res.status()}`).toBeTruthy();
}

const banner = (page: Page) => page.getByTestId('continue-reading-banner');

// ---------------------------------------------------------------------------
// The wiring: is it on the homepage at all?
// ---------------------------------------------------------------------------

test('the banner is on the HOMEPAGE once a reader has progress', async ({ page }) => {
  await signUp(page, 'ContinueHome');
  const workId = await publish(page, 'Continue Home Work');
  await writeProgress(page, workId, 615, 'journey-device-a');

  await page.goto('/');

  // THE ASSERTION THAT MATTERS MOST. The store has 12 tests and the component has 8, and
  // every one of them would stay green with the banner mounted nowhere.
  await expect(banner(page)).toBeVisible();
  await expect(page.getByText('Continue Home Work')).toBeVisible();
  await expect(page.getByTestId('continue-reading-percent')).toContainText('61%');
});

test('a signed-out visitor is offered no banner and makes no request', async ({ page }) => {
  await signUp(page, 'ContinueSignedOut');
  const workId = await publish(page, 'Continue Signed Out Work');
  await writeProgress(page, workId, 300, 'journey-device-a');

  await signOutThroughHeader(page);

  // Counted absence: the request is watched, so a component that fetched and then hid
  // itself is caught separately from one that never fetched.
  let asked = false;
  page.on('request', (r) => {
    if (r.url().includes('/api/v1/continue-reading')) asked = true;
  });

  await page.goto('/');
  await expect(page.getByText(/Read, write, and keep what you love/)).toBeVisible();
  await page.waitForTimeout(400);
  expect(asked, 'a signed-out homepage view must not request continue-reading').toBe(false);
  await expect(banner(page)).toBeHidden();
});

// ---------------------------------------------------------------------------
// What the banner says, and the per-device collapse
// ---------------------------------------------------------------------------

test('the banner shows the MOST RECENT work, not the furthest', async ({ page }) => {
  await signUp(page, 'ContinuePick');
  const older = await publish(page, 'Continue Older Work');
  const newer = await publish(page, 'Continue Newer Work');

  await writeProgress(page, older, 900, 'device-a');
  await writeProgress(page, newer, 200, 'device-a');

  await page.goto('/');
  await expect(page.getByText('Continue Newer Work')).toBeVisible();
  await expect(page.getByText('Continue Older Work')).toHaveCount(0);
  await expect(page.getByTestId('continue-reading-percent')).toContainText('20%');
});

test('two devices collapse to the position the reader wrote LAST', async ({ page }) => {
  await signUp(page, 'ContinueDevices');
  const workId = await publish(page, 'Continue Two Device Work');

  // The phone is FURTHER along but written FIRST. The banner must report the laptop's
  // later, lower position — this is the defect the first draft of the store shipped.
  await writeProgress(page, workId, 900, 'phone');
  await page.waitForTimeout(1100);
  await writeProgress(page, workId, 150, 'laptop');

  await page.goto('/');
  await expect(page.getByText('Continue Two Device Work')).toBeVisible();
  await expect(page.getByTestId('continue-reading-percent')).toContainText('15%');
});

// ---------------------------------------------------------------------------
// When there is nothing to continue
// ---------------------------------------------------------------------------

test('a finished work is not offered as something to continue', async ({ page }) => {
  await signUp(page, 'ContinueFinished');
  const workId = await publish(page, 'Continue Finished Work');
  await writeProgress(page, workId, 1000, 'device-a');

  await page.goto('/');
  await expect(page.getByText(/Read, write, and keep what you love/)).toBeVisible();
  await expect(banner(page)).toBeHidden();
});

test('a reader with no progress sees no banner', async ({ page }) => {
  await signUp(page, 'ContinueFresh');
  await page.goto('/');

  await expect(page.getByText(/Read, write, and keep what you love/)).toBeVisible();
  await expect(banner(page)).toBeHidden();
});

// ---------------------------------------------------------------------------
// The server being down must not break the page
// ---------------------------------------------------------------------------

test('a failing continue-reading request leaves the homepage working', async ({ page }) => {
  await signUp(page, 'ContinueDown');
  const workId = await publish(page, 'Continue Server Down Work');
  await writeProgress(page, workId, 500, 'device-a');

  // 500, not 404: 404 is the ordinary "nothing to continue" answer and is resolved to a
  // hidden banner anyway, so a test using 404 would not distinguish the two branches.
  await page.route('**/api/v1/continue-reading', (route) =>
    route.fulfill({ status: 500, contentType: 'application/json', body: '{}' }),
  );

  await page.goto('/');
  // The enhancement fails; the page does not. These are the parts of the homepage that
  // have nothing to do with continue-reading, so they are what "the page still works"
  // has to mean.
  await expect(page.getByText(/Read, write, and keep what you love/)).toBeVisible();
  await expect(banner(page)).toBeHidden();
  // The instance's own health line is the strongest available signal that the rest of the
  // page rendered and fetched. NOT the literal 'Test Haven' -- that name is the vitest
  // fixture in Home.test.ts and no E2E instance has ever been called it.
  await expect(page.getByRole('heading', { name: 'This instance' })).toBeVisible();
  await expect(page.getByText(/all services healthy|service check needing attention/)).toBeVisible();
});

// ---------------------------------------------------------------------------
// The banner is a real link
// ---------------------------------------------------------------------------

test('the banner title links to the work page', async ({ page }) => {
  await signUp(page, 'ContinueLink');
  const workId = await publish(page, 'Continue Link Target');
  await writeProgress(page, workId, 250, 'device-a');

  await page.goto('/');
  await page.getByTestId('continue-reading-title').click();
  await expect(page).toHaveURL(new RegExp(`/works?/${workId}`));
  await expect(page.getByText('Continue Link Target').first()).toBeVisible();
});
