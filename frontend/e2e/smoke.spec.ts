import { test, expect, type Page } from '@playwright/test';

const VIEWPORTS = [1440, 980, 780, 480];

// The site is fully static — no network errors are expected at all.
async function expectNoHorizontalOverflow(page: Page) {
  for (const width of VIEWPORTS) {
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate(
      () =>
        new Promise<void>((resolve) => {
          requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
        })
    );
    const metrics = await page.evaluate(() => ({
      scrollWidth: document.documentElement.scrollWidth,
      innerWidth: window.innerWidth,
    }));
    expect(
      metrics.scrollWidth,
      `no horizontal overflow at ${width}px`
    ).toBeLessThanOrEqual(metrics.innerWidth);
  }
}

test.describe('static site smoke', () => {
  const pageErrors: Error[] = [];
  const consoleErrors: string[] = [];

  test.beforeEach(({ page }) => {
    pageErrors.length = 0;
    consoleErrors.length = 0;
    page.on('pageerror', (err) => pageErrors.push(err));
    page.on('console', (msg) => {
      if (msg.type() === 'error') consoleErrors.push(msg.text());
    });
  });

  test.afterEach(() => {
    expect(pageErrors).toEqual([]);
    expect(consoleErrors).toEqual([]);
  });

  test('landing page is the v2 hero', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByRole('heading', { level: 1 })).toContainText('Get a pull request.');
    await expect(page.getByText('No infrastructure, nothing to learn.')).toBeVisible();
    await expectNoHorizontalOverflow(page);
  });

  test('Get Started documents the trigger flow', async ({ page }) => {
    await page.goto('/get-started');
    await expect(page.getByRole('heading', { level: 1 })).toContainText('Drive fkst-hosted');
    await expect(page.getByRole('heading', { name: 'Install the GitHub App' })).toBeVisible();
    await expect(page.getByText('### Session Name').first()).toBeVisible();
    await expectNoHorizontalOverflow(page);
  });

  test('hero CTA reaches Get Started; nav returns Home', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Get started' }).click();
    await expect(page).toHaveURL(/\/get-started$/);
    // exact: the logo link's aria-label "FKST — home" would otherwise match too
    await page.getByRole('link', { name: 'Home', exact: true }).click();
    await expect(page).toHaveURL(/\/$/);
  });

  test('dashboard tab prompts GitHub sign-in', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Dashboard' }).click();
    await expect(page).toHaveURL(/\/dashboard$/);
    await expect(
      page.getByRole('heading', { name: 'Sign in to view your dashboard' })
    ).toBeVisible();
  });

  test('language toggle switches to 中文 and persists', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('button', { name: '中文' }).click();
    await expect(page.getByRole('heading', { level: 1 })).toContainText('得到一个拉取请求');
    await expect(page.locator('html')).toHaveAttribute('lang', 'zh');
    // persists across reload
    await page.reload();
    await expect(page.locator('html')).toHaveAttribute('lang', 'zh');
    await expect(page.getByRole('heading', { level: 1 })).toContainText('得到一个拉取请求');
  });
});
