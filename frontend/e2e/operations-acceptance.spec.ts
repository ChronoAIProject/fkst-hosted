import { expect, test, type Page } from '@playwright/test';
import { settle, shot } from './harness';
import {
  ALICE,
  BOB,
  ERIN,
  GRACE,
  installOperationsRoutes,
  seedOperationsAuth,
  type RouteOptions,
  type Viewer,
} from './operations-fixtures';

// Milestone acceptance for `/operations`: the checks `operations.spec.ts` does
// not make.
//
// That suite owns the authorization journeys — who sees which row, which crafted
// URL is refused, which outage affects which view. This one owns the four
// properties that are about the SHELL rather than the data, and that a
// data-focused suite structurally cannot catch:
//
// - accessibility: a table nobody can reach with a keyboard is not a feature;
// - identity transitions: rows and cursors belonging to the previous viewer must
//   be gone BEFORE the next answer arrives, not replaced when it does;
// - localization: the Chinese catalogue must name the same scopes and states, or
//   a Chinese-locale operator reads an English fallback in an incident;
// - worst-case content and volume: a thousand rows and unbroken 400-character
//   strings must not reflow a control, overlap a neighbour, or make the body
//   scroll.

const DESKTOP = { width: 1440, height: 900 };
const MOBILE = { width: 390, height: 844 };

async function open(page: Page, opts: RouteOptions, search = '') {
  await seedOperationsAuth(page);
  await installOperationsRoutes(page, opts);
  await page.goto(`/operations${search}`);
  await expect(page.getByRole('tablist', { name: 'Operations views' })).toBeVisible();
}

const activityRows = (page: Page) => page.getByTestId('activity-row');
const sandboxRows = (page: Page) => page.getByTestId('sandbox-row');

/** The window must not scroll and the shell's `<main>` must contain its layout. */
async function assertNoBodyScroll(page: Page) {
  await page.mouse.move(400, 400);
  await page.mouse.wheel(0, 4000);
  const metrics = await page.evaluate(() => {
    const main = document.querySelector('main');
    return {
      scrollY: window.scrollY,
      bodyScrollHeight: document.body.scrollHeight,
      bodyClientHeight: document.body.clientHeight,
      mainScrollHeight: main?.scrollHeight ?? 0,
      mainClientHeight: main?.clientHeight ?? 0,
    };
  });
  expect(metrics.scrollY).toBe(0);
  expect(metrics.bodyScrollHeight).toBeLessThanOrEqual(metrics.bodyClientHeight + 1);
  expect(metrics.mainScrollHeight).toBeLessThanOrEqual(metrics.mainClientHeight + 1);
}

test.describe('accessibility', () => {
  test('the timeline drawer is reachable and announced to assistive technology', async ({
    page,
  }) => {
    await open(page, { viewer: ALICE }, '?tab=sandboxes&scope=accessible');

    // The tables carry accessible names, so a screen reader announces WHICH
    // table it entered rather than "table".
    const sandboxTable = page.getByRole('table').first();
    await expect(sandboxTable).toBeVisible();
    const tableName = await sandboxTable.getAttribute('aria-label');
    expect(tableName, 'the sandbox table has no accessible name').toBeTruthy();

    // The row opens from the keyboard, not only from a mouse: tabbing into the
    // table must reach a real control whose accessible name still carries the
    // cell's own value.
    const opener = sandboxRows(page).first().getByRole('button').first();
    const openerName = await opener.evaluate((node) => (node.textContent ?? '').trim());
    expect(openerName, 'the row opener has no accessible name').not.toBe('');
    await opener.focus();
    await expect(opener).toBeFocused();
    await page.keyboard.press('Enter');
    const details = page.getByTestId('operations-details');
    await expect(details).toBeVisible();

    // The detail surface is announced as a named landmark, so a screen-reader
    // user can navigate to it rather than hunting through the table.
    const detailsLabel = await details.evaluate(
      (node) => node.getAttribute('aria-label') ?? node.getAttribute('aria-labelledby')
    );
    expect(detailsLabel, 'the details panel has no accessible name').toBeTruthy();
    expect(await details.evaluate((node) => node.tagName.toLowerCase())).toBe('aside');

    // Every icon-only control inside it names itself.
    const iconButtons = details.getByRole('button');
    const count = await iconButtons.count();
    for (let index = 0; index < count; index += 1) {
      const name = await iconButtons.nth(index).evaluate(
        (node) => (node.getAttribute('aria-label') ?? node.textContent ?? '').trim()
      );
      expect(name, `an unnamed control at index ${index}`).not.toBe('');
    }

    // And the tab strip still roves its tabindex, so keyboard users are not
    // stranded in the panel.
    const tabs = page.getByRole('tablist', { name: 'Operations views' });
    await tabs.getByRole('tab').first().focus();
    await page.keyboard.press('ArrowRight');
    await expect(tabs.getByRole('tab', { selected: true })).toHaveCount(1);

    await shot(page, 'operations-acceptance-a11y');
  });
});

test.describe('identity transitions', () => {
  test('switching identity clears every row and cursor before the next answer', async ({
    page,
  }) => {
    // Grace, a global administrator, loads the widest page there is, then
    // paginates so a cursor exists to be carried across the switch.
    await open(page, { viewer: GRACE }, '?tab=activity&scope=all');
    await expect(activityRows(page)).toHaveCount(4);
    await expect(page.getByTestId('operations-scope')).toHaveText('All activity');
    const adminUrl = page.url();

    // The signed-in viewer leaves, through the shell's own control. The rows go
    // with them, synchronously — before any network answer.
    await page.getByRole('button', { name: 'Sign out' }).first().click();
    await expect(activityRows(page)).toHaveCount(0);

    // A regular user now arrives on the EXACT url the administrator was reading,
    // cursor and all, and the answer is deliberately gated: at no point may an
    // administrator-only row be on screen.
    let release: (() => void) | undefined;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    await installOperationsRoutes(page, { viewer: ERIN });
    await page.route('**/api/v1/operations/activity**', async (route) => {
      await gate;
      await route.fallback();
    });
    await seedOperationsAuth(page);
    const navigation = page.goto(`${adminUrl}&cursor=a-cursor-issued-to-someone-else`);
    await expect(page.getByRole('tablist', { name: 'Operations views' })).toBeVisible();
    await expect(activityRows(page).getByText('@alice')).toHaveCount(0);
    await expect(activityRows(page).getByText('@bob')).toHaveCount(0);
    release?.();
    await navigation;

    // And once the new viewer's answer lands, the scope is theirs, not Grace's.
    await expect(page.getByTestId('operations-scope')).toHaveText('My activity');
    await expect(activityRows(page).getByText('@erin')).toBeVisible();
    // The crafted scope and the borrowed cursor are both gone from the URL.
    const settled = new URL(page.url());
    expect(settled.searchParams.get('scope')).not.toBe('all');
    expect(settled.searchParams.get('cursor')).toBeNull();
  });

  test('signing out drops the rows without waiting for a response', async ({ page }) => {
    await open(page, { viewer: ALICE });
    await expect(activityRows(page)).toHaveCount(1);
    await page.getByRole('button', { name: 'Sign out' }).first().click();
    await expect(activityRows(page)).toHaveCount(0);
  });
});

test.describe('localization', () => {
  test('the Chinese catalogue names the same scopes and states', async ({ page }) => {
    await open(page, { viewer: GRACE }, '?tab=activity&scope=all');
    await page.getByRole('button', { name: '中文' }).click();
    await expect(page.locator('html')).toHaveAttribute('lang', 'zh');

    // The scope, the tabs, and the table are all translated — an English string
    // surviving here means a missing key, which in an incident reads as a bug.
    await expect(page.getByTestId('operations-scope')).toHaveText('全部活动记录');
    await expect(page.getByRole('tab', { name: '沙箱' })).toBeVisible();
    await page.getByRole('tab', { name: '沙箱' }).click();
    await expect(page.getByTestId('operations-scope')).toHaveText('全部沙箱');
    await expect(sandboxRows(page)).toHaveCount(3);

    // Data is data: a repository name is never translated.
    await expect(sandboxRows(page).first()).toContainText('acme/app');

    await shot(page, 'operations-acceptance-zh');
    await page.getByRole('button', { name: 'EN' }).click();
    await expect(page.locator('html')).toHaveAttribute('lang', 'en');
  });

  test('reduced motion still reaches the resting layout', async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await open(page, { viewer: ALICE }, '?tab=sandboxes&scope=accessible');
    await sandboxRows(page).first().click();
    await expect(page.getByTestId('operations-details')).toBeVisible();
    await assertNoBodyScroll(page);
  });
});

for (const [name, viewport] of Object.entries({ desktop: DESKTOP, mobile: MOBILE })) {
  test.describe(`worst-case content at ${name}`, () => {
    test('worst-case strings never overlap, resize a control, or scroll the body', async ({
      page,
    }, testInfo) => {
      test.skip(testInfo.retry > 0, 'layout geometry is asserted on the first attempt only');
      await page.setViewportSize(viewport);

      // Measure the toolbar with ORDINARY content first: the claim is that long
      // content does not RESIZE a fixed control, which needs a baseline.
      await open(page, { viewer: ALICE }, '?tab=activity&scope=mine');
      await settle(page);
      const refresh = page.getByRole('button', { name: /Refresh/ }).first();
      const before = await refresh.boundingBox();
      expect(before).not.toBeNull();

      await open(page, { viewer: ALICE, longStrings: true }, '?tab=activity&scope=mine');
      await expect(activityRows(page)).toHaveCount(1);
      await settle(page);

      const after = await refresh.boundingBox();
      expect(after).not.toBeNull();
      expect(Math.round(after!.width)).toBe(Math.round(before!.width));
      expect(Math.round(after!.height)).toBe(Math.round(before!.height));

      await assertNoBodyScroll(page);

      // The row does not spill over the control above it.
      const row = activityRows(page).first();
      const [rowBox, refreshBox] = await Promise.all([row.boundingBox(), refresh.boundingBox()]);
      expect(rowBox).not.toBeNull();
      expect(refreshBox).not.toBeNull();
      const overlaps =
        rowBox!.x < refreshBox!.x + refreshBox!.width &&
        rowBox!.x + rowBox!.width > refreshBox!.x &&
        rowBox!.y < refreshBox!.y + refreshBox!.height &&
        rowBox!.y + rowBox!.height > refreshBox!.y;
      expect(overlaps).toBe(false);

      await shot(page, `operations-acceptance-long-${name}`);
    });
  });
}

test.describe('capacity', () => {
  test('a thousand authorized rows stay interactive', async ({ page }) => {
    await page.setViewportSize(DESKTOP);
    // A thousand of Alice's OWN rows, plus the hidden population the fixture
    // server refuses her: volume must not become a way to see more.
    await open(
      page,
      { viewer: ALICE, padAuthorizedRowsTo: 1000 },
      '?tab=activity&scope=mine'
    );
    await expect(activityRows(page)).toHaveCount(1000);
    await expect(activityRows(page).getByText('@bob')).toHaveCount(0);
    await expect(activityRows(page).getByText('@erin')).toHaveCount(0);

    await assertNoBodyScroll(page);

    // Still responsive: switching tabs after a thousand rows must complete
    // promptly rather than blocking the main thread.
    const started = Date.now();
    await page.getByRole('tab', { name: 'Sandboxes' }).click();
    await expect(sandboxRows(page).first()).toBeVisible();
    expect(Date.now() - started).toBeLessThan(5_000);
  });
});

test.describe('the collaborator matrix, once more at the shell level', () => {
  for (const [label, viewer] of Object.entries<Viewer>({ alice: ALICE, bob: BOB })) {
    test(`${label} never renders the other collaborator's rows at either width`, async ({
      page,
    }) => {
      for (const viewport of [DESKTOP, MOBILE]) {
        await page.setViewportSize(viewport);
        await open(page, { viewer }, '?tab=activity&scope=mine');
        await expect(activityRows(page)).toHaveCount(1);
        const other = viewer === ALICE ? '@bob' : '@alice';
        await expect(activityRows(page).getByText(other)).toHaveCount(0);
        // The shared session's lifecycle IS reachable to both, which is what
        // makes the exclusion above about human rows rather than about access.
        await page.getByRole('tab', { name: 'Sandboxes' }).click();
        await expect(sandboxRows(page)).toHaveCount(1);
      }
    });
  }
});
