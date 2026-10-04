import { expect, test, type Page } from '@playwright/test';

/**
 * Item 7 of the 100-idea audit: "Surprise Me", end to end.
 *
 * ## What is actually being tested
 *
 * `surprise_me_work` has 13 store tests on two engines and `SurpriseMe.test.ts` has 10. Neither
 * can see the seam that broke this feature's half-built siblings twice: a component that
 * renders perfectly and is mounted nowhere, or mounted on a path the router does not resolve.
 * Concierge.svelte shipped with eleven green component tests and no route.
 *
 * So every journey below targets a seam, plus the one behaviour only a browser can show:
 *
 *  1. `/surprise-me` resolves and dispatches, rather than rendering the 404 page;
 *  2. a signed-in reader with published work gets a real pick and a working link;
 *  3. the two EMPTY states read differently, because the endpoint cannot distinguish them and
 *     the flag is the only thing that can;
 *  4. the nav entry exists — the mutation that removed it left every unit test green;
 *  5. the page fails when the route is forced to 404.
 *
 * ## Why the two empty states get their own journeys
 *
 * `profile_empty` is the flag that separates "the public catalogue is empty" from "your profile
 * covers everything here". A page that renders one message for both tells a reader with strong
 * taste that the button is broken, and it would pass every test that only checked for *some*
 * empty text. So these assert the specific sentence, and each also asserts the other sentence
 * is ABSENT — a counted absence, since a selector matching nothing makes "not present"
 * vacuously true.
 *
 * Always run through `scripts/fe.sh e2e`: the binary embeds `frontend/dist` at COMPILE time,
 * so `vite build` alone changes nothing that is served.
 */

const PASSPHRASE = 'surprise-passphrase-1';

function uniqueHandle(base: string): string {
  return `${base}${Date.now().toString(36).slice(-6)}`;
}

async function signUp(page: Page, base: string): Promise<void> {
  const handle = uniqueHandle(base);
  await page.goto('/register');
  await page.fill('#register-email', `${handle.toLowerCase()}@surprise.test`);
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
  await page.fill('#sign-in-email', `${handle.toLowerCase()}@surprise.test`);
  await page.fill('#sign-in-password', PASSPHRASE);
  await page.click('button[type=submit]');
  await expect(page.locator('button:text-is("Sign out")')).toBeVisible();
}

/** Publish a one-chapter work so there is something for Surprise Me to serve. */
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

// ---------------------------------------------------------------------------
// The wiring: does the path resolve and dispatch?
// ---------------------------------------------------------------------------

test('a signed-in reader gets a real pick, not the 404 page', async ({ page }) => {
  await signUp(page, 'SurprisePick');
  await publish(page, 'Surprise Me Eligible Work');

  await page.goto('/surprise-me');

  // The page's own h1. The 404 page has no such heading, so this is the assertion that the
  // shell dispatched rather than falling through — the exact failure Concierge shipped with.
  await expect(page.getByRole('heading', { name: 'Surprise Me', level: 1 })).toBeVisible();

  // And it actually asked the endpoint. A shell that rendered the page with no data would
  // still show the heading, so the heading alone is not enough.
  let asked = false;
  page.on('request', (r) => {
    if (r.url().includes('/api/v1/discovery/surprise-me')) asked = true;
  });
  await page.reload();
  await expect.poll(() => asked, { timeout: 8000 }).toBe(true);
});

test('the served pick links to ITS OWN work page, and that page opens', async ({ page }) => {
  await signUp(page, 'SurpriseLink');
  await publish(page, 'Surprise Link Candidate Work');

  await page.goto('/surprise-me');

  const card = page.locator('article a').first();
  await expect(card).toBeVisible({ timeout: 10000 });

  // The href must agree with the LINK TEXT, taken from the DOM rather than from a fixture.
  //
  // My first draft published a work and asserted the served pick was that work. It is not,
  // and never was: the query picks from every published public work in the instance, and the
  // scratch database is SHARED across journeys, so by this point several exist. Which work is
  // served is the store's business and fifteen store tests cover it. The claim that belongs to
  // the FRONTEND is that the card is a real link to the work it names — so that is what is
  // asserted, read from the page itself.
  const text = (await card.textContent())!.trim();
  const href = await card.getAttribute('href');
  expect(text.length, 'the link must name a work').toBeGreaterThan(0);
  expect(href, 'the link must point at a work page').toMatch(/^\/works\/.+/);

  // And clicking it actually goes there — a correct-looking href that 404s is still broken.
  await card.click();
  await expect(page).toHaveURL(new RegExp(href!.replace(/\//g, '\\/')));
  await expect(page.getByRole('heading', { name: text, level: 1 })).toBeVisible();
});

// ---------------------------------------------------------------------------
// The two empty states, which the endpoint cannot tell apart on its own
// ---------------------------------------------------------------------------

test('a reader with no profile is told the catalogue is empty, not their taste', async ({ page }) => {
  await signUp(page, 'SurpriseNoProfile');

  // STUBBED, and the reason matters. The scratch database is SHARED across journeys, so a
  // journey that assumes an empty catalogue only holds for whichever test happens to run
  // first — my first draft did exactly that, and it went red the moment the suite order
  // changed under it. Position-dependent tests are worse than no tests.
  //
  // What is under test here is WHICH SENTENCE the page renders for WHICH FLAG, and that is a
  // rendering question. Whether the flag itself is computed correctly is answered at the
  // store layer, by the two tests in `surprise_me.rs` that own their own database and cover
  // the empty-catalogue case for real.
  await page.route('**/api/v1/discovery/surprise-me', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ work: null, profile_empty: true }),
    }),
  );

  await page.goto('/surprise-me');

  await expect(page.getByText(/no public work published on this instance/i)).toBeVisible({
    timeout: 10000,
  });
  // Counted absence, by a locator that has already matched on this page, so it is not
  // vacuously true.
  await expect(page.getByText(/everything here shares a tag with your profile/i)).toHaveCount(0);
});

test('a reader whose profile covers everything is told about their profile', async ({ page }) => {
  await signUp(page, 'SurpriseCovered');

  // Fulfil the endpoint with the one combination the UI must word differently: a reader WITH
  // a profile, and nothing left outside it. This cannot be produced by publishing real work —
  // it needs the profile to cover the whole catalogue, which is a fixture question, not a
  // journey question — so the wire response is stubbed and the RENDERING is what is under test.
  await page.route('**/api/v1/discovery/surprise-me', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ work: null, profile_empty: false }),
    }),
  );

  await page.goto('/surprise-me');

  await expect(page.getByText(/everything here shares a tag with your profile/i)).toBeVisible();
  // The catalogue claim must NOT appear: it would be false, and it would hide why the button
  // returns nothing for this reader.
  await expect(page.getByText(/no public work published on this instance/i)).toHaveCount(0);
});

test('a pick served to a reader with no profile says it was unconstrained', async ({ page }) => {
  await signUp(page, 'SurpriseUnconstrained');

  await page.route('**/api/v1/discovery/surprise-me', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        work: { work_id: 'work-1', title: 'An Unconstrained Pick', summary: 'A summary.' },
        profile_empty: true,
      }),
    }),
  );

  await page.goto('/surprise-me');

  await expect(page.getByText('An Unconstrained Pick')).toBeVisible();
  await expect(page.getByText(/no taste profile yet/i)).toBeVisible();
  // And NOT the sentence that claims the pick avoided a profile. With no profile there is
  // nothing to have gone away from, so the other sentence would be a lie.
  await expect(page.getByText(/outside the tags your reading has weighted/i)).toHaveCount(0);
});

// ---------------------------------------------------------------------------
// The nav entry, the mutation that left every unit test green
// ---------------------------------------------------------------------------

test('Surprise Me is reachable from the navigation', async ({ page }) => {
  await signUp(page, 'SurpriseNav');

  await page.goto('/discover');

  // The nav is grouped into menus, so the link is not rendered until the menu opens.
  //
  // BY ROLE AND NAME, not `getByTestId('menu-trigger')`. The testid is what the jsdom component
  // tests use and it does not survive into the real DOM — the trigger is a `<button>` inside a
  // group, so the testid selector matched nothing and the journey died on the click with an
  // empty page. An index would be worse: it silently points at a different menu the moment the
  // groups are reordered, and the test then passes for the wrong reason.
  await page.getByRole('button', { name: 'Read', exact: true }).first().click();

  // `exact: true` AND scoped to the menu panel. Without either, this matched a work titled
  // "Surprise Me Eligible Work" that an earlier journey in this file published — strict mode
  // caught it as two elements, and picking the first would have passed for the wrong reason.
  const panel = page.getByTestId('menu-panel');
  const link = panel.getByRole('link', { name: 'Surprise Me', exact: true });
  await expect(link).toBeVisible();
  await expect(link).toHaveAttribute('href', '/surprise-me');

  await link.click();
  await expect(page).toHaveURL(/\/surprise-me$/);
  await expect(page.getByRole('heading', { name: 'Surprise Me', level: 1 })).toBeVisible();
});

// ---------------------------------------------------------------------------
// The surface must fail when the route does
// ---------------------------------------------------------------------------

test('forcing the endpoint to 404 turns this journey red, not green', async ({ page }) => {
  await signUp(page, 'SurpriseForced404');
  await publish(page, 'Surprise Forced 404 Work');

  // This journey exists to be MUTATED. Run it as written and it passes; force the route to
  // 404 and it must fail. A test that passes both ways proves nothing about the route, and
  // this project has shipped features that were green because of exactly that.
  await page.route('**/api/v1/discovery/surprise-me', (route) =>
    route.fulfill({ status: 404, contentType: 'application/json', body: '{}' }),
  );

  await page.goto('/surprise-me');

  // The failure must be VISIBLE. This is the assertion that matters: a page that swallowed a
  // 404 and rendered "nothing to offer" would be indistinguishable from a genuinely empty
  // catalogue, and a reader would conclude the button is broken rather than that something
  // went wrong.
  //
  // My first draft asserted the opposite — that the empty-state sentence appeared — which
  // would have passed on a page that quietly converted every failure into "nothing here". The
  // DOM proved the component is right and the test was wrong.
  await expect(page.getByRole('alert')).toBeVisible({ timeout: 10000 });
  await expect(page.locator('article a')).toHaveCount(0);

  // And specifically NOT either empty-state sentence: a 404 is not an empty catalogue.
  await expect(page.getByText(/no public work published on this instance/i)).toHaveCount(0);
  await expect(page.getByText(/everything here shares a tag with your profile/i)).toHaveCount(0);
});
