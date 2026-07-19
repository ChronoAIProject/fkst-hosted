import { test, expect, type Page } from '@playwright/test';
import {
  installApiRoutes,
  seedAuth,
  PERSONAL,
  ORG,
  REPO,
} from './fixtures';

// Absolute screenshot directory handed down by the orchestrator.
const SHOTS =
  '/private/tmp/claude-501/-Users-chronoai-code-work-fkst-hosted/1faa5963-9e29-40ef-a0bd-52444366bc74/scratchpad/ui-shots';

const shot = async (page: Page, name: string, fullPage = false) => {
  // Let any in-flight CSS entrance animation (drawer slide-in, overlay fade, row
  // stagger) settle first — otherwise the capture is a mid-transition frame that
  // misrepresents the settled UI. `toBeVisible()` does not wait for animations.
  await page.evaluate(() =>
    Promise.all(
      document
        .getAnimations()
        // Skip infinite loops (node glow, dot blink, shimmer) whose `finished`
        // never resolves; only wait out finite entrance animations.
        .filter((a) => a.effect?.getComputedTiming().iterations !== Infinity)
        .map((a) => a.finished.catch(() => {})),
    ),
  );
  await page.screenshot({ path: `${SHOTS}/${name}.png`, fullPage });
};

/** The drawer's own tab strip (scoped so log-file tabs never collide). */
const drawerTab = (page: Page, name: string) =>
  page
    .getByRole('tablist', { name: 'Session detail sections' })
    .getByRole('tab', { name });

test.describe('dashboard full UI journey', () => {
  const pageErrors: string[] = [];

  test.beforeEach(async ({ page }) => {
    pageErrors.length = 0;
    page.on('pageerror', (err) => pageErrors.push(String(err)));
    await seedAuth(page);
    await installApiRoutes(page);
  });

  test('level 0 → 1 → 2, session drawer, all four tabs, degraded + i18n', async ({
    page,
  }) => {
    // ---- Flow 1: Level 0 root canvas + sidebar -----------------------------
    await page.goto('/dashboard');
    await expect(page.getByRole('heading', { name: 'Your fkst sessions' })).toBeVisible();

    const canvas = page.locator('.react-flow');
    const accountNode = canvas.getByRole('button', { name: `Open account ${PERSONAL}` });
    await expect(accountNode).toBeVisible();
    await expect(canvas.getByRole('button', { name: `Open account ${ORG}` })).toBeVisible();
    // Sidebar charts + legend render at root.
    await expect(page.getByText('Running sessions')).toBeVisible();
    await expect(page.getByText('Packages in use')).toBeVisible();
    await expect(page.getByText('Legend').first()).toBeVisible();
    await shot(page, '01-level0-root-canvas', true);

    // ---- Flow 2: Drill into an account (level 1 repos) ---------------------
    await accountNode.click();
    const repoNode = canvas.getByRole('button', {
      name: `Open repository ${PERSONAL}/${REPO}`,
    });
    await expect(repoNode).toBeVisible();
    await expect(canvas.getByRole('button', { name: `Open repository ${PERSONAL}/api-service` })).toBeVisible();
    await shot(page, '02-level1-account-repos', true);

    // ---- Flow 3: Drill into a repo (level 2 sessions list) -----------------
    await repoNode.click();
    await expect(page.getByRole('heading', { name: 'Sessions' })).toBeVisible();
    // Both session cards present in the level-2 sidebar.
    await expect(page.getByText('feature-auth').first()).toBeVisible();
    await expect(page.getByText('refactor-core').first()).toBeVisible();
    await shot(page, '03-level2-sessions-list', true);

    // ---- Flow 4: Open a session's Details drawer ---------------------------
    await page
      .getByRole('button', { name: 'Open details for session feature-auth' })
      .click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole('heading', { name: 'feature-auth' })).toBeVisible();
    await expect(drawerTab(page, 'Status')).toHaveAttribute('aria-selected', 'true');
    await shot(page, '04-drawer-open-status', false);

    // ---- Flow 5: Status tab (decoded lifecycle + work-item chips) ----------
    // Decoded phase + per-work-item chips.
    await expect(dialog.getByText('Health', { exact: false }).first()).toBeVisible();
    await expect(dialog.getByText('Thinking')).toBeVisible();
    await expect(dialog.getByText('Implementing')).toBeVisible();
    await expect(dialog.getByText('Ready')).toBeVisible();
    await expect(dialog.getByText('Done')).toBeVisible(); // the closed work issue
    await shot(page, '05a-status-lifecycle-workitems', false);

    // Click "Live engine details" → observe queues render.
    await dialog.getByRole('button', { name: 'Live engine details' }).click();
    await expect(
      dialog.getByText('workflow-writer.workflow_writer_materialization_tick')
    ).toBeVisible();
    await expect(dialog.getByText('github-devloop.reconcile_tick')).toBeVisible();
    await expect(dialog.getByText('2 deliveries pending')).toBeVisible();
    await shot(page, '05b-status-live-engine-queues', false);

    // ---- Flow 6: Packages tab ---------------------------------------------
    await drawerTab(page, 'Packages').click();
    await expect(dialog.getByText('Dev workflow', { exact: true })).toBeVisible();
    await expect(dialog.getByText('Devloop', { exact: true })).toBeVisible();
    await expect(
      dialog.getByText('ChronoAIProject/fkst-packages@fkst-hosted:workflow-dev')
    ).toBeVisible();
    // The observe snapshot was already loaded on the Status tab → shared here.
    await expect(dialog.getByText('Queue activity')).toBeVisible();
    await shot(page, '06-packages-tab', false);

    // ---- Flow 7: Logs tab (file tabs, select, search, refresh) -------------
    await drawerTab(page, 'Logs').click();
    const logFiles = page.getByRole('tablist', { name: 'Log files' });
    await expect(logFiles.getByRole('tab', { name: /driver\.log/ })).toBeVisible();
    await expect(logFiles.getByRole('tab', { name: /codex\.log/ })).toBeVisible();
    await expect(logFiles.getByRole('tab', { name: /README\.md/ })).toBeVisible();
    // driver.log auto-selected → truncation notice visible.
    await expect(dialog.getByText(/Tail — showing the last/)).toBeVisible();
    await shot(page, '07a-logs-file-selected', false);

    // Search for a token that lives in the driver log → highlighted matches.
    const search = dialog.getByPlaceholder('Find in file…');
    await search.fill('reconcile');
    await expect(dialog.getByText('5 matches')).toBeVisible();
    await expect(dialog.locator('mark').first()).toBeVisible();
    await shot(page, '07b-logs-search-highlight', false);

    // Refresh re-fetches the same file (button stays functional).
    await dialog.getByRole('button', { name: 'Refresh' }).click();
    await expect(dialog.locator('mark').first()).toBeVisible();

    // Switch to a different file tab.
    await logFiles.getByRole('tab', { name: /codex\.log/ }).click();
    await expect(dialog.getByText(/proposed patch for #112/)).toBeVisible();
    await shot(page, '07c-logs-other-file', false);

    // ---- Flow 8: Outcomes tab (PR files: text, image, video previews) ------
    await drawerTab(page, 'Outcomes').click();
    await expect(dialog.getByText('feat: login form')).toBeVisible();
    await expect(dialog.getByText('#301')).toBeVisible();
    await shot(page, '08a-outcomes-prs', false);

    // Expand the text file preview.
    await dialog.getByRole('button', { name: /login\.tsx/ }).click();
    await expect(dialog.getByText('export function LoginForm()')).toBeVisible();
    await shot(page, '08b-outcomes-text-preview', false);

    // Expand the image preview (single expansion at a time).
    await dialog.getByRole('button', { name: /login-screenshot\.png/ }).click();
    const img = dialog.locator('img[alt="login-screenshot.png"]');
    await expect(img).toBeVisible();
    // The image actually decoded (real PNG bytes through the blob route).
    expect(await img.evaluate((el: HTMLImageElement) => el.naturalWidth)).toBeGreaterThan(0);
    await shot(page, '08c-outcomes-image-preview', false);

    // Expand the video preview.
    await dialog.getByRole('button', { name: /login-demo\.mp4/ }).click();
    await expect(dialog.locator('video')).toBeVisible();
    await shot(page, '08d-outcomes-video-preview', false);

    // Close the live-session drawer.
    await dialog.getByRole('button', { name: 'Close session details' }).click();
    await expect(page.getByRole('dialog')).toHaveCount(0);

    // ---- Flow 9: A degraded session's status -------------------------------
    await page
      .getByRole('button', { name: 'Open details for session refactor-core' })
      .click();
    const degradedDialog = page.getByRole('dialog');
    await expect(degradedDialog.getByRole('heading', { name: 'refactor-core' })).toBeVisible();
    // Degraded phase chip appears both in header and lifecycle strip.
    await expect(degradedDialog.getByText('Degraded').first()).toBeVisible();
    await expect(degradedDialog.getByText('Failed')).toBeVisible(); // impl-failed work item
    await shot(page, '09a-degraded-status', false);

    // Its live-engine fetch fails → error state (empty/error coverage).
    await degradedDialog.getByRole('button', { name: 'Live engine details' }).click();
    await expect(
      degradedDialog.getByText('Could not load the live engine details.')
    ).toBeVisible();
    await shot(page, '09b-degraded-live-engine-error', false);

    await degradedDialog.getByRole('button', { name: 'Close session details' }).click();
    await expect(page.getByRole('dialog')).toHaveCount(0);

    // ---- Flow 10: i18n toggle (the app is dark-only; no theme toggle) ------
    await page.getByRole('button', { name: '中文' }).click();
    await expect(page.locator('html')).toHaveAttribute('lang', 'zh');
    await expect(page.getByRole('heading', { name: '你的 fkst 会话' })).toBeVisible();
    await shot(page, '10-dashboard-zh-locale', true);
    // back to English so the run leaves a clean default.
    await page.getByRole('button', { name: 'EN' }).click();
    await expect(page.locator('html')).toHaveAttribute('lang', 'en');

    // No uncaught runtime errors during the whole journey.
    expect(pageErrors, `page errors: ${pageErrors.join('; ')}`).toEqual([]);
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
