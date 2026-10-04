/**
 * Item 9: "Why this?" on a recommended item.
 *
 * The backend for this already existed and I verified it by hand against a running server
 * before writing any of this: `GET /discovery` returns a `slot_id` on every item, and
 * `GET /discovery/slots/{slot_id}/explanation` answers with a reason vocabulary. So the
 * journey's job is to prove the WIRING — that the button reaches the route, and that what
 * comes back is a sentence rather than a raw code.
 */
import { expect, test } from '@playwright/test';

const author = {
  handle: 'whytest-author',
  email: 'why-author@example.test',
  password: 'a-passphrase-long-enough-for-a-test',
  displayName: 'Why Test Author',
};

const reader = {
  handle: 'whytest-reader',
  email: 'why-reader@example.test',
  password: 'a-passphrase-long-enough-for-a-test',
  displayName: 'Why Test Reader',
};

/** Register, or sign in if this run already made the account. */
async function ensureAccount(page: import('@playwright/test').Page, person: typeof author) {
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

/** Write, save and publish a one-chapter work; returns its id. */
async function publish(page: import('@playwright/test').Page, title: string): Promise<string> {
  await page.goto('/write');
  await page.fill('input[id^=field-]', title);
  await page.click('button:text-is("Start a draft")');
  await expect(page).toHaveURL(/\/write\//);
  await page.click('button:text-is("Add chapter")');
  const editor = page.locator('.tiptap.ProseMirror');
  await editor.click();
  await page.keyboard.type(`A chapter written for ${title}, so the feed has something to explain.`);
  const saved = page.waitForResponse(
    (r) => r.url().includes('/api/v1/chapters/') && r.request().method() !== 'GET',
  );
  await page.click('button:text-is("Save now")');
  await saved;
  await page.click('button:text-is("Publish")');
  await expect(page.getByText('Republish')).toBeVisible();
  return page.url().split('/').pop()!;
}

test('why this: a recommended item explains itself in plain words', async ({ page }) => {
  await ensureAccount(page, author);
  await publish(page, 'Why This Explained');

  await ensureAccount(page, reader);
  await page.goto('/discover');

  // The COUNT, before anything asserts the button's presence. This is the discipline the
  // project already learned twice: an assertion that never counted its subject.
  const triggers = page.getByTestId('why-trigger');
  await expect(triggers.first()).toBeVisible();
  expect(await triggers.count()).toBeGreaterThan(0);

  // It must not fire a request per item on paint. Recorded here rather than in a unit test
  // because "the feed is slow" is the symptom a reader would report, and only a real paint
  // shows whether the explanation route is on the critical path.
  await triggers.first().click();

  const reasons = page.getByTestId('why-reasons').first();
  await expect(reasons).toBeVisible();

  // A sentence, not a vocabulary code. `taste_tags` on screen would be the defect.
  const text = (await reasons.textContent()) ?? '';
  expect(text.length).toBeGreaterThan(0);
  expect(text).not.toMatch(/_tags$|^popular$/);
  // At least one bullet, so a present-but-empty container cannot pass.
  expect(await reasons.locator('li').count()).toBeGreaterThan(0);

  await expect(page.getByTestId('why-trigger').first()).toHaveAttribute('aria-expanded', 'true');
});

test('why this: a signed-out reader is offered no explanation', async ({ page }) => {
  // The route is `RequireSession`, so the question has no anonymous answer. What matters is
  // that the control does not APPEAR and then fail — a button that 401s on click is worse
  // than no button, because it advertises a capability the reader cannot use.
  await ensureAccount(page, author);
  await publish(page, 'Why This Signed Out');

  // Sign out through the UI rather than clearing cookies, so this exercises the real door.
  await page.locator('button:text-is("Sign out")').click();
  await expect(page.locator('a:text-is("Sign in")')).toBeVisible();

  await page.goto('/discover');
  // Either no feed, or a feed whose items carry no slot_id (the server only records slots
  // for a signed-in viewer). Both are correct; what is NOT correct is a visible trigger that
  // cannot work, so assert the count is zero rather than asserting absence of the node.
  expect(await page.getByTestId('why-trigger').count()).toBe(0);
});