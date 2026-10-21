import { expect, test, type Page } from '@playwright/test';

/**
 * The reading surface, **with JavaScript switched off** (spec §8.5, §10.1, §21.1).
 *
 * ## Why this suite exists
 *
 * Spec §8.5: *"The reader is server-rendered HTML — no JavaScript required to
 * read."* Spec §21.1: *"Accessibility is a floor: the reader works without
 * JavaScript."* And the plan's T-M5-01 asks for the whole chain — open a
 * work, read a unit, move to the next one, come back — with **the navigation
 * as a real link, not a fetch**.
 *
 * A single-page application cannot be that surface. Its content arrives after
 * the document loads, so with JavaScript off the reader gets an empty page and
 * a spinner that never resolves — and every unit test in the repository would
 * still be green, because the tests run in a browser with JavaScript.
 *
 * So this suite runs in the `no-js` Playwright project, where the browser has
 * JavaScript disabled, and it asserts the chain **through the rendered
 * document**: the text is in the page, the navigation is a link, and following
 * it reads the next unit.
 *
 * ## The authoring is done through the API, not through the browser
 *
 * The registration and the writing of the work are done with `request`, which
 * does not depend on the browser's JavaScript. That keeps the journey itself
 * about the *reading* surface, which is the thing under test: a form on the
 * registration page would be a second subject, and it would fail here for
 * reasons that have nothing to do with the reader.
 */

const PASSWORD = 'no-js-reader-passphrase-1';

interface Who {
  email: string;
  password: string;
  handle: string;
  displayName: string;
}

/** Unique per run: the scratch server keeps its database for the whole run. */
function who(handle: string, displayName = handle): Who {
  const uniq = `${handle}${Date.now().toString(36).slice(-6)}`;
  return {
    email: `${uniq.toLowerCase()}@no-js-reader.test`,
    password: PASSWORD,
    handle: uniq,
    displayName,
  };
}

/**
 * The CSRF token, read from the **cookie jar** and not from `document.cookie`.
 *
 * `document.cookie` needs JavaScript, and this project has none -- which is
 * the whole point of it. The jar is the browser's own state and is readable
 * without a script, and the token is in it because the application put it
 * there when it issued the session.
 */
async function csrfToken(page: Page): Promise<string> {
  const cookies = await page.context().cookies();
  return cookies.find((cookie) => cookie.name === 'darkhold_csrf')?.value ?? '';
}

/**
 * Register an account and publish a work with two units, through the API.
 *
 * The units carry **text**, because a unit with no text has nothing to read
 * and the journey would pass on an empty page.
 */
async function publishWorkWithTwoUnits(
  page: Page,
  author: Who,
): Promise<{ workId: string; firstUnit: string; secondUnit: string }> {
  const request = page.request;

  const registered = await request.post('/api/v1/auth/register', {
    data: {
      email: author.email,
      password: author.password,
      handle: author.handle,
      display_name: author.displayName,
      age_band: 'adult',
    },
  });
  expect(registered.status()).toBe(201);

  // Every state-changing request after this point carries the token.
  const csrf = await csrfToken(page);
  expect(csrf, 'the application issued a CSRF token with the session').not.toBe('');
  const headers = { 'x-csrf-token': csrf };

  const created = await request.post('/api/v1/works', {
    data: { title: 'A Work Read Without JavaScript' },
    headers,
  });
  expect(created.status()).toBe(201);
  const workId = (await created.json()).id as string;

  const units: string[] = [];
  for (const [index, title] of ['The First Unit', 'The Second Unit'].entries()) {
    const unit = await request.post(`/api/v1/works/${workId}/chapters`, {
      data: { title },
      headers,
    });
    expect(unit.status()).toBe(201);
    const unitId = (await unit.json()).id as string;

    const written = await request.patch(`/api/v1/chapters/${unitId}`, {
      data: {
        expected_version: 1,
        document: {
          type: 'doc',
          content: [
            {
              type: 'paragraph',
              content: [
                { type: 'text', text: `The body of unit number ${index + 1}.` },
              ],
            },
          ],
        },
      },
      headers,
    });
    expect(written.status()).toBe(200);
    units.push(unitId);
  }

  const published = await request.post(`/api/v1/works/${workId}/publish`, {
    data: { expected_version: 1 },
    headers,
  });
  expect(published.status()).toBe(200);

  return { workId, firstUnit: units[0], secondUnit: units[1] };
}

/**
 * The chain is the claim, and the claim is about a browser with no
 * JavaScript -- so this file runs in the `no-js` Playwright project and
 * NOWHERE else. The selection is by file, in `playwright.config.ts`, rather
 * than by a conditional skip here: a suite that skips itself has to know the
 * project's name, and the file is the thing that knows.
 */
test.describe('the reader with JavaScript switched off', () => {
  test('a reader opens a work, reads a unit, moves to the next one and comes back', async ({
    page,
  }) => {
    const author = who('NoJsReader');
    const { workId, firstUnit, secondUnit } = await publishWorkWithTwoUnits(page, author);

    // **Open the work.** The reader's URL, followed as a real link.
    await page.goto(`/works/${workId}/units/${firstUnit}`);

    // **The text is in the document.** Not fetched, not waiting for a script:
    // the page arrived with it.
    await expect(page.locator('.reader-surface__body')).toContainText(
      'The body of unit number 1.',
    );

    // **The navigation is a link.** A `<a href>`, not a handler.
    const next = page.locator('.reader-surface__next');
    await expect(next).toHaveAttribute('href', `/works/${workId}/units/${secondUnit}`);

    // **Move to the next one.**
    await next.click();
    await expect(page).toHaveURL(new RegExp(`/works/${workId}/units/${secondUnit}`));
    await expect(page.locator('.reader-surface__body')).toContainText(
      'The body of unit number 2.',
    );

    // **And come back.** The browser's own history, and the "previous" link.
    await page.goBack();
    await expect(page.locator('.reader-surface__body')).toContainText(
      'The body of unit number 1.',
    );
    await page.locator('.reader-surface__prev').click();
    await expect(page.locator('.reader-surface__body')).toContainText(
      'The body of unit number 1.',
    );
  });

  test('the position save is a form post, and it is not a failure', async ({
    page,
  }) => {
    const author = who('NoJsPosition');
    const { workId, firstUnit } = await publishWorkWithTwoUnits(page, author);

    await page.goto(`/works/${workId}/units/${firstUnit}`);

    // The form is **on the page**, and it is a POST. A control that only
    // exists when JavaScript runs is a control that fails when it does not.
    const form = page.locator('.reader-surface__position');
    await expect(form).toHaveAttribute('method', 'post');
    await expect(form).toHaveAttribute('action', `/works/${workId}/units/${firstUnit}/position`);

    // Press it. The reader is returned to the page they were on -- not to an
    // error page, and not to a login wall.
    await form.locator('button[type=submit]').click();
    await expect(page).toHaveURL(new RegExp(`/works/${workId}/units/${firstUnit}`));
    await expect(page.locator('.reader-surface__body')).toContainText(
      'The body of unit number 1.',
    );
  });

  test('the table of contents is on the page and every entry is a link', async ({
    page,
  }) => {
    const author = who('NoJsToc');
    const { workId, firstUnit, secondUnit } = await publishWorkWithTwoUnits(page, author);

    await page.goto(`/works/${workId}/units/${firstUnit}`);

    const toc = page.locator('.reader-surface__toc');
    await expect(toc).toBeVisible();
    await expect(toc.locator('a')).toHaveCount(2);
    await expect(toc.locator(`a[href="/works/${workId}/units/${secondUnit}"]`)).toHaveCount(1);
  });
});
