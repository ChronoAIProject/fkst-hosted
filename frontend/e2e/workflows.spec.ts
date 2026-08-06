import { expect, test, type Page } from '@playwright/test';
import { shot } from './harness';

// End-to-end coverage of a repository's scheduled workflows.
//
// They live inside the repository workspace rather than behind a route of their
// own: a schedule is a repository's issue, so the repository is the context you
// are already in, not a parameter to re-choose.
//
// `/dashboard` is fixed-viewport, so the load-bearing assertion here is the one
// a unit test cannot make: the body never scrolls, and the schedule table
// scrolls inside its own region. Nesting the workflows body under the view
// switch must not break that `h-full` chain.

const DESKTOP = { width: 1440, height: 900 };
const MOBILE = { width: 390, height: 844 };

const SUMMARY = {
  scheduleIssue: 50,
  title: 'nightly sourcing',
  htmlUrl: 'https://github.com/acme/site/issues/50',
  workflowId: 'github-candidate-sourcing',
  runMode: 'cron: 0 1 * * 1-5',
  cadence: 'weekdays at 01:00 UTC',
  state: 'running',
  nextDue: '2099-01-01T01:00:00Z',
  lastRun: {
    slot: '2026-07-31T01:00:00Z',
    manual: false,
    status: 'ok',
    startedAt: '2026-07-31T01:00:00Z',
    endedAt: '2026-07-31T01:12:00Z',
    durationS: 720,
    issue: 4242,
    detail: null,
  },
  successRate30d: 0.75,
  invalidDetail: null,
};

const PAUSED = {
  ...SUMMARY,
  scheduleIssue: 51,
  workflowId: 'weekly-digest',
  state: 'paused',
  cadence: 'every Monday at 09:00 UTC',
};

const INVALID = {
  ...SUMMARY,
  scheduleIssue: 52,
  workflowId: '',
  title: 'broken schedule',
  state: 'invalid',
  cadence: '',
  nextDue: null,
  invalidDetail: 'missing required section `### Run Mode`',
};

/** Serve the schedules surface, and FAIL any other request: the browser must
 *  never talk to GitHub directly from this route. */
async function installRoutes(page: Page) {
  await page.route('**/api/v1/repos/acme/site/schedules', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        owner: 'acme',
        name: 'site',
        installed: true,
        schedules: [SUMMARY, PAUSED, INVALID],
      }),
    });
  });
  await page.route('**/api/v1/repos/acme/site/schedules/50', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        summary: SUMMARY,
        upcoming: ['2099-01-01T01:00:00Z', '2099-01-02T01:00:00Z'],
        arguments: { role: 'AI Tools Application Engineer', min_score: '6' },
        runs: [
          {
            slot: '2026-07-31T01:00:00Z',
            manual: false,
            status: 'failed',
            startedAt: '2026-07-31T01:00:00Z',
            endedAt: '2026-07-31T01:03:00Z',
            durationS: 180,
            issue: 4242,
            detail: 'step 2 returned no parseable payload',
          },
        ],
      }),
    });
  });
  await page.route('**/api/v1/repos/acme/site/schedules/50/runs/**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        run: {
          slot: '2026-07-31T01:00:00Z',
          manual: false,
          status: 'failed',
          startedAt: '2026-07-31T01:00:00Z',
          endedAt: '2026-07-31T01:03:00Z',
          durationS: 180,
          issue: 4242,
          detail: 'step 2 returned no parseable payload',
        },
        steps: [
          { index: 1, id: 'scrape', status: 'ok', durationS: 41 },
          { index: 2, id: 'score', status: 'failed', durationS: 9 },
          { index: 3, id: 'publish', status: 'skipped', durationS: null },
        ],
        runIssue: 4242,
      }),
    });
  });
  await page.route('https://api.github.com/**', async (route) => {
    await route.abort('failed');
  });
}

async function open(page: Page, search = '') {
  await page.addInitScript(() => {
    window.localStorage.setItem('fkst-gh-access', 'e2e-fake-access-token');
  });
  await installRoutes(page);
  // Reach schedules the way an operator does: open the repository, then switch
  // its workspace to Workflows. A direct URL would not exercise the switch, and
  // the switch is now the only way in.
  await page.goto(`/dashboard?owner=acme&repo=site${search}`);
  await page.getByTestId('repo-workspace').waitFor();
  await page.getByRole('tab', { name: 'Workflows' }).click();
  await page.getByTestId('repo-workflows').waitFor();
}

/** The fixed-viewport contract: the window cannot scroll and neither does the
 *  body — the list scrolls inside its own region. */
async function assertNoBodyScroll(page: Page) {
  const overflow = await page.evaluate(() => ({
    window: document.documentElement.scrollHeight > window.innerHeight + 1,
    body: document.body.scrollHeight > document.body.clientHeight + 1,
  }));
  expect(overflow.window, 'the window must not scroll on a fixed-viewport route').toBe(false);
  expect(overflow.body, 'the body must not overflow itself').toBe(false);
}

test.describe('scheduled workflows', () => {
  test('lists a repository’s schedules with each lifecycle visible', async ({ page }) => {
    await page.setViewportSize(DESKTOP);
    await open(page);
    await expect(page.getByTestId('schedule-list')).toBeVisible();
    await expect(page.getByText('github-candidate-sourcing')).toBeVisible();
    await expect(page.getByTestId('lifecycle-running')).toBeVisible();
    await expect(page.getByTestId('lifecycle-paused')).toBeVisible();
    await expect(page.getByTestId('lifecycle-invalid')).toBeVisible();
    // The broken one explains itself in the list rather than hiding the reason
    // behind a click — the whole point of surfacing invalid inline.
    await expect(page.getByTestId('invalid-detail-52')).toContainText('### Run Mode');
    await shot(page, 'workflows-list');
  });

  test('opens a schedule and expands one run into its per-step outcomes', async ({ page }) => {
    await page.setViewportSize(DESKTOP);
    await open(page);
    await page.getByText('github-candidate-sourcing').click();
    await expect(page.getByTestId('schedule-detail')).toBeVisible();
    await expect(page.getByTestId('upcoming')).toBeVisible();
    await expect(page.getByTestId('arguments')).toContainText('AI Tools Application Engineer');
    // Run-now is disabled while a run is in flight: the server answers 409
    // either way, so saying so first beats inviting a click that always fails.
    await expect(page.getByTestId('action-run-now')).toBeDisabled();

    await page.getByTestId('run-row-2026-07-31T01:00:00Z').click();
    await expect(page.getByTestId('run-stepper')).toBeVisible();
    await expect(page.getByTestId('step-1')).toContainText('scrape');
    await expect(page.getByTestId('step-2')).toContainText('score');
    await expect(page.getByTestId('step-3')).toContainText('publish');
    await expect(page.getByTestId('step-status-skipped')).toBeVisible();
    await shot(page, 'workflows-run-detail');
  });

  test('the whole view is a shareable URL', async ({ page }) => {
    await page.setViewportSize(DESKTOP);
    await open(page, '&schedule=50&run=2026-07-31T01:00:00Z');
    // Deep-linked straight into one run's steps, with no clicks at all.
    await expect(page.getByTestId('run-stepper')).toBeVisible();
    await expect(page.getByTestId('step-2')).toContainText('score');
  });

  for (const [name, viewport] of [
    ['desktop', DESKTOP],
    ['mobile', MOBILE],
  ] as const) {
    test(`${name}: the body never scrolls and the list scrolls inside its own region`, async ({
      page,
    }) => {
      await page.setViewportSize(viewport);
      await open(page);
      await expect(page.getByTestId('schedule-list')).toBeVisible();
      await assertNoBodyScroll(page);
    });
  }
});
