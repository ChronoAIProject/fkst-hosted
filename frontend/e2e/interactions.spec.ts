import { test, expect, type Page } from '@playwright/test';
import { installApiRoutes, seedAuth, LIVE_SESSION_ID } from './fixtures';
import { drawerTab, openAccount, openRepo, shot } from './harness';

// Interaction quality: keyboard-accessible tabs, a one-click full-id copy, toast
// feedback on mutations, the involuntary-expiry re-auth prompt, the 404 view, the
// ErrorBoundary fallback, and a prefers-reduced-motion pass proving transitions
// collapse without ever hiding content.

/** Drill to the live session's detail drawer (via the sidebar, canvas-agnostic). */
async function openLiveDrawer(page: Page) {
  await seedAuth(page);
  await installApiRoutes(page);
  await page.goto('/dashboard');
  await expect(page.getByRole('heading', { name: 'Your fkst sessions' })).toBeVisible();
  await openAccount(page, 'octo-dev');
  await openRepo(page, 'octo-dev', 'web-app');
  await page.getByRole('button', { name: 'Open details for session feature-auth' }).click();
  await expect(page.getByTestId('session-detail')).toBeVisible();
}

test.describe('accessibility, feedback, and error surfaces', () => {
  test('drawer tabs: roving arrow-key focus + aria tab/panel linkage', async ({ page }) => {
    await openLiveDrawer(page);
    const status = drawerTab(page, 'Status');
    const packages = drawerTab(page, 'Packages');
    const logs = drawerTab(page, 'Logs');

    // Status is selected initially and is the only Tab-reachable tab (roving).
    await expect(status).toHaveAttribute('aria-selected', 'true');
    await expect(status).toHaveAttribute('tabindex', '0');
    await expect(packages).toHaveAttribute('tabindex', '-1');

    // aria linkage: every tab controls the single panel; the panel is labelled
    // by the ACTIVE tab.
    const panelId = await status.getAttribute('aria-controls');
    expect(panelId).toBeTruthy();
    // React's useId ids contain colons — target by attribute, not a CSS #id.
    const panel = page.locator(`[id="${panelId}"]`);
    await expect(panel).toHaveRole('tabpanel');
    await expect(panel).toHaveAttribute('aria-labelledby', (await status.getAttribute('id'))!);

    // ArrowRight moves selection AND focus to the next tab (automatic activation).
    await status.focus();
    await page.keyboard.press('ArrowRight');
    await expect(packages).toHaveAttribute('aria-selected', 'true');
    await expect(packages).toBeFocused();
    await expect(panel).toHaveAttribute('aria-labelledby', (await packages.getAttribute('id'))!);

    // ArrowRight again → Logs; ArrowLeft wraps back to Packages.
    await page.keyboard.press('ArrowRight');
    await expect(logs).toHaveAttribute('aria-selected', 'true');
    await page.keyboard.press('ArrowLeft');
    await expect(packages).toHaveAttribute('aria-selected', 'true');
    await shot(page, 'ix-01-drawer-tab-keyboard');
  });

  test('the FULL session id is shown and one click copies it to the clipboard', async ({
    page,
    context,
  }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await openLiveDrawer(page);
    const dialog = page.getByTestId('session-detail');
    // Full id (not the 8-char prefix) is rendered in the header.
    await expect(dialog.getByText(LIVE_SESSION_ID, { exact: true })).toBeVisible();

    await dialog.getByRole('button', { name: 'Copy session ID' }).click();
    const clip = await page.evaluate(() => navigator.clipboard.readText());
    expect(clip, 'the full id is on the clipboard').toBe(LIVE_SESSION_ID);
    await shot(page, 'ix-02-copy-session-id');
  });

  test('creating a session raises a success toast', async ({ page }) => {
    await seedAuth(page);
    await installApiRoutes(page);
    await page.goto('/dashboard');
    await expect(page.getByRole('heading', { name: 'Your fkst sessions' })).toBeVisible();
    await openAccount(page, 'octo-dev');
    await openRepo(page, 'octo-dev', 'web-app');
    await page.getByRole('button', { name: 'New session' }).click();
    const modal = page.getByRole('dialog');
    await modal.getByLabel('Session name').fill('my-new-session');
    await modal.getByLabel('Packages 1').fill('owner/repo@main:pkg');
    await modal.getByRole('button', { name: 'Create trigger issue' }).click();

    // The toast is a polite live region — assert its confirming text.
    await expect(page.getByText('Session created')).toBeVisible();
    await shot(page, 'ix-03-create-toast');
  });

  test('stopping a session confirms through the ConfirmDialog and closes it', async ({ page }) => {
    await seedAuth(page);
    await installApiRoutes(page);
    await page.goto('/dashboard');
    await expect(page.getByRole('heading', { name: 'Your fkst sessions' })).toBeVisible();
    await openAccount(page, 'octo-dev');
    await openRepo(page, 'octo-dev', 'web-app');

    await page.getByRole('button', { name: 'Stop session feature-auth' }).click();
    const confirm = page.getByRole('dialog', { name: 'Stop session feature-auth?' });
    await expect(confirm).toBeVisible();
    await confirm.getByRole('button', { name: 'Stop session' }).click();
    // The mutation succeeds → the confirm dialog closes (the visible confirmation
    // of the stop; the list then re-fetches). NOTE: unlike create, the stop path
    // raises no toast today — this asserts the confirmation that exists.
    await expect(confirm).toHaveCount(0);
    await shot(page, 'ix-04-stop-confirmed');
  });

  test('an involuntary 401 shows the re-auth prompt, not the cold sign-in', async ({ page }) => {
    await seedAuth(page); // access token but NO refresh token → 401 cannot recover
    await installApiRoutes(page, { overviewStatus: 401 });
    await page.goto('/dashboard');

    // The context-preserving re-auth prompt appears…
    await expect(page.getByText('Your session expired')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Sign in again' })).toBeVisible();
    // …and NOT the cold, never-signed-in gate card.
    await expect(
      page.getByRole('heading', { name: 'Sign in to view your dashboard' })
    ).toHaveCount(0);
    await shot(page, 'ix-05-session-expired');
  });

  test('an unknown URL renders the 404 view naming the missing path', async ({ page }) => {
    await page.goto('/this-route-does-not-exist');
    await expect(page.getByRole('heading', { name: 'This page does not exist' })).toBeVisible();
    // The 404 names the exact path rather than silently redirecting.
    await expect(page.getByText('/this-route-does-not-exist')).toBeVisible();
    await expect(page.getByRole('link', { name: 'Back to home →' })).toBeVisible();
    await shot(page, 'ix-06-not-found');
  });

  test('a render throw lands on the ErrorBoundary fallback', async ({ page }) => {
    // A malformed overview (account.repos = null) throws while a level renders,
    // which the route error boundary catches and replaces with the fallback.
    await seedAuth(page);
    await installApiRoutes(page, { malformedOverview: true });
    await page.goto('/dashboard');

    const alert = page.getByRole('alert');
    await expect(alert).toBeVisible();
    await expect(alert.getByRole('heading', { name: 'Something went wrong' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Reload the page' })).toBeVisible();
    await shot(page, 'ix-07-error-boundary');
  });
});

test.describe('prefers-reduced-motion', () => {
  // Emulate the OS/browser reduced-motion preference (equivalent to the context
  // `reducedMotion: 'reduce'` option) — applied on the page so it survives every
  // navigation in the flow below.
  test.use({ reducedMotion: 'reduce' });

  test('transitions collapse to their final state; no content is hidden', async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await openLiveDrawer(page);
    const dialog = page.getByTestId('session-detail');

    // The drawer + its header content are at their FINAL state immediately (no
    // slide/fade to wait out) — content is present, never gated behind motion.
    await expect(dialog.getByRole('heading', { name: 'feature-auth' })).toBeVisible();
    await expect(dialog.getByText(LIVE_SESSION_ID, { exact: true })).toBeVisible();

    // The header chips carry `anim-chip-in`, which the reduced-motion media
    // query disables. With emulation active, EVERY such chip's entrance
    // animation collapses to none, yet each still renders at its final state.
    const collapsed = await page.evaluate(() => {
      const chips = [...document.querySelectorAll('.anim-chip-in')] as HTMLElement[];
      return {
        reduceMatches: window.matchMedia('(prefers-reduced-motion: reduce)').matches,
        count: chips.length,
        allNone: chips.every((c) => getComputedStyle(c).animationName === 'none'),
        allVisible: chips.every((c) => {
          const r = c.getBoundingClientRect();
          return r.width > 0 && r.height > 0;
        }),
      };
    });
    expect(collapsed.reduceMatches, 'reduced-motion emulation is active').toBe(true);
    expect(collapsed.count).toBeGreaterThan(0);
    expect(collapsed.allNone, 'entrance animations are disabled').toBe(true);
    expect(collapsed.allVisible, 'chips still render at their final state').toBe(true);

    // Tab bodies crossfade under FadeSwap; reduced motion makes the swap instant,
    // so switching tabs reveals content with no waiting.
    await drawerTab(page, 'Packages').click();
    await expect(dialog.getByText('Configuration')).toBeVisible();
    await shot(page, 'ix-08-reduced-motion');
  });
});
