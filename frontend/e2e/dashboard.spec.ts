import { test, expect } from '@playwright/test';
import { installApiRoutes, seedAuth, PERSONAL, ORG, REPO } from './fixtures';
import { drawerTab, openAccount, openRepo, settle, shot, sidebar } from './harness';

// The dashboard's full UI journey against the refactored, fixed-viewport shell:
// levels 0→1→2, the slide-in session drawer with its four tabs, a degraded
// session, and the i18n toggle.
//
// NAVIGATION NOTE: levels are driven through the right "Details panel" sidebar
// (openAccount/openRepo in the harness), NOT the React Flow canvas. The canvas
// exposes the same level-nav buttons, but it currently renders with zero height
// (see the product-bug report: the dashboard's `h-full` chain does not resolve
// through the shell's `py-10` <Outlet> wrapper), so its nodes are not clickable.
// The sidebar path is the stable, always-available way to drill levels.

test.describe('dashboard full UI journey', () => {
  const pageErrors: string[] = [];

  test.beforeEach(async ({ page }) => {
    pageErrors.length = 0;
    page.on('pageerror', (err) => pageErrors.push(String(err)));
    await seedAuth(page);
    await installApiRoutes(page);
  });

  test('levels 0→1→2, session drawer, all four tabs, degraded + i18n', async ({ page }) => {
    // ---- Level 0: root — sidebar charts, legend, accounts -------------------
    await page.goto('/dashboard');
    await expect(page.getByRole('heading', { name: 'Your fkst sessions' })).toBeVisible();

    const side = sidebar(page);
    await expect(side.getByRole('button', { name: `Open account ${PERSONAL}` })).toBeVisible();
    await expect(side.getByRole('button', { name: `Open account ${ORG}` })).toBeVisible();
    // Sidebar charts + legend render at root.
    await expect(page.getByText('Running sessions')).toBeVisible();
    await expect(page.getByText('Packages in use')).toBeVisible();
    await expect(page.getByText('Legend').first()).toBeVisible();
    // The canvas region is mounted (even though its interactive nodes live
    // behind the height issue noted above).
    await expect(page.locator('.react-flow')).toBeAttached();
    await shot(page, '01-level0-root', true);

    // ---- Level 1: an account's repositories (sidebar) -----------------------
    await openAccount(page, PERSONAL);
    await expect(
      side.getByRole('button', { name: `Open repository ${PERSONAL}/${REPO}` })
    ).toBeVisible();
    await expect(
      side.getByRole('button', { name: `Open repository ${PERSONAL}/api-service` })
    ).toBeVisible();
    await shot(page, '02-level1-account-repos', true);

    // ---- Level 2: a repo's sessions -----------------------------------------
    await openRepo(page, PERSONAL, REPO);
    await expect(page.getByText('feature-auth').first()).toBeVisible();
    await expect(page.getByText('refactor-core').first()).toBeVisible();
    await shot(page, '03-level2-sessions', true);

    // ---- Open the live session's detail drawer ------------------------------
    await page.getByRole('button', { name: 'Open details for session feature-auth' }).click();
    const dialog = page.getByTestId('session-detail');
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole('heading', { name: 'feature-auth' })).toBeVisible();
    await expect(drawerTab(page, 'Status')).toHaveAttribute('aria-selected', 'true');
    await shot(page, '04-drawer-status');

    // Status tab: decoded lifecycle + per-work-item chips.
    await expect(dialog.getByText('Health', { exact: false }).first()).toBeVisible();
    // .first(): the status-charts work repeats these phase words in the
    // distribution chart + timeline, so a bare getByText is ambiguous.
    await expect(dialog.getByText('Thinking').first()).toBeVisible();
    await expect(dialog.getByText('Implementing').first()).toBeVisible();
    await expect(dialog.getByText('Ready').first()).toBeVisible();
    await expect(dialog.getByText('Done').first()).toBeVisible();

    // Live engine details → observe queues.
    await dialog.getByRole('button', { name: 'Live engine details' }).click();
    await expect(
      dialog.getByText('workflow-writer.workflow_writer_materialization_tick')
    ).toBeVisible();
    await expect(dialog.getByText('github-devloop.reconcile_tick')).toBeVisible();
    await expect(dialog.getByText('2 deliveries pending')).toBeVisible();
    await shot(page, '05-status-live-engine');

    // ---- Packages tab -------------------------------------------------------
    await drawerTab(page, 'Packages').click();
    await expect(dialog.getByText('Dev workflow', { exact: true })).toBeVisible();
    await expect(dialog.getByText('Devloop', { exact: true })).toBeVisible();
    await expect(
      dialog.getByText('ChronoAIProject/fkst-packages@fkst-hosted:workflow-dev')
    ).toBeVisible();
    await expect(dialog.getByText('Queue activity')).toBeVisible();
    await shot(page, '06-packages');

    // ---- Logs tab -----------------------------------------------------------
    await drawerTab(page, 'Logs').click();
    const logFiles = page.getByRole('tablist', { name: 'Log files' });
    await expect(logFiles.getByRole('tab', { name: /driver\.log/ })).toBeVisible();
    await expect(logFiles.getByRole('tab', { name: /codex\.log/ })).toBeVisible();
    await expect(dialog.getByText(/Tail — showing the last/)).toBeVisible();

    const search = dialog.getByPlaceholder('Find in file…');
    await search.fill('reconcile');
    await expect(dialog.getByText('5 matches')).toBeVisible();
    await expect(dialog.locator('mark').first()).toBeVisible();
    await shot(page, '07-logs-search');

    await dialog.getByRole('button', { name: 'Refresh' }).click();
    await expect(dialog.locator('mark').first()).toBeVisible();
    await logFiles.getByRole('tab', { name: /codex\.log/ }).click();
    await expect(dialog.getByText(/proposed patch for #112/)).toBeVisible();

    // ---- Outcomes tab (text, image, video previews) -------------------------
    await drawerTab(page, 'Outcomes').click();
    await expect(dialog.getByText('feat: login form')).toBeVisible();
    await expect(dialog.getByText('#301')).toBeVisible();

    // Previews are lazy: expand the row, then explicitly "Load preview" (only
    // one row is expanded at a time, so a single Load-preview control is live).
    await dialog.getByRole('button', { name: /login\.tsx/ }).click();
    await dialog.getByRole('button', { name: 'Load preview' }).click();
    await expect(dialog.getByText('export function LoginForm()')).toBeVisible();

    await dialog.getByRole('button', { name: /login-screenshot\.png/ }).click();
    await dialog.getByRole('button', { name: 'Load preview' }).click();
    const img = dialog.locator('img[alt="login-screenshot.png"]');
    await expect(img).toBeVisible();
    expect(await img.evaluate((el: HTMLImageElement) => el.naturalWidth)).toBeGreaterThan(0);

    await dialog.getByRole('button', { name: /login-demo\.mp4/ }).click();
    await dialog.getByRole('button', { name: 'Load preview' }).click();
    await expect(dialog.locator('video')).toBeVisible();
    await shot(page, '08-outcomes-previews');

    // ---- A degraded session: select it in the rail to swap the inline detail -
    await page.getByRole('button', { name: 'Open details for session refactor-core' }).click();
    const degraded = page.getByTestId('session-detail');
    await expect(degraded.getByRole('heading', { name: 'refactor-core' })).toBeVisible();
    await expect(degraded.getByText('Degraded').first()).toBeVisible();
    await expect(degraded.getByText('Failed').first()).toBeVisible();
    await shot(page, '09-degraded');

    // Not live → the live-engine fetch is not offered; the availability notice
    // renders instead (the lifecycle-aware Status tab gates the button on
    // liveness === 'live').
    await expect(
      degraded.getByText(/Live engine details are available while the session is running/)
    ).toBeVisible();

    // ---- i18n toggle (the app is dark-only; no theme toggle) ----------------
    await page.getByRole('button', { name: '中文' }).click();
    await expect(page.locator('html')).toHaveAttribute('lang', 'zh');
    await expect(page.getByRole('heading', { name: '你的 fkst 会话' })).toBeVisible();
    await shot(page, '10-dashboard-zh', true);
    await page.getByRole('button', { name: 'EN' }).click();
    await expect(page.locator('html')).toHaveAttribute('lang', 'en');

    // No uncaught runtime errors during the whole journey.
    expect(pageErrors, `page errors: ${pageErrors.join('; ')}`).toEqual([]);
  });

  test('the collapsible legend expands to reveal its status key', async ({ page }) => {
    await page.goto('/dashboard');
    await expect(page.getByRole('heading', { name: 'Your fkst sessions' })).toBeVisible();
    const legend = page.getByRole('button', { name: /Legend/i }).first();
    await expect(legend).toBeVisible();
    // Collapsible: aria-expanded flips and the status entries reveal on toggle.
    const before = await legend.getAttribute('aria-expanded');
    await legend.click();
    await settle(page);
    const after = await legend.getAttribute('aria-expanded');
    expect(before).not.toBe(after);
    await shot(page, '12-legend-toggled');
  });

  test('overview load failure shows the error screen', async ({ page }) => {
    await page.unroute('**/api/v1/**');
    await installApiRoutes(page, { failOverview: true });
    await page.goto('/dashboard');
    await expect(
      page.getByText('Could not load your repositories. Please try again.')
    ).toBeVisible();
    await shot(page, '11-overview-load-failed', true);
  });
});
