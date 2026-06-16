import { test, expect } from '@playwright/test';

test.describe('Smoke tests', () => {
  const pageErrors: Error[] = [];
  const consoleErrors: string[] = [];

  test.beforeEach(({ page }) => {
    pageErrors.length = 0;
    consoleErrors.length = 0;

    page.on('pageerror', (err) => {
      pageErrors.push(err);
    });

    page.on('console', (msg) => {
      if (msg.type() === 'error') {
        const text = msg.text();
        // Allowlist for API network request failures in no-backend mode
        const isAllowed =
          text.includes('/api/v1/health') ||
          text.includes('/api/v1/packages') ||
          text.includes('/api/v1/sessions') ||
          text.includes('net::ERR_CONNECTION_REFUSED') ||
          text.includes('Failed to load resource');
        if (!isAllowed) {
          consoleErrors.push(text);
        }
      }
    });
  });

  test.afterEach(() => {
    expect(pageErrors).toEqual([]);
    expect(consoleErrors).toEqual([]);
  });

  test('packages page smoke test', async ({ page }) => {
    await page.goto('/packages');

    // (a) /packages renders the intro lede
    await expect(page.getByText('Packages are the behavior layer')).toBeVisible();

    if (process.env.VITE_FKST_API_BASE) {
      // If live backend env var is set, verify we can render the page with real data
      await expect(page.getByText('roots scanned').first()).toBeVisible();
    } else {
      // Honest unreachable state when backend is down
      await expect(page.getByText('package store unreachable — unknown').first()).toBeVisible();
    }

    // (c) NO horizontal overflow: document.documentElement.scrollWidth <= window.innerWidth
    // at 1440, 980, 780, 480 viewports
    const viewports = [1440, 980, 780, 480];
    for (const width of viewports) {
      await page.setViewportSize({ width, height: 800 });
      // requestAnimationFrame round-trip to let layout settle
      await page.evaluate(() => {
        return new Promise<void>((resolve) => {
          requestAnimationFrame(() => {
            requestAnimationFrame(() => {
              resolve();
            });
          });
        });
      });
      const metrics = await page.evaluate(() => {
        return {
          scrollWidth: document.documentElement.scrollWidth,
          innerWidth: window.innerWidth,
        };
      });
      console.log(`[Packages] Viewport: ${width}, scrollWidth: ${metrics.scrollWidth}, innerWidth: ${metrics.innerWidth}`);
      expect(metrics.scrollWidth).toBeLessThanOrEqual(metrics.innerWidth);
    }
  });

  test('overview page smoke test', async ({ page }) => {
    await page.goto('/overview');

    // (b) /overview renders the four stage names
    await expect(page.getByText('Design', { exact: true })).toBeVisible();
    await expect(page.getByText('Build', { exact: true })).toBeVisible();
    await expect(page.getByText('Review', { exact: true })).toBeVisible();
    await expect(page.getByText('Ship', { exact: true })).toBeVisible();

    // (c) NO horizontal overflow
    const viewports = [1440, 980, 780, 480];
    for (const width of viewports) {
      await page.setViewportSize({ width, height: 800 });
      await page.evaluate(() => {
        return new Promise<void>((resolve) => {
          requestAnimationFrame(() => {
            requestAnimationFrame(() => {
              resolve();
            });
          });
        });
      });
      const metrics = await page.evaluate(() => {
        return {
          scrollWidth: document.documentElement.scrollWidth,
          innerWidth: window.innerWidth,
        };
      });
      console.log(`[Overview] Viewport: ${width}, scrollWidth: ${metrics.scrollWidth}, innerWidth: ${metrics.innerWidth}`);
      expect(metrics.scrollWidth).toBeLessThanOrEqual(metrics.innerWidth);
    }
  });

  test('settings page smoke test', async ({ page }) => {
    await page.goto('/settings');

    // (e) /settings renders posture unknown
    await expect(page.locator('body')).toContainText('posture as of unknown');

    // (c) NO horizontal overflow
    const viewports = [1440, 980, 780, 480];
    for (const width of viewports) {
      await page.setViewportSize({ width, height: 800 });
      await page.evaluate(() => {
        return new Promise<void>((resolve) => {
          requestAnimationFrame(() => {
            requestAnimationFrame(() => {
              resolve();
            });
          });
        });
      });
      const metrics = await page.evaluate(() => {
        return {
          scrollWidth: document.documentElement.scrollWidth,
          innerWidth: window.innerWidth,
        };
      });
      console.log(`[Settings] Viewport: ${width}, scrollWidth: ${metrics.scrollWidth}, innerWidth: ${metrics.innerWidth}`);
      expect(metrics.scrollWidth).toBeLessThanOrEqual(metrics.innerWidth);
    }
  });

  test('goals page smoke test', async ({ page }) => {
    await page.goto('/goals');

    // Cheap smoke: verify filters render
    await expect(page.getByText('Stage all').first()).toBeVisible();
    await expect(page.getByText('Repo all').first()).toBeVisible();

    // (c) NO horizontal overflow
    const viewports = [1440, 980, 780, 480];
    for (const width of viewports) {
      await page.setViewportSize({ width, height: 800 });
      await page.evaluate(() => {
        return new Promise<void>((resolve) => {
          requestAnimationFrame(() => {
            requestAnimationFrame(() => {
              resolve();
            });
          });
        });
      });
      const metrics = await page.evaluate(() => {
        return {
          scrollWidth: document.documentElement.scrollWidth,
          innerWidth: window.innerWidth,
        };
      });
      console.log(`[Goals] Viewport: ${width}, scrollWidth: ${metrics.scrollWidth}, innerWidth: ${metrics.innerWidth}`);
      expect(metrics.scrollWidth).toBeLessThanOrEqual(metrics.innerWidth);
    }
  });

  test('goal details page smoke test', async ({ page }) => {
    await page.goto('/goals/123');

    // Cheap smoke: verify back list button
    await expect(page.getByText('← Goals · list')).toBeVisible();

    // (c) NO horizontal overflow
    const viewports = [1440, 980, 780, 480];
    for (const width of viewports) {
      await page.setViewportSize({ width, height: 800 });
      await page.evaluate(() => {
        return new Promise<void>((resolve) => {
          requestAnimationFrame(() => {
            requestAnimationFrame(() => {
              resolve();
            });
          });
        });
      });
      const metrics = await page.evaluate(() => {
        return {
          scrollWidth: document.documentElement.scrollWidth,
          innerWidth: window.innerWidth,
        };
      });
      console.log(`[Goal Details] Viewport: ${width}, scrollWidth: ${metrics.scrollWidth}, innerWidth: ${metrics.innerWidth}`);
      expect(metrics.scrollWidth).toBeLessThanOrEqual(metrics.innerWidth);
    }
  });
});
