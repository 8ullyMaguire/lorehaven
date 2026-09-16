import { expect, test } from '@playwright/test';

test('media catalogue serves real queries, errors and feeds at narrow widths', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/media');
  await expect(page.getByRole('heading', { name: 'Browse media' })).toBeVisible();
  await expect(page.getByRole('status')).toContainText(/\d+ results?/);
  await page.getByLabel('Search media', { exact: true }).fill('title:"quiet archive"');
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  await expect(page.getByRole('link', { name: 'Atom feed' })).toHaveAttribute('href', '/api/v1/media/feed?q=title%3A%22quiet+archive%22&format=atom');
  const href = await page.getByRole('link', { name: 'Atom feed' }).getAttribute('href');
  const response = await page.request.get(href!);
  expect(response.status()).toBe(200);
  expect(response.headers()['content-type']).toContain('application/atom+xml');
  expect(await response.text()).toContain('<feed');
  await page.getByLabel('Search media', { exact: true }).fill('(');
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  await expect(page.getByRole('alert')).toBeVisible();
  for (const width of [320, 768, 1280]) {
    await page.setViewportSize({ width, height: 900 });
    const dimensions = await page.evaluate(() => ({ content: document.documentElement.scrollWidth, viewport: document.documentElement.clientWidth }));
    expect(dimensions.content).toBeLessThanOrEqual(dimensions.viewport);
  }
  expect(errors).toEqual([]);
});
