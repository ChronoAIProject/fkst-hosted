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
} from './operations-fixtures';

// End-to-end coverage of the `/operations` workspace.
//
// The fixture server enforces the same row-level authorization the backend does
// (see `operations-fixtures.ts`), so every assertion below is about a real
// boundary rather than about what the test decided to hand the page. The suite
// runs at desktop and at a narrow mobile width, because "no overlap, no body
// scroll, nothing clipped" is a claim that only means anything at both.

const DESKTOP = { width: 1440, height: 900 };
const MOBILE = { width: 390, height: 844 };

/** Open `/operations` as one viewer with the fixture server installed. */
async function open(page: Page, opts: RouteOptions, search = '') {
  await seedOperationsAuth(page);
  await installOperationsRoutes(page, opts);
  await page.goto(`/operations${search}`);
  await expect(page.getByRole('tablist', { name: 'Operations views' })).toBeVisible();
}

const activityRows = (page: Page) => page.getByTestId('activity-row');
const sandboxRows = (page: Page) => page.getByTestId('sandbox-row');

/**
 * Prove the fixed-viewport contract at whatever viewport is current: the window
 * cannot scroll, the body does not overflow itself, AND the shell's single
 * `<main>` scroll region does not overflow either.
 *
 * The last check is the load-bearing one for this route. `<main>` scrolls on
 * doc pages by design, so a workspace whose toolbar and panels grew past it
 * would still leave `window.scrollY` at 0 while being visibly broken — the
 * caller would have to scroll a region that is supposed to hold a fixed-height
 * layout. Asserting `main` fits is what actually catches that.
 */
async function assertNoBodyScroll(page: Page) {
  await page.mouse.move(400, 400);
  await page.mouse.wheel(0, 4000);
  const metrics = await page.evaluate(() => {
    const main = document.querySelector('main');
    return {
      scrollY: window.scrollY,
      bodyOverflow: getComputedStyle(document.body).overflowY,
      bodyScrollHeight: document.body.scrollHeight,
      bodyClientHeight: document.body.clientHeight,
      mainScrollHeight: main?.scrollHeight ?? 0,
      mainClientHeight: main?.clientHeight ?? 0,
    };
  });
  expect(metrics.scrollY).toBe(0);
  expect(metrics.bodyOverflow).toBe('hidden');
  expect(metrics.bodyScrollHeight).toBeLessThanOrEqual(metrics.bodyClientHeight + 1);
  expect(metrics.mainScrollHeight).toBeLessThanOrEqual(metrics.mainClientHeight + 1);
}

test.describe('discovery and direct navigation', () => {
  test('every authenticated user finds Operations in the nav and can open it', async ({ page }) => {
    await seedOperationsAuth(page);
    await installOperationsRoutes(page, { viewer: ERIN });
    await page.goto('/');
    const link = page.getByRole('link', { name: 'Operations' });
    await expect(link).toBeVisible();
    await link.click();
    // Erin is an ordinary user with no accessible sessions, but she still has
    // her OWN request history: she gets data, never a denied page.
    await expect(page.getByTestId('operations-scope')).toHaveText('My activity');
    await expect(activityRows(page)).toHaveCount(1);
    await expect(activityRows(page).getByText('@erin')).toBeVisible();
    await expect(activityRows(page).getByText('@alice')).toHaveCount(0);
  });

  test('a direct link restores the exact tab, scope, and filters', async ({ page }) => {
    await open(page, { viewer: ALICE }, '?tab=sandboxes&scope=accessible&status=running');
    await expect(page.getByTestId('operations-scope')).toHaveText('My accessible sandboxes');
    await expect(page.getByLabel('Status')).toHaveValue('running');
    await expect(sandboxRows(page)).toHaveCount(1);
  });
});

test.describe('cross-user isolation', () => {
  test('Alice sees only her own request rows', async ({ page }) => {
    await open(page, { viewer: ALICE });
    await expect(activityRows(page)).toHaveCount(1);
    await expect(activityRows(page).getByText('@alice')).toBeVisible();
    await expect(activityRows(page).getByText('@bob')).toHaveCount(0);
    await expect(activityRows(page).getByText('@erin')).toHaveCount(0);
    await shot(page, 'operations-activity-alice');
  });

  test('Bob cannot see Alice’s rows even though they share one session', async ({ page }) => {
    await open(page, { viewer: BOB });
    await expect(activityRows(page)).toHaveCount(1);
    await expect(activityRows(page).getByText('@bob')).toBeVisible();
    await expect(activityRows(page).getByText('@alice')).toHaveCount(0);
  });

  test('both collaborators see the shared sandbox; unrelated Erin does not', async ({ page }) => {
    await open(page, { viewer: ALICE }, '?tab=sandboxes&scope=accessible');
    await expect(sandboxRows(page)).toHaveCount(1);
    await expect(sandboxRows(page).getByTitle('sess-shared', { exact: true })).toBeVisible();

    await open(page, { viewer: BOB }, '?tab=sandboxes&scope=accessible');
    await expect(sandboxRows(page)).toHaveCount(1);

    await open(page, { viewer: ERIN }, '?tab=sandboxes&scope=accessible');
    await expect(sandboxRows(page)).toHaveCount(0);
    await expect(page.getByTestId('operations-empty')).toBeVisible();
    await shot(page, 'operations-sandboxes-empty');
  });
});

test.describe('a crafted URL cannot widen anything', () => {
  test('a regular user asking for the global scope is normalized, never shown a row', async ({
    page,
  }) => {
    await open(page, { viewer: BOB }, '?tab=activity&scope=all&actor_id=101');
    // Normalized to the allowed scope, with the cross-actor filter dropped.
    await expect.poll(() => new URL(page.url()).searchParams.get('scope')).toBe('mine');
    expect(new URL(page.url()).searchParams.has('actor_id')).toBe(false);
    await expect(page.getByTestId('operations-scope-reset')).toBeVisible();
    // Not one global fixture row was ever painted.
    await expect(activityRows(page).getByText('@alice')).toHaveCount(0);
    await expect(activityRows(page).getByText('Anonymous')).toHaveCount(0);
    await expect(page.getByTestId('operations-scope-control')).toHaveCount(0);
    await shot(page, 'operations-scope-denied');
  });

  test('the same holds for the sandbox view', async ({ page }) => {
    await open(page, { viewer: ERIN }, '?tab=sandboxes&scope=all');
    await expect.poll(() => new URL(page.url()).searchParams.get('scope')).toBe('accessible');
    await expect(sandboxRows(page)).toHaveCount(0);
    await expect(page.getByTitle('osb-orphan-1')).toHaveCount(0);
  });

  test('an exact but unauthorized session id is indistinguishable from a missing one', async ({
    page,
  }) => {
    await open(page, { viewer: ERIN }, '?tab=sandboxes&scope=accessible&session_id=sess-shared');
    await expect(page.getByTestId('operations-error')).toContainText('No such session.');
  });
});

test.describe('global administrator', () => {
  test('defaults to the all scope and sees every actor plus the unattributed', async ({ page }) => {
    await open(page, { viewer: GRACE });
    await expect.poll(() => new URL(page.url()).searchParams.get('scope')).toBe('all');
    await expect(page.getByTestId('operations-scope')).toHaveText('All activity');
    // Alice's, Bob's, Erin's, plus the unattributed record no regular user may
    // ever be shown.
    await expect(activityRows(page)).toHaveCount(4);
    await expect(activityRows(page).getByText('@alice')).toBeVisible();
    await expect(activityRows(page).getByText('@bob')).toBeVisible();
    await expect(activityRows(page).getByText('Anonymous')).toBeVisible();
    await expect(page.getByLabel('Actor id')).toBeVisible();
    await shot(page, 'operations-activity-global');
  });

  test('can switch to the personal scope, which clears the actor filters', async ({ page }) => {
    await open(page, { viewer: GRACE }, '?tab=activity&scope=all&actor_id=101');
    await expect(page.getByTestId('operations-scope-control')).toBeVisible();
    await page.getByRole('tab', { name: 'Mine' }).click();
    await expect.poll(() => new URL(page.url()).searchParams.get('scope')).toBe('mine');
    expect(new URL(page.url()).searchParams.has('actor_id')).toBe(false);
    await expect(page.getByLabel('Actor id')).toHaveCount(0);
  });

  test('inspects orphan, legacy, and conflicted runtimes', async ({ page }) => {
    await open(page, { viewer: GRACE }, '?tab=sandboxes&scope=all');
    await expect(sandboxRows(page)).toHaveCount(3);
    await expect(sandboxRows(page).getByText('Unknown (legacy runtime)')).toBeVisible();
    // A null lifetime is Unlimited, and a null restart count is Not reported —
    // never `0s` and never `0`.
    await expect(sandboxRows(page).getByText('Unlimited')).toBeVisible();
    await expect(sandboxRows(page).getByText('Not reported')).toBeVisible();
    await expect(sandboxRows(page).getByText('Attribution conflict')).toBeVisible();
    await shot(page, 'operations-sandboxes-global');
  });
});

test.describe('sandbox to activity cross-link', () => {
  test('shows the viewer’s own calls plus lifecycle, and no collaborator calls', async ({
    page,
  }) => {
    await open(page, { viewer: ALICE }, '?tab=sandboxes&scope=accessible');
    await sandboxRows(page).first().click();
    await page.getByRole('button', { name: 'View activity for this session' }).click();

    await expect.poll(() => new URL(page.url()).searchParams.get('tab')).toBe('activity');
    expect(new URL(page.url()).searchParams.get('record_kind')).toBe('all');
    expect(new URL(page.url()).searchParams.get('session_id')).toBe('sess-shared');
    // Alice's own API row plus the system lifecycle row — and nothing of Bob's.
    await expect(activityRows(page)).toHaveCount(2);
    await expect(activityRows(page).getByText('Lifecycle')).toBeVisible();
    await expect(activityRows(page).getByText('@alice')).toBeVisible();
    await expect(activityRows(page).getByText('@bob')).toHaveCount(0);
    await shot(page, 'operations-session-timeline');
  });
});

test.describe('independent failure states', () => {
  test('an analytics outage cannot remove the live sandbox table', async ({ page }) => {
    await open(page, { viewer: ALICE, activityUnavailable: true });
    await expect(page.getByTestId('operations-error')).toContainText('no activity query configured');
    await shot(page, 'operations-activity-error');

    await page.getByRole('tab', { name: 'Sandboxes' }).click();
    await expect(sandboxRows(page)).toHaveCount(1);
    await expect(page.getByTestId('operations-error')).toHaveCount(0);
  });

  test('a runtime outage cannot falsify activity', async ({ page }) => {
    await open(page, { viewer: ALICE, runtimeUnavailable: true }, '?tab=sandboxes&scope=accessible');
    await expect(page.getByTestId('operations-error')).toBeVisible();
    await page.getByRole('tab', { name: 'Activity' }).click();
    await expect(activityRows(page)).toHaveCount(1);
  });

  test('a cold session-visibility projection is not an empty fleet', async ({ page }) => {
    await open(page, { viewer: ALICE, registryCold: true }, '?tab=sandboxes&scope=accessible');
    await expect(page.getByTestId('operations-error')).toContainText(
      'Session visibility is still recovering'
    );
    await expect(page.getByTestId('operations-empty')).toHaveCount(0);
  });

  test('a partial page is visibly distinct from a complete empty one', async ({ page }) => {
    await open(page, { viewer: ALICE, activityPartial: true });
    await expect(page.getByTestId('activity-partial')).toContainText(
      'The analytics source could not answer'
    );
    await shot(page, 'operations-activity-partial');

    await open(page, { viewer: ALICE, empty: true });
    await expect(page.getByTestId('operations-empty')).toBeVisible();
    await expect(page.getByTestId('activity-partial')).toHaveCount(0);
    await shot(page, 'operations-activity-empty');
  });

  test('an incomplete page with no rows never claims a complete empty result', async ({ page }) => {
    await open(page, { viewer: ALICE, activityPartial: true, empty: true });
    await expect(page.getByTestId('operations-incomplete')).toContainText('This page is incomplete');
    // The one thing that must NOT be on screen: "no records match".
    await expect(page.getByTestId('operations-empty')).toHaveCount(0);
    await shot(page, 'operations-activity-incomplete');
  });

  test('a snapshot older than the staleness bound is marked, and keeps its rows', async ({
    page,
  }) => {
    await open(
      page,
      { viewer: ALICE, runtimeObservedSecondsAgo: 40 },
      '?tab=sandboxes&scope=accessible'
    );
    await expect(page.getByTestId('sandbox-stale')).toBeVisible();
    // Last-good rows survive: they are still what was last observed.
    await expect(sandboxRows(page)).toHaveCount(1);
    await shot(page, 'operations-sandboxes-stale');
  });
});

test.describe('polling', () => {
  test('pauses while the tab is hidden and resumes on return', async ({ page }) => {
    let calls = 0;
    await seedOperationsAuth(page);
    await installOperationsRoutes(page, { viewer: ALICE });
    await page.route('**/api/v1/operations/sandboxes*', async (route) => {
      calls += 1;
      await route.fallback();
    });
    await page.goto('/operations?tab=sandboxes&scope=accessible');
    await expect(sandboxRows(page)).toHaveCount(1);

    const beforeHidden = calls;
    await page.evaluate(() => {
      Object.defineProperty(document, 'hidden', { configurable: true, value: true });
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await page.waitForTimeout(12_000);
    expect(calls).toBe(beforeHidden);

    await page.evaluate(() => {
      Object.defineProperty(document, 'hidden', { configurable: true, value: false });
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await expect.poll(() => calls).toBeGreaterThan(beforeHidden);
  });
});

test.describe('layout, at both widths', () => {
  for (const [name, viewport] of [
    ['desktop', DESKTOP],
    ['mobile', MOBILE],
  ] as const) {
    test(`${name}: the body never scrolls and the table scrolls inside its own region`, async ({
      page,
    }) => {
      await page.setViewportSize(viewport);
      await open(page, { viewer: GRACE }, '?tab=sandboxes&scope=all');
      await expect(sandboxRows(page)).toHaveCount(3);
      await settle(page);
      await assertNoBodyScroll(page);

      // The wide table lives in its own horizontal scroller.
      const overflows = await page.evaluate(() => {
        const table = document.querySelector('table');
        const scroller = table?.parentElement;
        if (!scroller) return null;
        return {
          canScrollX: scroller.scrollWidth > scroller.clientWidth,
          overflow: getComputedStyle(scroller).overflow,
        };
      });
      expect(overflows).not.toBeNull();
      expect(overflows!.overflow).toContain('auto');
      expect(overflows!.canScrollX).toBe(true);

      await shot(page, `operations-layout-${name}`);
    });

    test(`${name}: the details panel opens without covering the controls`, async ({ page }) => {
      await page.setViewportSize(viewport);
      await open(page, { viewer: ALICE }, '?tab=sandboxes&scope=accessible');
      await sandboxRows(page).first().click();
      const details = page.getByTestId('operations-details');
      await expect(details).toBeVisible();
      await settle(page);
      await assertNoBodyScroll(page);

      // The tablist stays reachable and un-obscured beside/above the panel.
      const tabs = page.getByRole('tablist', { name: 'Operations views' });
      await expect(tabs).toBeVisible();
      const [tabsBox, detailsBox] = await Promise.all([tabs.boundingBox(), details.boundingBox()]);
      expect(tabsBox).not.toBeNull();
      expect(detailsBox).not.toBeNull();
      const overlaps =
        tabsBox!.x < detailsBox!.x + detailsBox!.width &&
        tabsBox!.x + tabsBox!.width > detailsBox!.x &&
        tabsBox!.y < detailsBox!.y + detailsBox!.height &&
        tabsBox!.y + tabsBox!.height > detailsBox!.y;
      expect(overlaps).toBe(false);

      await shot(page, `operations-details-${name}`);
    });
  }
});

test.describe('the browser talks to nothing but this backend', () => {
  test('never calls PostHog, the relay, Kubernetes, or OpenSandbox, and ships no secret', async ({
    page,
  }) => {
    const requested: string[] = [];
    page.on('request', (request) => requested.push(request.url()));
    await open(page, { viewer: GRACE });
    await page.getByRole('tab', { name: 'Sandboxes' }).click();
    await expect(sandboxRows(page)).toHaveCount(3);

    const external = requested.filter((url) => !url.startsWith('http://localhost'));
    expect(external, `unexpected external requests: ${external.join(', ')}`).toHaveLength(0);
    for (const url of requested) {
      expect(url).not.toMatch(/posthog|\/i\/v0\/e|batch|8080\/relay|kubernetes|opensandbox/i);
    }

    // Nothing resembling a project key, query credential, or relay token is in
    // the served bundle.
    const scripts = await page.evaluate(() =>
      Array.from(document.querySelectorAll('script[src]')).map((s) => (s as HTMLScriptElement).src)
    );
    for (const src of scripts) {
      const body = await (await page.request.get(src)).text();
      expect(body).not.toMatch(/phc_[A-Za-z0-9]/);
      expect(body).not.toMatch(/POSTHOG_(PROJECT_)?API_KEY/);
      expect(body).not.toMatch(/FKST_AUDIT_RELAY_TOKEN/);
    }
  });
});
