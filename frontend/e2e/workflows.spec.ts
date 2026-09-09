import { expect, test, type Page } from '@playwright/test';
import { installApiRoutes, seedAuth, PERSONAL, REPO } from './fixtures';
import { drawerTab, openAccount, openRepo, settle, shot } from './harness';

// End-to-end coverage of a SESSION's scheduled workflows.
//
// They live in the session detail's Workflows tab, not behind a route and not
// beside a repository's sessions: a schedule is assigned to a session creator,
// its run issue is routed to that creator, and the run executes inside that
// session's pod. A repository may host several creators' sessions, so a
// repository-level list mixed schedules that could never run for each other.
//
// `/dashboard` is fixed-viewport, so one assertion here is the kind a unit test
// cannot make: the body never scrolls, and the rail and detail scroll inside
// their own regions. A master/detail tab is exactly the geometry that breaks
// that `h-full` chain when it is wired wrong.

const DESKTOP = { width: 1440, height: 900 };
const MOBILE = { width: 390, height: 844 };

/** Reach the Workflows tab the way an operator does: account → repository →
 *  the session that owns the schedules → its Workflows tab. */
async function openWorkflows(page: Page) {
  await seedAuth(page);
  await installApiRoutes(page);
  await page.goto('/dashboard');
  await openAccount(page, PERSONAL);
  await openRepo(page, PERSONAL, REPO);
  await page.getByTestId('repo-workspace').waitFor();
  await drawerTab(page, 'Workflows').click();
  await page.getByTestId('session-workflows').waitFor();
}

/** The fixed-viewport contract: the window cannot scroll and neither does the
 *  body — the tab's own regions do.
 *
 *  Settled first, deliberately. The workspace's entrance animations translate
 *  elements into place, so a frame captured mid-transition legitimately reports
 *  a taller document; the contract is about the RESTING layout, and measuring
 *  before the animations finish tests the transition instead. */
async function assertNoBodyScroll(page: Page) {
  await settle(page);
  const overflow = await page.evaluate(() => ({
    window: document.documentElement.scrollHeight > window.innerHeight + 1,
    body: document.body.scrollHeight > document.body.clientHeight + 1,
  }));
  expect(overflow.window, 'the window must not scroll on a fixed-viewport route').toBe(false);
  expect(overflow.body, 'the body must not overflow itself').toBe(false);
}

test.describe('a session’s scheduled workflows', () => {
  test('the rail lists only this session’s schedules, and never hides an unrouted one', async ({
    page,
  }) => {
    await page.setViewportSize(DESKTOP);
    await openWorkflows(page);

    // #50 is this session creator's.
    await expect(page.getByTestId('schedule-row-50')).toBeVisible();
    // #51 belongs to another creator — this session can neither run nor operate
    // it, so it is not this session's to show.
    await expect(page.getByTestId('schedule-row-51')).toHaveCount(0);
    // #52 is assigned to nobody, so NO session will run it. It is listed as a
    // link out rather than dropped: the fix is a GitHub assignment, and hiding
    // it would delete a silently-dead schedule from the product.
    await expect(page.getByTestId('unrouted-schedules')).toBeVisible();
    await expect(page.getByTestId('unrouted-row-52')).toBeVisible();
    await shot(page, 'workflows-rail');
  });

  test('the first schedule fills the detail pane, and an in-flight run reports itself', async ({
    page,
  }) => {
    await page.setViewportSize(DESKTOP);
    await openWorkflows(page);

    const detail = page.getByTestId('schedule-detail');
    await expect(detail).toBeVisible();
    await expect(page.getByTestId('upcoming')).toBeVisible();
    await expect(page.getByTestId('arguments')).toContainText('AI Tools Application Engineer');

    // The run is in flight: the runner posts one record at the end, so what can
    // honestly be shown is its age and the issue it is running as.
    const latest = page.getByTestId('latest-run');
    await expect(latest.getByTestId('latest-run-timing')).toContainText('running for');
    await expect(latest.getByTestId('run-issue-link')).toHaveAttribute(
      'href',
      `https://github.com/${PERSONAL}/${REPO}/issues/4300`
    );
    await expect(latest.getByTestId('run-stepper')).toContainText('Awaiting the first step record');
    // Run-now is refused server-side while a run is in flight, so the button
    // says so first instead of inviting a click that always 409s.
    await expect(page.getByTestId('action-run-now')).toBeDisabled();
    await shot(page, 'workflows-in-flight');
  });

  test('an earlier run expands into its per-step outcomes', async ({ page }) => {
    await page.setViewportSize(DESKTOP);
    await openWorkflows(page);

    const history = page.getByTestId('run-history');
    // The newest run is already open above; the history holds the earlier ones.
    await expect(history.getByTestId('run-row-2026-08-05T01:00:00Z')).toHaveCount(0);

    await history.getByTestId('run-row-2026-07-31T01:00:00Z').click();
    await expect(history.getByTestId('step-1')).toContainText('scrape');
    await expect(history.getByTestId('step-2')).toContainText('score');
    await expect(history.getByTestId('step-3')).toContainText('publish');
    await expect(history.getByTestId('step-status-skipped')).toBeVisible();
    await shot(page, 'workflows-run-detail');
  });

  test('a repository has no workflows view of its own', async ({ page }) => {
    await page.setViewportSize(DESKTOP);
    await seedAuth(page);
    await installApiRoutes(page);
    await page.goto('/dashboard');
    await openAccount(page, PERSONAL);
    await openRepo(page, PERSONAL, REPO);

    await expect(page.getByTestId('repo-workspace')).toBeVisible();
    await expect(page.getByTestId('workspace-view-switch')).toHaveCount(0);
    // The only Workflows tab in the tree is the session detail's.
    await expect(page.getByRole('tab', { name: 'Workflows' })).toHaveCount(1);
  });

  for (const [name, viewport] of [
    ['desktop', DESKTOP],
    ['mobile', MOBILE],
  ] as const) {
    test(`${name}: the body never scrolls and the tab scrolls inside its own regions`, async ({
      page,
    }) => {
      await page.setViewportSize(viewport);
      await openWorkflows(page);
      await expect(page.getByTestId('schedule-detail')).toBeVisible();
      await assertNoBodyScroll(page);
    });
  }
});
