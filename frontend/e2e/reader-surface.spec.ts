import { expect, test, type APIRequestContext, type Page } from '@playwright/test';
import { expectSignedIn, signOutThroughHeader } from './support/session';

/**
 * The reader surface end to end — items 14, 27 and 33 of the 100-idea audit.
 *
 * Spec: `docs/spec-reader-surface-t1.md`. Plan: `docs/plan-reader-surface-t1.md` step S7.
 *
 * ## Why these journeys and not just more component tests
 *
 * The store tests prove the queries; the component tests prove the empty cases. Neither
 * proves that the two are **wired to each other**, and the failure mode that matters
 * here is exactly a wiring failure that is invisible from either side:
 *
 *  - a component that renders a rail it is never handed, so it is always empty and the
 *    "renders nothing when empty" test still passes;
 *  - a page that fetches and then throws the result away.
 *
 * Both produce a page that looks correct in every unit test and does nothing in a
 * browser. So these journeys assert the *content* of the rail against works this suite
 * published itself.
 *
 * ## The privacy rule, verified both ways
 *
 * `most_bookmarked_is_public_and_ignores_private_bookmarks` is the one that matters: ten
 * private bookmarks on one work, one public on another, and the public one leads. It
 * was confirmed red with `AND b.is_public = 1` removed from the store query before this
 * suite was trusted.
 *
 * Always run through `scripts/fe.sh e2e`, never `playwright test` directly: the binary
 * embeds `frontend/dist` at COMPILE time, so a `vite build` alone changes nothing that
 * is served.
 */

const PASSPHRASE = 'reader-surface-passphrase-1';

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
    email: `${uniq.toLowerCase()}@reader-surface.test`,
    password: PASSPHRASE,
    handle: uniq,
    displayName,
  };
}

const author = who('RsAuthor');
const reader = who('RsReader');

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
  await expectSignedIn(page);
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

/**
 * Create a taxonomy node, or reuse the one this run already made.
 *
 * `POST /taxonomy/nodes` is how the import path creates one, so it is the real door
 * rather than a seed-script shortcut. The nodes are per-fandom and per-freeform, which
 * is what the similarity scorer distinguishes by weight.
 */
async function ensureNode(
  request: APIRequestContext,
  kind: string,
  canonical: string,
): Promise<string> {
  // The door is `POST /api/v1/taxonomy` (not `/taxonomy/nodes`) and it answers
  // `{ "node": { id, ... } }` -- the id is nested, not at the top level. Both were
  // wrong in the first version of this file, and both failed with a shape error that
  // read like a server bug rather than a wrong URL.
  const created = await request.post('/api/v1/taxonomy', { data: { kind, canonical } });
  if (created.ok()) {
    const body = await created.json();
    const id = body?.node?.id;
    if (id) return id as string;
  }

  // Already exists (canonical is unique): look it up rather than fail the journey.
  const listed = await request.get(`/api/v1/taxonomy?kind=${encodeURIComponent(kind)}`);
  if (listed.ok()) {
    const body = await listed.json();
    const nodes: Array<{ id: string; canonical?: string; label?: string }> =
      body?.nodes ?? body?.items ?? [];
    const hit = nodes.find(
      (n) => (n.canonical ?? n.label ?? '').toLowerCase() === canonical.toLowerCase(),
    );
    if (hit) return hit.id;
  }
  throw new Error(`could not obtain a ${kind} taxonomy node for "${canonical}"`);
}

async function tagWork(
  request: APIRequestContext,
  workId: string,
  nodeId: string,
): Promise<void> {
  const res = await request.post(`/api/v1/works/${workId}/tags`, {
    data: { node_id: nodeId, weight: 0 },
  });
  expect(res.ok(), `tagging ${workId} with ${nodeId} failed: ${res.status()}`).toBeTruthy();
}

async function bookmark(
  request: APIRequestContext,
  input: { subjectId: string; isPublic: boolean; note?: string },
): Promise<void> {
  const res = await request.post('/api/v1/bookmarks', {
    data: {
      subject_type: 'work',
      subject_id: input.subjectId,
      chapter_id: null,
      position_permille: null,
      note: input.note ?? '',
      is_public: input.isPublic,
    },
  });
  expect(res.ok(), `bookmarking ${input.subjectId} failed: ${res.status()}`).toBeTruthy();
}

// ---------------------------------------------------------------------------
// Item 33 — the similar-works rail, on a work page
// ---------------------------------------------------------------------------

test('similar works: a tagged work recommends a genuinely similar one', async ({ page }) => {
  await ensureAccount(page, author);

  const fandom = 'HP Similarity Fandom';
  const hp = await ensureNode(page.request, 'fandom', fandom);
  const angst = await ensureNode(page.request, 'mood', 'RsAngst');

  const subject = await publish(page, 'Rs Subject Work');
  const match = await publish(page, 'Rs Match Work');
  const unrelated = await publish(page, 'Rs Unrelated Work');

  // Subject and match share both a fandom (weight 3) and a mood (weight 1).
  await tagWork(page.request, subject, hp);
  await tagWork(page.request, subject, angst);
  await tagWork(page.request, match, hp);
  await tagWork(page.request, match, angst);
  // The unrelated work shares nothing, so it must not appear.
  await tagWork(page.request, unrelated, await ensureNode(page.request, 'fandom', 'Rs Other Fandom'));

  await page.goto(`/works/${subject}`);
  const rail = page.locator('section[aria-labelledby="similar-works-heading"]');
  await expect(rail).toBeVisible();
  // Scoped to the LIST, not the section. The heading reads "Similar to \u201cRs Subject
  // Work\u201d" -- the subject's own title is IN it -- so a section-scoped "Rs Subject
  // Work must not appear" assertion matches the heading and fails while the component
  // behaves exactly as specified. The claim is "the work must not be RECOMMENDED to
  // itself", and that is a claim about the list.
  const items = rail.locator('li');
  await expect(items).toHaveCount(1);
  await expect(items.first()).toContainText('Rs Match Work');
  await expect(rail.locator('li', { hasText: 'Rs Subject Work' })).toHaveCount(0);
  await expect(rail.locator('li', { hasText: 'Rs Unrelated Work' })).toHaveCount(0);
});

test('similar works: the score is shown, not just the word "similar"', async ({ page }) => {
  await ensureAccount(page, author);

  const hp = await ensureNode(page.request, 'fandom', 'HP Score Fandom');
  const angst = await ensureNode(page.request, 'mood', 'RsScoreAngst');

  const subject = await publish(page, 'Rs Scored Subject');
  const match = await publish(page, 'Rs Scored Match');
  await tagWork(page.request, subject, hp);
  await tagWork(page.request, subject, angst);
  await tagWork(page.request, match, hp);
  await tagWork(page.request, match, angst);

  await page.goto(`/works/${subject}`);
  // Asserted on the ACCESSIBLE NAME with getByLabel, not on text: the rendered text is
  // "100%" and the phrase "tag overlap" lives in the aria-label, so
  // `toContainText('% tag overlap')` fails against a perfectly correct component.
  await expect(page.getByLabel('100% tag overlap')).toBeVisible();
  // And the number is on screen, not only in an attribute.
  await expect(
    page.locator('section[aria-labelledby="similar-works-heading"] li').first(),
  ).toContainText('100%');
});

// ---------------------------------------------------------------------------
// Item 27 — the public weekly leaderboard, and its privacy rule
// ---------------------------------------------------------------------------

test('most bookmarked: the leaderboard is readable signed out', async ({ page }) => {
  // The public bookmark is created FIRST, while signed in, and the page is then read
  // LOGGED OUT. This was "goto /discover and expect the rail" in the first version,
  // which fails for the right reason: a fresh instance has no public bookmarks, the
  // server correctly answers `{"works": []}`, and the component correctly renders
  // nothing. An empty rail is a designed state, not a broken door -- so the test has to
  // create the data it claims to read.
  await ensureAccount(page, author);
  const work = await publish(page, 'Rs Signed Out Favourite');
  await bookmark(page.request, { subjectId: work, isPublic: true });

  await signOutThroughHeader(page);

  await page.goto('/discover');
  // No session at all. A login wall in front of public data teaches readers the data
  // is not public.
  const rail = page.locator('section[aria-labelledby="most-bookmarked-heading"]');
  await expect(rail).toBeVisible();
  await expect(rail.getByText('Rs Signed Out Favourite')).toBeVisible();
});

test('most bookmarked: private bookmarks never outrank a public one', async ({ page }) => {
  await ensureAccount(page, reader);

  const hidden = await publish(page, 'Rs Private Favourite');
  const shown = await publish(page, 'Rs Public Favourite');

  // Ten readers who kept their bookmark private.
  for (let i = 0; i < 10; i += 1) {
    await ensureAccount(page, who(`RsPrivate${i}`));
    await bookmark(page.request, { subjectId: hidden, isPublic: false });
    if (i === 0) {
      // Back to the shared reader for the public bookmark, so the leaderboard is not
      // polluted with ten single-bookmark accounts.
      await ensureAccount(page, reader);
    }
  }
  // One reader who made theirs public.
  await bookmark(page.request, { subjectId: shown, isPublic: true });

  await page.goto('/discover');
  const rail = page.locator('section[aria-labelledby="most-bookmarked-heading"]');
  await expect(rail).toBeVisible();
  await expect(rail.getByText('Rs Public Favourite')).toBeVisible();
  await expect(rail.getByText('Rs Private Favourite')).toHaveCount(0);
  // The label is the reader-facing half of the same privacy rule.
  await expect(rail).toContainText(/public bookmarks/i);
});

// ---------------------------------------------------------------------------
// Item 14 — new in your fandoms, and its refusal to fall back
// ---------------------------------------------------------------------------

test('new in your fandoms: appears for a reader with a public bookmark in a fandom', async ({
  page,
}) => {
  await ensureAccount(page, reader);

  const fandom = await ensureNode(page.request, 'fandom', 'Rs Fandom For New');
  const seedWork = await publish(page, 'Rs Already Read');
  const newWork = await publish(page, 'Rs Brand New In Fandom');

  await tagWork(page.request, seedWork, fandom);
  await tagWork(page.request, newWork, fandom);

  // The reader bookmarked something IN that fandom, publicly. That is what gives the
  // section a subject.
  await bookmark(page.request, { subjectId: seedWork, isPublic: true });

  await page.goto('/discover');
  const section = page.locator('section[aria-labelledby="new-in-your-fandoms-heading"]');
  await expect(section).toBeVisible();
  await expect(section.getByText('Rs Brand New In Fandom')).toBeVisible();
  // The reader's own library is not "new" to them.
  await expect(section.getByText('Rs Already Read')).toHaveCount(0);
});

test('new in your fandoms: absent for a reader whose only bookmarks are private', async ({
  page,
}) => {
  await ensureAccount(page, who('RsPrivateOnly'));

  const fandom = await ensureNode(page.request, 'fandom', 'Rs Fandom Private Only');
  const seedWork = await publish(page, 'Rs Privately Read');
  const newWork = await publish(page, 'Rs New But Hidden');
  await tagWork(page.request, seedWork, fandom);
  await tagWork(page.request, newWork, fandom);

  await bookmark(page.request, { subjectId: seedWork, isPublic: false });

  await page.goto('/discover');
  // The section must be ABSENT -- not empty-with-a-heading, and not filled from every
  // fandom. A private bookmark revealing the reader's taste is the defect.
  //
  // Deliberately NOT "this work appears nowhere on the page": the discovery feed lists
  // every published work regardless of anyone's bookmarks, so that assertion fails
  // against a correct build for a reason that has nothing to do with this section. The
  // control for "a PUBLIC bookmark WOULD have produced this section" is the preceding
  // test, which uses the same fandom and the same kind of work.
  await expect(page.locator('section[aria-labelledby="new-in-your-fandoms-heading"]')).toHaveCount(0);
});

/**
 * Item 4: the leaderboard shows words and estimated reading time.
 *
 * This is the assertion that the store column actually REACHES a reader. The store half is
 * covered by 22 tests on two engines and the render half by 21 component tests, and both
 * halves can be correct while the number never arrives: the client type is `SurfaceWork`,
 * the server serialises `word_count`, and a rename on either side leaves every unit test
 * green. Only a journey that reads the rendered page can see the join.
 *
 * `publish` writes a chapter through the editor, so the count comes from a real
 * `chapter_revisions` row rather than a fixture insert — which means this also proves the
 * aggregate reaches the CURRENT revision of a chapter the reader just wrote.
 */
test('most bookmarked: the card shows words and estimated reading time', async ({ page }) => {
  await ensureAccount(page, author);
  const work = await publish(page, 'Rs Length Reported');
  await bookmark(page.request, { subjectId: work, isPublic: true });

  await page.goto('/discover');
  const rail = page.locator('section[aria-labelledby="most-bookmarked-heading"]');
  await expect(rail).toBeVisible();

  const length = rail
    .locator('li', { hasText: 'Rs Length Reported' })
    .locator('[data-testid=work-length]');
  await expect(length).toBeVisible();
  // "no words yet" would mean the aggregate found nothing, which is the specific failure
  // this asserts against: a rail that renders a card and no length at all looks identical
  // to one that renders both.
  await expect(length).not.toContainText('no words yet');
  await expect(length).toContainText('words ·');
  await expect(length).toContainText(/min|minute/);
});
