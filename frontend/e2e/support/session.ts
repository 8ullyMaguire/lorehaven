import { expect, type Page } from '@playwright/test';

/**
 * Sign-in / sign-out for the journey suites, in one place.
 *
 * ## Why these are helpers and not `button:text-is("Sign out")` at each call site
 *
 * The header used to carry four controls for a signed-in reader — `Writing as
 * @handle`, `Settings`, `Account`, `Sign out`, plus the appearance `<select>` — and
 * every spec in `e2e/` asserted its signed-in state by waiting for
 * `button:text-is("Sign out")`. That was 22 call sites across 11 files.
 *
 * `REMAINING-2026-10-05.md` §2b collapsed those four into one account menu, because
 * the row was 234px over budget signed in and 43% of the nav sat behind a scrollbar.
 * The relocation broke all 22 sites at once, and the full suite reported 21 failures
 * that all read `element(s) not found` — pointing at a button that was no longer
 * supposed to be where it was.
 *
 * The correct response is not to put the button back. It is to have the 22 sites
 * depend on *the fact they actually mean* — "a session exists" and "there is a way to
 * end it" — rather than on one button's position. A layout change then costs zero
 * edits here, and the next layout change costs zero edits too.
 *
 * ## Why the signed-in probe is the identity line
 *
 * `Writing as @handle` is the only element in the bar that renders for a signed-in
 * reader with a pseud and for nothing else. It is a better "am I signed in?" signal
 * than any control, because it is the *state* rather than an affordance: it cannot be
 * present while signed out, and it survives any rearrangement of the menu.
 *
 * ## The `isVisible()` trap, recorded because it cost four false diagnoses
 *
 * `locator.isVisible()` does NOT wait, and `locator.isVisible({ timeout })` accepts a
 * `timeout` and ignores it — the call is a single instantaneous check. Used as a
 * branch condition after a submit it reads the DOM microseconds after the request
 * was sent, sees nothing, and takes the wrong branch.
 *
 * `expect(locator).toBeVisible({ timeout })` DOES poll. Every wait in this file is a
 * `expect`, never a bare `isVisible`.
 */

/** The identity line: present iff signed in with an active pseud. */
export const IDENTITY_LINE = 'header .controls a.writing-as';

/** The account menu's trigger. */
export const ACCOUNT_MENU_TRIGGER = 'header .controls [data-testid="menu-trigger"]';

/** Whether a session is active. Does NOT wait — see the note above. */
export async function hasSession(page: Page): Promise<boolean> {
  return page
    .locator(IDENTITY_LINE)
    .isVisible()
    .then(() => true)
    .catch(() => false);
}

/**
 * Wait until the reader is signed in.
 *
 * @throws if `timeout` passes. Use `signInOrRegister` when the account may not exist.
 */
export async function expectSignedIn(page: Page, timeout = 10_000): Promise<void> {
  await expect(page.locator(IDENTITY_LINE)).toBeVisible({ timeout });
}

/**
 * Open the account menu and click Sign out.
 *
 * Goes to `/account` first, so the reader is on a real page before the menu opens —
 * `NavMenu` closes on focus leaving the component, and a click on a page that is still
 * hydrating can land before the header has settled.
 */
export async function signOutThroughHeader(page: Page): Promise<void> {
  await page.goto('/account');
  await expect(page.locator(ACCOUNT_MENU_TRIGGER)).toBeVisible();
  await page.locator(ACCOUNT_MENU_TRIGGER).click();

  const panel = page.locator('header .controls [data-testid="menu-panel"]');
  await expect(panel).toBeVisible();
  await panel.locator('button:text-is("Sign out")').click();

  // Signed out is asserted by the thing that reappears in the row, not by an absence:
  // an absence passes just as happily when the header failed to render at all.
  await expect(page.locator('header a:text-is("Register")')).toBeVisible();
}

/** Sign in with a known account; the account must already exist. */
export async function signIn(page: Page, person: { email: string; password: string }): Promise<void> {
  await page.goto('/sign-in');
  await page.fill('#sign-in-email', person.email);
  await page.fill('#sign-in-password', person.password);
  await page.click('button[type=submit]');
  await expectSignedIn(page);
}
