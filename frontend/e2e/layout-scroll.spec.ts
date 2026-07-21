import { test, expect, type Page } from '@playwright/test';
import { installApiRoutes, seedAuth } from './fixtures';
import { openAccount, openRepo, reactFlowZoom, settle, shot } from './harness';

// The headline of the UI refactor: a fixed-viewport shell where the WINDOW/body
// never scrolls — a single <main> (docs pages) or purpose-built inner regions
// (dashboard) own all scrolling. These specs prove both halves: (1) the document
// cannot scroll on any main route at desktop AND mobile, even under wheel +
// keyboard pressure, and (2) the INTENDED inner container scrolls when its
// content overflows.

const DESKTOP = { width: 1440, height: 900 };
const MOBILE = { width: 390, height: 844 };

/** Set up the dashboard's auth + API mocks; docs routes need neither. */
async function prepareDashboard(page: Page, manySessions = false) {
  await seedAuth(page);
  await installApiRoutes(page, { manySessions });
}

/** Read the document scroll state after actively trying to scroll the body via
 *  wheel and keyboard. `scrollingElement` is the root scroller (documentElement
 *  here); with the body clipped it must never exceed its own client height, and
 *  window.scrollY must stay pinned at 0. */
async function assertBodyDoesNotScroll(page: Page) {
  // Actively try to move the page: wheel over the viewport center, then the
  // keyboard scroll keys after focusing the body.
  await page.mouse.move(200, 300);
  await page.mouse.wheel(0, 4000);
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.locator('body').press('End');
  await page.locator('body').press('PageDown');
  await page.locator('body').press('Space');

  const m = await page.evaluate(() => {
    const se = document.scrollingElement ?? document.documentElement;
    return {
      scrollY: window.scrollY,
      scrollX: window.scrollX,
      sh: se.scrollHeight,
      ch: se.clientHeight,
      bodyOverflow: getComputedStyle(document.body).overflowY,
    };
  });
  expect(m.scrollY, 'window.scrollY stays 0').toBe(0);
  expect(m.scrollX, 'window.scrollX stays 0').toBe(0);
  // +1 tolerates sub-pixel rounding of the 100% height chain.
  expect(m.sh, 'document does not overflow its own client height').toBeLessThanOrEqual(m.ch + 1);
  expect(m.bodyOverflow, 'body overflow is clipped').toBe('hidden');
}

/** Find the first genuinely-overflowing scroller at/under `root` (matching a
 *  scrollable overflow-y), scroll it to the end, and report whether scrollTop
 *  actually moved. Restores the original position so screenshots stay stable. */
async function probeInternalScroll(page: Page, rootSelector: string) {
  return page.evaluate((sel) => {
    const root = document.querySelector(sel);
    if (!root) return { found: false as const };
    const nodes = [root, ...root.querySelectorAll('*')] as HTMLElement[];
    for (const el of nodes) {
      const oy = getComputedStyle(el).overflowY;
      if ((oy === 'auto' || oy === 'scroll') && el.scrollHeight > el.clientHeight + 1) {
        const before = el.scrollTop;
        el.scrollTop = el.scrollHeight;
        const moved = el.scrollTop > before;
        el.scrollTop = before;
        return { found: true as const, moved, sh: el.scrollHeight, ch: el.clientHeight };
      }
    }
    return { found: false as const };
  }, rootSelector);
}

test.describe('the window/body never scrolls', () => {
  for (const vp of [DESKTOP, MOBILE]) {
    const size = `${vp.width}x${vp.height}`;

    test(`/ (introduction) does not scroll the document @ ${size}`, async ({ page }) => {
      await page.setViewportSize(vp);
      await page.goto('/');
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
      await assertBodyDoesNotScroll(page);
    });

    test(`/get-started does not scroll the document @ ${size}`, async ({ page }) => {
      await page.setViewportSize(vp);
      await page.goto('/get-started');
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
      await assertBodyDoesNotScroll(page);
    });

    test(`/dashboard does not scroll the document @ ${size}`, async ({ page }) => {
      await page.setViewportSize(vp);
      await prepareDashboard(page);
      await page.goto('/dashboard');
      await expect(page.getByRole('heading', { name: 'Your fkst sessions' })).toBeVisible();
      await settle(page);
      await assertBodyDoesNotScroll(page);
    });
  }
});

test.describe('the intended inner container scrolls', () => {
  test('the v2 landing fits one viewport — nothing scrolls', async ({ page }) => {
    await page.setViewportSize(MOBILE);
    await page.goto('/');
    await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
    // The landing is a single-viewport hero: <main> holds exactly-fitting
    // full-height content, so there is no live overflow scroller anywhere…
    const res = await probeInternalScroll(page, 'main');
    expect(res.found, '<main> has no overflow on the landing').toBe(false);
    // …and the body did not move either.
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
    await shot(page, 'ls-01-intro-single-viewport');
  });

  test('the <main> region scrolls on Get Started', async ({ page }) => {
    await page.setViewportSize(MOBILE);
    await page.goto('/get-started');
    await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
    const res = await probeInternalScroll(page, 'main');
    expect(res.found && res.moved, '<main> scrollTop moves').toBe(true);
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
  });

  test('level-2 session overflow is absorbed by an in-app scroller, never the window', async ({
    page,
  }) => {
    await page.setViewportSize(DESKTOP);
    await prepareDashboard(page, /* manySessions */ true);
    await page.goto('/dashboard');
    await expect(page.getByRole('heading', { name: 'Your fkst sessions' })).toBeVisible();
    await openAccount(page, 'octo-dev');
    await openRepo(page, 'octo-dev', 'web-app');
    await settle(page);

    // The long session list overflows. An in-app region (`<main>` and/or the
    // sidebar panel) must absorb it; the WINDOW must not scroll.
    const main = await probeInternalScroll(page, 'main');
    const aside = await probeInternalScroll(page, 'aside[aria-label="Details panel"]');
    expect(main.found || aside.found, 'an in-app region owns the overflow').toBe(true);
    expect(await page.evaluate(() => window.scrollY), 'window stays pinned at 0').toBe(0);
    await shot(page, 'ls-02-sidebar-overflow');
  });

  // The level-2 sidebar panel is a fixed-height region that scrolls INTERNALLY
  // while <main> stays put: on the app route the shell gives the routed content
  // an h-full wrapper, so the dashboard's h-full chain resolves and the `aside`
  // owns its own overflow instead of growing and pushing <main> to scroll.
  test('the level-2 workspace rail scrolls internally (rail, not <main>)', async ({
    page,
  }) => {
    await page.setViewportSize(DESKTOP);
    await prepareDashboard(page, true);
    await page.goto('/dashboard');
    await openAccount(page, 'octo-dev');
    await openRepo(page, 'octo-dev', 'web-app');
    await settle(page);
    const res = await probeInternalScroll(page, '[data-testid="session-rail"]');
    expect(res.found && res.moved, 'the workspace session rail scrollTop moves').toBe(true);
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
  });

  test('the create-trigger modal body scrolls, page stays put', async ({ page }) => {
    await page.setViewportSize(MOBILE); // the tall form + 85vh cap forces overflow
    await prepareDashboard(page);
    await page.goto('/dashboard');
    await expect(page.getByRole('heading', { name: 'Your fkst sessions' })).toBeVisible();
    await openAccount(page, 'octo-dev');
    await openRepo(page, 'octo-dev', 'web-app');
    await page.getByRole('button', { name: 'New session' }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await settle(page);

    const res = await probeInternalScroll(page, '[role="dialog"]');
    expect(res.found, 'the modal body overflows internally').toBe(true);
    expect(res.found && res.moved, 'the modal body scrollTop moves').toBe(true);
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
    await shot(page, 'ls-03-modal-scroll');
  });

  test('the drawer body scrolls, page stays put', async ({ page }) => {
    await page.setViewportSize(MOBILE);
    await prepareDashboard(page);
    await page.goto('/dashboard');
    await expect(page.getByRole('heading', { name: 'Your fkst sessions' })).toBeVisible();
    // Environments drawer → New environment gives a tall (overflowing) body.
    await page.getByRole('button', { name: 'Environments' }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await dialog.getByRole('button', { name: 'New environment' }).click();
    await expect(dialog.getByText('Install commands')).toBeVisible();
    // Grow the form well past the drawer height so its body must scroll.
    for (let i = 0; i < 10; i++) await dialog.getByRole('button', { name: 'Add command' }).click();
    await settle(page);

    const res = await probeInternalScroll(page, '[role="dialog"]');
    expect(res.found, 'the drawer body overflows internally').toBe(true);
    expect(res.found && res.moved, 'the drawer body scrollTop moves').toBe(true);
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
    await shot(page, 'ls-04-drawer-scroll');
  });

  test('a wheel over the canvas never zooms React Flow and never scrolls the window', async ({
    page,
  }) => {
    await page.setViewportSize(DESKTOP);
    await prepareDashboard(page);
    await page.goto('/dashboard');
    await expect(page.getByRole('heading', { name: 'Your fkst sessions' })).toBeVisible();
    await settle(page);

    const zoomBefore = await reactFlowZoom(page);
    // The canvas must NOT call preventDefault on the wheel (preventScrolling
    // off), so the event is free to reach the page scroller instead of zooming.
    const defaultPrevented = await page.evaluate(() => {
      const rf = document.querySelector('.react-flow');
      if (!rf) return null;
      const ev = new WheelEvent('wheel', { deltaY: 240, cancelable: true, bubbles: true });
      rf.dispatchEvent(ev);
      return ev.defaultPrevented;
    });
    // Also send real trackpad-style wheels over the canvas area.
    await page.mouse.move(300, 480);
    await page.mouse.wheel(0, 600);
    await page.mouse.wheel(0, 600);
    await settle(page);

    const zoomAfter = await reactFlowZoom(page);
    expect(defaultPrevented, 'canvas does not preventDefault the wheel').toBe(false);
    if (zoomBefore != null && zoomAfter != null) {
      expect(zoomAfter, 'React Flow zoom is unchanged by the wheel').toBeCloseTo(zoomBefore, 3);
    }
    expect(await page.evaluate(() => window.scrollY), 'window never scrolls').toBe(0);
  });
});
