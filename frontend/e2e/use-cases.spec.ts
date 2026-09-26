import { expect, test, type Page } from '@playwright/test';

/**
 * The twenty most common use cases, end to end, in a real browser against the
 * real binary — breadth over depth, one use case per test.
 *
 * `journeys.spec.ts` documents the deep path (publish → price → buy → read →
 * be notified). This file answers a different question: can a person do the
 * ordinary things? Landing, registering, signing in and out, browsing,
 * searching, reading, rating, reviewing, taking a note, writing, publishing,
 * being notified, keeping preferences, picking an identity, posting to the
 * forum, exporting, and hitting a missing page.
 *
 * Each test makes its own account, because Playwright restarts the worker after
 * a failure and anything held in a module variable would be gone. Finding the
 * work through the search box on the way in also means every test that needs it
 * re-tests that path.
 *
 * It is not self-sufficient in the way the comment here used to claim, and the
 * difference is worth being precise about. The account is per-test; **the work
 * is not**. `findWork` looks up `WORK_TITLE`, which test 7 publishes, and ten
 * tests call it. So this file is order-dependent by design -- `workers: 1` plus
 * a numbered sequence is what makes it work -- and selecting a subset with `-g`
 * will fail any test that needs the work, with a timeout waiting for a link
 * that will never appear.
 *
 * That is an acceptable trade for a breadth-over-depth suite: publishing a work
 * per test would make twenty tests each pay for a full write-publish cycle and
 * still leave every test sharing one instance. But it has to be *stated*, since
 * the previous version of this comment asserted the opposite and sent me looking
 * for a product bug that was not there.
 */

const PASSPHRASE = 'use-case-passphrase-1';
const WORK_TITLE = 'The Use Case Chronicle';
const CHAPTER_TEXT =
  'The kettle had other plans. It sits on the counter and refuses to boil, which is the sort of thing a kettle does in a story where nothing else is going to happen. Outside, the street fills with rain and the sound of a neighbour practising an instrument badly and with conviction. This paragraph exists so the page is taller than the window, because the reader records a position only when the reader scrolls, and a test that never scrolls would prove nothing about the reader.';

interface Who {
  email: string;
  password: string;
  handle: string;
  displayName: string;
}

function who(handle: string, displayName = handle): Who {
  return {
    email: `${handle.toLowerCase()}@use-cases.test`,
    password: PASSPHRASE,
    handle,
    displayName,
  };
}

const author = who('UCAuthor', 'Use Case Author');
const reader = who('UCReader', 'Use Case Reader');
const second = who('UCSecond', 'Use Case Second');

async function signIn(page: Page, person: Who): Promise<void> {
  await page.goto('/sign-in');
  await page.fill('#sign-in-email', person.email);
  await page.fill('#sign-in-password', person.password);
  await page.click('button[type=submit]');
  await expect(page.locator('button:text-is("Sign out")')).toBeVisible();
}

/**
 * Be signed in as `person`, creating the account if this run has not made it
 * yet. Tests share one server but not one browser context, so every test that
 * needs an account comes through here: first use registers, later uses sign in.
 */
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

  // The address is taken: this person already exists in this run.
  await signIn(page, person);
}

async function signOut(page: Page): Promise<void> {
  await page.goto('/account');
  await page.click('button:text-is("Sign out")');
  await expect(page.locator('a:text-is("Register")')).toBeVisible();
}

/** The author's draft, found the way the author finds it. */
async function openDraft(page: Page): Promise<string> {
  await page.goto('/write');
  await page.getByRole('link', { name: new RegExp(WORK_TITLE) }).first().click();
  await expect(page).toHaveURL(/\/write\/[0-9a-f-]{36}/);
  return page.url().split('/').pop()!;
}

/** The published work, found through search; leaves the page on it. */
async function findWork(page: Page): Promise<string> {
  await page.goto('/search');
  await page.fill('input[type=search]', WORK_TITLE);
  await page.getByRole('link', { name: new RegExp(WORK_TITLE) }).first().click();
  await expect(page).toHaveURL(/\/works\/[0-9a-f-]{36}/);
  return page.url().match(/works\/([0-9a-f-]{36})/)![1];
}

async function openChapter(page: Page): Promise<void> {
  await page.locator('ol.chapters li a').first().click();
  await expect(page.locator('.prose').first()).toBeVisible();
}

// ---------------------------------------------------------------------------
// 1 — a visitor lands on the entrance
// ---------------------------------------------------------------------------

test('1. a visitor lands on the entrance and the instance describes itself', async ({ page }) => {
  const failures: string[] = [];
  page.on('pageerror', (error) => failures.push(error.message));

  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Read, write, and keep what you love.' })).toBeVisible();
  await expect(page.locator('#instance-heading')).toBeVisible();
  // The entrance search is deliberately off until the search surface lands.
  await expect(page.locator('#hero-search')).toBeDisabled();
  expect(failures, `console errors: ${failures.join('; ')}`).toEqual([]);
});

// ---------------------------------------------------------------------------
// 2, 3, 4 — the account door
// ---------------------------------------------------------------------------

test('2. a visitor registers and lands on their account', async ({ page }) => {
  await ensureAccount(page, author);
  await expect(page.getByRole('heading', { name: 'Your account' })).toBeVisible();
  await expect(page.getByText(author.email)).toBeVisible();
});

test('3. a registered account signs out and back in', async ({ page }) => {
  await ensureAccount(page, second);
  await signOut(page);
  await signIn(page, second);
  await page.goto('/account');
  await expect(page.getByRole('heading', { name: 'Your account' })).toBeVisible();
});

test('4. a visitor asks for a password reset and the answer does not name accounts', async ({ page }) => {
  await page.goto('/password-reset');
  await page.fill('#reset-email', 'nobody-here@use-cases.test');
  await page.click('button[type=submit]');
  // Answered the same way for an address that exists and one that does not.
  await expect(page.getByRole('status')).toContainText(/if .*account|reset link|email/i);
});

// ---------------------------------------------------------------------------
// 5, 6, 7, 8, 9 — the writing path
// ---------------------------------------------------------------------------

test('5. an author starts a draft from the writing surface', async ({ page }) => {
  await ensureAccount(page, author);
  await page.goto('/write');
  await page.fill('input[id^=field-]', WORK_TITLE);
  await page.click('button:text-is("Start a draft")');
  await expect(page).toHaveURL(/\/write\/[0-9a-f-]{36}/);
});

test('6. the author writes a chapter and saves it', async ({ page }) => {
  await ensureAccount(page, author);
  await openDraft(page);
  await page.click('button:text-is("Add chapter")');

  const editor = page.locator('.tiptap.ProseMirror');
  await editor.click();
  await page.keyboard.type(CHAPTER_TEXT);

  const saved = page.waitForResponse(
    (response) => response.url().includes('/api/v1/chapters/') && response.request().method() !== 'GET',
  );
  await page.click('button:text-is("Save now")');
  expect((await saved).status()).toBeLessThan(400);
});

test('7. the author publishes and the work becomes public', async ({ page }) => {
  await ensureAccount(page, author);
  await openDraft(page);
  await page.click('button:text-is("Publish")');
  await expect(page.getByText('Republish')).toBeVisible();
});

test('8. the author sees the work in their own list', async ({ page }) => {
  await ensureAccount(page, author);
  await page.goto('/write');
  await expect(page.getByRole('link', { name: new RegExp(WORK_TITLE) }).first()).toBeVisible();
});

test('9. a signed-out visitor is told that writing needs an account', async ({ page }) => {
  await page.goto('/write');
  await expect(page.getByText('Sign in to write')).toBeVisible();
  await expect(page.getByRole('link', { name: 'Create an account' })).toBeVisible();
});

// ---------------------------------------------------------------------------
// 10, 11, 12 — the reading path
// ---------------------------------------------------------------------------

test('10. a signed-out visitor reads a published work', async ({ page }) => {
  await findWork(page);
  await openChapter(page);
  await expect(page.locator('.prose').first()).toContainText('The kettle had other plans');
});

test('11. a reader opens a chapter and the reader records where they got to', async ({ page }) => {
  await ensureAccount(page, reader);
  await findWork(page);
  await openChapter(page);

  const progress = page.waitForResponse(
    (response) => response.url().includes('/reading/progress') && response.request().method() === 'PUT',
  );
  await page.mouse.wheel(0, 1200);
  expect((await progress).status()).toBeLessThan(400);
});

test('12. the work the reader opened appears in their history', async ({ page }) => {
  await ensureAccount(page, reader);
  await page.goto('/library/history');
  await expect(page.getByRole('link', { name: new RegExp(WORK_TITLE) }).first()).toBeVisible();
});

/**
 * Reading history lives at `/library/history`. The only link to it is inside the
 * mobile "More" drawer, which is not rendered at or above 52rem — so on a
 * desktop there is no way to click through to it from anywhere in the site.
 * Marked `fail`: it documents the gap, and turns red when a link is added.
 */
test('12b. a signed-in reader can reach reading history from the desktop navigation', async ({ page }) => {
  await ensureAccount(page, reader);
  await page.goto('/');
  await expect(page.locator('a[href="/library/history"]:visible').first()).toBeVisible({ timeout: 5000 });
});

// ---------------------------------------------------------------------------
// 13, 14, 15 — reading feedback
// ---------------------------------------------------------------------------

test('13. a reader rates the work', async ({ page }) => {
  await ensureAccount(page, reader);
  await findWork(page);
  await page.getByRole('radio', { name: '4 out of 5' }).click();
  await page.click('button:text-is("Save rating")');
  await expect(page.getByRole('status')).toContainText('Saved');
});

test('14. a reader posts a public review and it stands on the work page', async ({ page }) => {
  await ensureAccount(page, reader);
  await findWork(page);
  await page.fill('#review-body', 'A calm little story about a stubborn kettle.');
  await page.getByLabel(/Publish this review/).check();
  await page.click('button:text-is("Save review")');
  await expect(page.getByText('A calm little story about a stubborn kettle.')).toBeVisible();

  // A visitor sees the review too: it is public.
  await signOut(page);
  await findWork(page);
  await expect(page.getByText('A calm little story about a stubborn kettle.')).toBeVisible();
});

test('15. a reader keeps a private note that nobody else can read', async ({ page }) => {
  await ensureAccount(page, reader);
  await findWork(page);
  await page.fill('#note-draft', 'Ask about the second kettle.');
  await page.click('button:text-is("Add note")');
  await expect(page.getByText('Ask about the second kettle.')).toBeVisible();

  // Not on the public page: notes belong to the reader.
  const anon = await page.context().browser()!.newContext();
  const anonPage = await anon.newPage();
  await anonPage.goto(page.url());
  await expect(anonPage.getByText('Ask about the second kettle.')).toHaveCount(0);
  await anon.close();
});

// ---------------------------------------------------------------------------
// 16, 17, 18 — notifications and preferences
// ---------------------------------------------------------------------------

test('16. the author is told when someone replies to their topic and can clear the inbox', async ({ page }) => {
  await ensureAccount(page, author);
  await page.goto('/community');
  await page.click('a:has-text("General")');
  await page.fill('#topic-title', `Notification check ${Date.now()}`);
  await page.click('button:text-is("Start topic")');
  const topicHref = await page.locator('a:has-text("Notification check")').first().getAttribute('href');

  // Somebody else answers.
  await signOut(page);
  await ensureAccount(page, reader);
  await page.goto(topicHref!);
  await page.fill('#reply-body', 'A reply that should ring a bell.');
  await page.click('button:text-is("Post reply")');
  // Prove the reply landed before asking about the notification: a refused
  // post used to be invisible here, and the test then blamed the inbox.
  await expect(page.getByText('A reply that should ring a bell.')).toBeVisible();
  await signOut(page);

  await ensureAccount(page, author);
  await page.goto('/notifications');
  await expect(page.locator('.notification-list li').first()).toBeVisible();
  await expect(page.getByText(/replied to your topic/i)).toBeVisible();
  await page.click('button:text-is("Mark all as read")');
  await expect(page.locator('button:text-is("Mark all as read")')).toBeDisabled();
});

/**
 * The same courtesy for reviews. A public review is the loudest thing a reader
 * can do to a work, and nothing used to tell the author it happened.
 *
 * The body has to be text the positivity gate *delivers*: the default
 * preferences hold constructive criticism (`accept_constructive: false`), so a
 * neutral or negative sentence is delivered to nobody and this test would fail
 * for the wrong reason. The receipt — the only delivery signal the server sends
 * (spec §12.4) — is asserted first, so a held review explains itself here rather
 * than three steps later as a missing notification.
 */
test('16b. the author is told when their work is reviewed', async ({ page }) => {
  await ensureAccount(page, author);
  // Empty the inbox first, so the only notification that can appear afterwards
  // is the one this test is about.
  await page.goto('/notifications');
  const markAll = page.locator('button:text-is("Mark all as read")');
  // The button is rendered only when the list is non-empty, and disabled when
  // nothing is unread (test 16 above leaves it read) — so neither its presence
  // nor its state can be assumed.
  if ((await markAll.count()) > 0 && (await markAll.isEnabled())) {
    await markAll.click();
  }
  await expect(page.locator('.notification-list li:not(.read)')).toHaveCount(0);

  await signOut(page);
  await ensureAccount(page, second);
  await findWork(page);
  await page.fill('#review-body', 'Warm and well made, and the ending earns it.');
  await page.getByLabel(/Publish this review/).check();
  await page.click('button:text-is("Save review")');
  await expect(page.getByText('Comment posted.')).toBeVisible();

  await signOut(page);
  await ensureAccount(page, author);
  await page.goto('/notifications');
  // One *unread* review notification is the thing being asserted: the author's
  // inbox legitimately holds earlier ones from other tests in this file, which
  // the preamble above marked read.
  await expect(
    page.locator('.notification-list li:not(.read)').filter({ hasText: /reviewed your work/i }),
  ).toHaveCount(1, { timeout: 10_000 });
});

test('17. a reader changes their reader settings and they survive a reload', async ({ page }) => {
  await ensureAccount(page, reader);
  await findWork(page);
  await openChapter(page);
  await page.click('button:text-is("Reading settings")');

  const saved = page.waitForResponse(
    (response) => response.url().includes('/settings/typography') && response.request().method() !== 'GET',
  );
  const theme = page.getByLabel('Reader theme');
  await theme.selectOption('dark');
  // The panel renders before its own load lands; if that load overwrites the
  // choice, the save would quietly write the old value back. Say so here.
  await expect(theme, 'the choice survives until Save').toHaveValue('dark');
  await page.click('button:text-is("Save settings")');
  expect((await saved).status()).toBeLessThan(400);

  await page.reload();
  await page.click('button:text-is("Reading settings")');
  await expect(page.getByLabel('Reader theme')).toHaveValue('dark');
});

test('18. an account keeps its content ceiling', async ({ page }) => {
  await ensureAccount(page, second);
  await page.goto('/account');
  await page.getByRole('tab', { name: 'Reading' }).click();

  const ceiling = page.locator('#content-max-rating');
  await ceiling.waitFor();
  // The account page fetches these settings on mount, and both panels re-seed
  // their form from the server's copy whenever it changes — an edit made while
  // that response is in flight is discarded (the save button goes back to
  // "Saved" and disabled). Settle first so this measures the save.
  await page.waitForLoadState('networkidle');
  const before = await ceiling.inputValue();
  const options = await ceiling
    .locator('option')
    .evaluateAll((els) => els.map((el) => (el as HTMLOptionElement).value));
  const next = options.find((option) => option !== before)!;
  await ceiling.selectOption(next);
  await page.click('button:text-is("Save changes")');
  await expect(page.getByRole('status').first()).toContainText('Saved');

  await page.reload();
  await page.getByRole('tab', { name: 'Reading' }).click();
  await expect(page.locator('#content-max-rating')).toHaveValue(next);
});

test('18b. an account keeps a privacy choice', async ({ page }) => {
  await ensureAccount(page, second);
  await page.goto('/account');
  await page.getByRole('tab', { name: 'Privacy' }).click();

  const scope = page.locator('select[id^=privacy-]').first();
  await scope.waitFor();
  await page.waitForLoadState('networkidle');
  const before = await scope.inputValue();
  const options = await scope
    .locator('option')
    .evaluateAll((els) => els.map((el) => (el as HTMLOptionElement).value));
  const next = options.find((option) => option !== before);
  expect(next, 'a privacy key with more than one value').toBeTruthy();

  await scope.selectOption(next!);
  // Two panels on this page carry a "Save changes" button (the content ceiling
  // and privacy), and the click has to land in the one holding this select.
  // Picking "the first enabled button" saved the other panel's values and
  // reported success, which is how this test failed once in three runs.
  // The panel's own fieldset, which stays the same element as the button's
  // label changes. (Naming the ancestor by the label it contains re-evaluates
  // the predicate after the save and matches nothing: the label is then
  // "Saved", which is exactly the state this test asks for.)
  const panel = scope.locator('xpath=ancestor::fieldset[1]');
  await panel.locator('button:text-is("Save changes")').first().click();
  // The button reads "Save changes" while there is something to save and
  // "Saved" once it is clean. Asserting the clean state is the honest check;
  // a case-insensitive /saved/i also matches "Unsaved changes", the label the
  // panel shows *before* a save, which is how this test passed while saving
  // nothing.
  await expect(panel.locator('button:text-is("Saved")')).toBeVisible();

  await page.reload();
  await page.getByRole('tab', { name: 'Privacy' }).click();
  await expect(page.locator('select[id^=privacy-]').first()).toHaveValue(next!);
});

// ---------------------------------------------------------------------------
// 19, 20 — identity and community
// ---------------------------------------------------------------------------

test('19. a reader adds a second pseud and writes as it', async ({ page }) => {
  await ensureAccount(page, second);
  await page.goto('/pseud');
  await page.fill('#new-handle', 'UCSecondAlt');
  await page.fill('#new-display-name', 'Second Voice');
  await page.fill('#new-bio', 'A second face for the same person.');
  await page.click('button:text-is("Create pseud")');
  await expect(page.getByText('@UCSecondAlt')).toBeVisible();

  // Each pseud card carries its own "Act as this"; there is no header switcher.
  const card = page.locator('li.card').filter({ hasText: '@UCSecondAlt' });
  await card.getByRole('button', { name: 'Act as this' }).click();
  await expect(card.getByText('Acting as this')).toBeVisible();
});

test('20. a signed-in reader starts a forum topic and replies to it', async ({ page }) => {
  await ensureAccount(page, second);
  await page.goto('/community');
  await page.click('a:has-text("General discussion")');
  await page.fill('#topic-title', 'Use case says hello');
  await page.click('button:text-is("Start topic")');
  await expect(page.getByText('Use case says hello')).toBeVisible();

  await page.click('a:has-text("Use case says hello")');
  await page.fill('#reply-body', 'Replying from the use-case suite.');
  await page.click('button:text-is("Post reply")');
  await expect(page.getByText('Replying from the use-case suite.')).toBeVisible();
});

// ---------------------------------------------------------------------------
// Secondary use cases, kept because they are ordinary too
// ---------------------------------------------------------------------------

test('21. a visitor finds the work through search and discovery', async ({ page }) => {
  await findWork(page);
  await page.goto('/discover');
  await expect(page.getByText(WORK_TITLE).first()).toBeVisible();
});

test('22. a reader queues an export of a work', async ({ page }) => {
  await ensureAccount(page, reader);
  const workId = await findWork(page);
  await page.goto(`/exports?subject_type=work&subject_id=${workId}&title=${encodeURIComponent(WORK_TITLE)}`);
  await page.locator('#new-export').waitFor();
  // The page lists exports twice: once on mount, once when the session settles.
  // Queueing inside that window loses the new row, because the later response —
  // fetched before the queue existed — replaces the list wholesale. Settle
  // first, so this test measures the queue and not that race.
  await page.waitForLoadState('networkidle');
  await page.locator('label.ack input[type=checkbox]').check();
  await page.click('button:text-is("Make the export")');
  await expect(page.locator('section[aria-labelledby=my-exports] li').first()).toBeVisible();
});

test('23. a reader creates a shelf and sees their storage', async ({ page }) => {
  await ensureAccount(page, reader);
  await page.goto('/library');
  await expect(page.getByRole('heading', { name: 'Shelves' })).toBeVisible();
  await page.fill('#new-shelf-name', 'Use case shelf');
  await expect(page.locator('#new-shelf-name')).toHaveValue('Use case shelf');
  const add = page.locator('button:text-is("Add")');
  await expect(add).toBeEnabled();
  // The shelf only appears if the POST was accepted; a refused one used to
  // leave this test waiting ten seconds on a list that was never going to
  // change. Fail at the response instead, where the reason is.
  const posted = page.waitForResponse(
    (r) => r.url().includes('/api/v1/shelves') && r.request().method() === 'POST',
  );
  await add.click();
  const response = await posted;
  expect(response.status(), `creating a shelf: ${response.status()}`).toBeLessThan(400);
  await expect(page.getByRole('button', { name: /Use case shelf/ })).toBeVisible();
});

test('24. an unknown address gets the site\'s own page, not a blank screen', async ({ page }) => {
  await page.goto('/no/such/place');
  await expect(page.getByRole('heading', { name: 'No such page' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Return to the entrance' })).toBeVisible();
});
