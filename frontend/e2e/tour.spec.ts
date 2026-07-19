import { test, expect } from '@playwright/test';
import { installApiRoutes, seedAuth, TOUR_SEEN_KEY } from './fixtures';
import { shot } from './harness';

test.describe('guided product tour', () => {
  test.beforeEach(async ({ page }) => {
    await installApiRoutes(page);
  });

  test('auto-prompts on first sign-in, walks the steps, and does not re-prompt once seen', async ({
    page,
  }) => {
    await seedAuth(page, { firstRun: true }); // unseen → the tour should auto-open
    await page.goto('/dashboard');

    // First authenticated visit: the welcome step appears on its own, and the
    // seen flag is recorded the moment it opens (value is a timestamp).
    await expect(page.getByRole('dialog').getByText('Welcome to fkst')).toBeVisible();
    expect(await page.evaluate((k) => window.localStorage.getItem(k), TOUR_SEEN_KEY)).not.toBeNull();
    await shot(page, 'tour-01-welcome');

    // Next advances into the spotlight coachmarks over the real UI.
    await page.getByRole('button', { name: 'Next' }).click();
    await expect(page.getByTestId('tour-spotlight')).toBeVisible();
    await expect(page.getByText('The canvas')).toBeVisible();
    await shot(page, 'tour-02-canvas-spotlight');

    // Back returns to the welcome step; Skip ends the tour.
    await page.getByRole('button', { name: 'Back' }).click();
    await expect(page.getByText('Welcome to fkst')).toBeVisible();
    await page.getByRole('button', { name: 'Skip' }).click();
    await expect(page.getByTestId('tour-spotlight')).toHaveCount(0);
    await expect(page.getByText('Welcome to fkst')).toHaveCount(0);

    // Reload: the seen flag suppresses the auto-prompt for this login.
    await page.reload();
    await expect(page.getByRole('heading', { name: 'Your fkst sessions' })).toBeVisible();
    await expect(page.getByText('Welcome to fkst')).toHaveCount(0);
  });

  test('the topbar ? button re-launches the tour even after it has been seen', async ({
    page,
  }) => {
    await seedAuth(page); // default: already seen → no auto-prompt
    await page.goto('/dashboard');
    await expect(page.getByRole('heading', { name: 'Your fkst sessions' })).toBeVisible();
    await expect(page.getByText('Welcome to fkst')).toHaveCount(0);

    // The '?' launcher re-opens it on demand, ignoring the seen flag.
    await page.locator('[data-tour="help"]').click();
    await expect(page.getByRole('dialog').getByText('Welcome to fkst')).toBeVisible();
    await shot(page, 'tour-03-relaunch');

    // Skip ends it (the welcome step is a modal; the ✕ 'End the tour' affordance
    // lives on the spotlight coachmarks).
    await page.getByRole('button', { name: 'Skip' }).click();
    await expect(page.getByText('Welcome to fkst')).toHaveCount(0);
  });

  test('a spotlight step whose target is off the current level degrades to a centered card', async ({
    page,
  }) => {
    await seedAuth(page);
    await page.goto('/dashboard');
    await page.locator('[data-tour="help"]').click();
    // Walk to the 'Start a session' step: its target (new-session) lives at
    // level 2, so at the root it must still render (as a centered card) and not
    // crash / vanish.
    for (const label of ['The canvas', 'Where you are', 'The details panel']) {
      await page.getByRole('button', { name: 'Next' }).click();
      await expect(page.getByText(label)).toBeVisible();
    }
    await page.getByRole('button', { name: 'Next' }).click();
    await expect(page.getByText('Start a session')).toBeVisible();
    await expect(page.getByTestId('tour-spotlight')).toBeVisible();
  });
});
