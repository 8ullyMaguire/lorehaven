import { expect, test, type Page } from '@playwright/test';

/**
 * The common user journeys, end to end, in a real browser against the real
 * binary. These mirror the dogfood pass (docs/dogfood-2026-09-15.md) and the
 * postgres-journey steps, so the browser layer — the one that produced the
 * most real defects — stays permanently covered.
 *
 * One journey per test, each with its own accounts (the scratch server keeps
 * state across tests in a run), running serially against one server.
 */

const author = {
  email: 'e2e-author@lorehaven.test',
  password: 'e2e-author-passphrase-1',
  handle: 'E2EAuthor',
  displayName: 'E2E Author',
};

const buyer = {
  email: 'e2e-buyer@lorehaven.test',
  password: 'e2e-buyer-passphrase-2',
  handle: 'E2EBuyer',
  displayName: 'E2E Buyer',
};

const forumUser = {
  email: 'e2e-forum@lorehaven.test',
  password: 'e2e-forum-passphrase-3',
  handle: 'E2EForum',
  displayName: 'E2E Forum',
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

async function signIn(page: Page, who: typeof author): Promise<void> {
  await page.goto('/sign-in');
  await page.fill('#sign-in-email', who.email);
  await page.fill('#sign-in-password', who.password);
  await page.click('button[type=submit]');
  await expect(page.locator('button:text-is("Sign out")')).toBeVisible();
}

async function signOut(page: Page): Promise<void> {
  await page.goto('/account');
  await page.click('button:text-is("Sign out")');
  await expect(page.locator('a:text-is("Register")')).toBeVisible();
}

test('author publishes and prices, buyer buys and reads, the author is notified, discovery and search work', async ({
  page,
}) => {
  test.setTimeout(180_000);

  // The author writes and publishes.
  await register(page, author);
  await page.goto('/write');
  await page.fill('input[id^=field-]', 'The E2E Odyssey');
  await page.click('button:text-is("Start a draft")');
  await expect(page).toHaveURL(/\/write\//);
  await page.click('button:text-is("Add chapter")');
  const editor = page.locator('.tiptap.ProseMirror');
  await editor.click();
  await page.keyboard.type('Chapter One. Written by a test, read by a test.');
  // The save request is the deterministic sync point: the revision counter
  // in the chapter list renders its label across a whitespace break, so text
  // matching is unreliable there.
  const chapterSaved = page.waitForResponse(
    (r) => r.url().includes('/api/v1/chapters/') && r.request().method() !== 'GET',
  );
  await page.click('button:text-is("Save now")');
  await chapterSaved;

  await page.click('button:text-is("Publish")');
  await expect(page.getByText('Republish')).toBeVisible();

  const workId = page.url().split('/').pop()!;

  // The author prices the work from the editor (owner-only decision).
  await page.locator('#pricing-heading').waitFor();
  await page
    .locator('section:has(#pricing-heading) select')
    .first()
    .selectOption('purchase');
  await page
    .locator('section:has(#pricing-heading) input')
    .first()
    .fill('500');
  await page.click('button:text-is("Save pricing")');
  await expect(page.getByText('Pricing saved.')).toBeVisible();

  await signOut(page);

  // The buyer sees the paywall, buys, and reads.
  await register(page, buyer);
  await page.goto(`/works/${workId}`);
  await expect(page.getByText('This work is for purchase')).toBeVisible();
  await expect(page.getByText('5 EUR')).toBeVisible();
  await page.click('button:text-is("Buy now")');
  await expect(page.getByText('Your rating')).toBeVisible();

  // Reader feedback: rating, a public review, a private note.
  await page.locator('[role="radio"], button[aria-label*="star" i]').nth(4).click();
  await page.click('button:text-is("Save rating")');
  await page.fill('#review-body', 'Worth the five euros.');
  await page.locator('label:has-text("Publish this review") input').check();
  await page.click('button:text-is("Save review")');
  await expect(page.getByText('Comment posted.')).toBeVisible();
  await page.fill('#note-draft', 'Reread the kettle part.');
  await page.click('button:text-is("Add note")');
  await expect(page.getByText('Reread the kettle part.')).toBeVisible();

  // Discovery and search surface the work by name, not by uuid.
  await page.goto('/discover');
  await expect(page.getByText('The E2E Odyssey').first()).toBeVisible();
  await page.goto('/search');
  await page.fill('input[type=search]', 'E2E Odyssey');
  await expect(page.getByRole('link', { name: /The E2E Odyssey/ })).toBeVisible();

  await signOut(page);

  // The author learns about the sale and sees the 85% share.
  await signIn(page, author);
  await page.goto('/notifications');
  await expect(page.getByText('Someone bought your work')).toBeVisible();
  await page.click('button:text-is("Mark all as read")');
  await expect(page.locator('button:text-is("Mark all as read")')).toBeDisabled();

  await page.goto('/account');
  await page.click('button:text-is("Earnings"), a:text-is("Earnings")');
  await expect(page.getByText('sale').first()).toBeVisible();
  await expect(page.getByText('425').first()).toBeVisible();
});

test('forum: a signed-in reader starts a topic and replies to one', async ({
  page,
}) => {
  await register(page, forumUser);
  await page.goto('/community');
  await page.click('a:has-text("General discussion")');
  await page.fill('#topic-title', 'E2E says hello');
  await page.click('button:text-is("Start topic")');
  await expect(page.getByText('E2E says hello')).toBeVisible();

  await page.click('a:has-text("E2E says hello")');
  await page.fill('#reply-body', 'Replying from the browser suite.');
  await page.click('button:text-is("Post reply")');
  await expect(page.getByText('Replying from the browser suite.')).toBeVisible();
});
