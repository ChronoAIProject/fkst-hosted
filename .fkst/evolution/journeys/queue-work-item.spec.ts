// Executable journey jny_2f7c — "Queue a work item onto a running session".
//
// EVOLUTION-MANAGED OUTPUT (spec section 12.3). Regenerated from the product
// model; edit `observed/journeys.yaml` and the generator, not this file.
//
// A journey is both product EVIDENCE and CAPTURE SOURCE (section 23.5): this one
// run verifies capability cap_9d41 AND produces the screenshots and the video
// frames every other artifact derives from. That is what stops a demo from
// drifting away from the behaviour it claims to show — the video cannot pass
// while the assertion fails, because they are the same run.
//
// SYNTHETIC DATA ONLY (section 25.7). Every response is served by the E2E route
// fixtures, which mirror the real wire shapes field for field while naming
// fictional accounts. No live backend, no credential, no production data.
//
// WHY the fixtures are imported rather than copied: they are the repository's
// single description of the API's wire shapes, so a copy here would silently
// rot. Their only `@playwright/test` import is type-only, so importing them
// cannot pull a second runner instance into this file.

import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test, expect, type Page } from '@playwright/test';
import { installApiRoutes, seedAuth, PERSONAL, REPO } from '../../../frontend/e2e/fixtures';

// Resolved from this file rather than from the cwd: the journey is run by a
// config two directories away, and a cwd-relative path would write captures
// wherever the runner happened to be invoked.
const HERE = dirname(fileURLToPath(import.meta.url));
/** Managed screenshots subtree (section 13.2 fixes this destination). */
const SHOTS = join(HERE, '..', 'screenshots');
/** Build output, NOT Evolution state — section 12.2 keeps temporary frames out. */
const CHECKPOINTS = join(HERE, '..', '..', '..', 'tools', 'evolution', 'out', 'checkpoints.json');

interface Checkpoint {
  id: string;
  /** Milliseconds from the first navigation — the video's zero. */
  offsetMs: number;
  caption: string;
}

const checkpoints: Checkpoint[] = [];
let startedAt = 0;

/**
 * Wait out every FINITE entrance animation so a capture sees the resting UI
 * rather than a mid-transition frame. Infinite loops (spinners, glow) are
 * skipped because their `finished` promise never resolves.
 */
async function settle(page: Page): Promise<void> {
  await page.evaluate(() =>
    Promise.all(
      document
        .getAnimations()
        .filter((a) => a.effect?.getComputedTiming().iterations !== Infinity)
        .map((a) => a.finished.catch(() => {}))
    )
  );
}

/**
 * Record a narration checkpoint and, when the capture is a declared screenshot,
 * write it into the managed screenshots subtree.
 *
 * The caption is deliberately recorded HERE rather than written by hand later:
 * a caption authored away from the step it describes is the first thing to go
 * stale when the journey changes.
 */
async function checkpoint(
  page: Page,
  id: string,
  caption: string,
  capture: 'screenshot' | 'video-only' = 'screenshot'
): Promise<void> {
  await settle(page);
  checkpoints.push({ id, offsetMs: Date.now() - startedAt, caption });
  if (capture === 'screenshot') {
    await mkdir(SHOTS, { recursive: true });
    await page.screenshot({ path: join(SHOTS, `${id}.png`), fullPage: false });
  }
  // A short hold so the moment is legible in the video. Without it a capture is
  // a single frame nobody can read at playback speed.
  await page.waitForTimeout(1200);
}

/** The dashboard's right "Details panel" — the stable surface for level navigation. */
const sidebar = (page: Page) => page.getByRole('complementary', { name: 'Details panel' });

test.describe('jny_2f7c — queue a work item onto a running session', () => {
  const pageErrors: string[] = [];

  test.beforeEach(async ({ page }) => {
    pageErrors.length = 0;
    page.on('pageerror', (err) => pageErrors.push(String(err)));
    await seedAuth(page);
    await installApiRoutes(page);
  });

  test.afterEach(() => {
    // A journey that threw in the browser is not evidence of anything, even if
    // every assertion happened to pass.
    expect(pageErrors, 'the journey must run without page errors').toEqual([]);
  });

  test('a session owner queues work without leaving the dashboard', async ({ page }) => {
    startedAt = Date.now();

    // ---- The dashboard root: every account the App is installed on ----------
    await page.goto('/dashboard');
    await expect(page.getByRole('heading', { name: 'Your fkst sessions' })).toBeVisible();
    await checkpoint(
      page,
      'overview',
      'Every account and repository where the fkst GitHub App is installed.',
      'video-only'
    );

    // ---- Drill into an account, then a repository ---------------------------
    await sidebar(page).getByRole('button', { name: `Open account ${PERSONAL}` }).click();
    await expect(
      sidebar(page).getByRole('button', { name: `Open repository ${PERSONAL}/${REPO}` })
    ).toBeVisible();
    await checkpoint(page, 'account-repos', 'Drill into an account to see its repositories.', 'video-only');

    await sidebar(page).getByRole('button', { name: `Open repository ${PERSONAL}/${REPO}` }).click();
    await expect(page.getByRole('heading', { name: 'Sessions' }).first()).toBeVisible();
    await expect(page.getByText('feature-auth').first()).toBeVisible();
    await checkpoint(
      page,
      'sessions-level',
      'A repository’s running sessions, each backed by its trigger issue.'
    );

    // ---- Queue work from the selected session -------------------------------
    await page.getByRole('button', { name: 'Add work item' }).click();
    const composer = page.getByRole('dialog', { name: 'Queue work' });
    await expect(composer).toBeVisible();

    // The label selector carries the session's COMPLETE effective set, including
    // labels discovered from its packages — the property that makes the queued
    // issue routable to exactly this session.
    await expect(composer.getByLabel('Work label').locator('option')).toHaveText([
      'fkst-security',
      'fkst-work',
    ]);
    await composer.getByLabel('Title').fill('Audit callback validation');
    await composer
      .getByLabel('Details (optional)')
      .fill('## Acceptance criteria\n\n- [ ] Cover expired states');
    await checkpoint(
      page,
      'work-composer',
      'Queue a work item. The label list is the session’s effective set; details accept Markdown.'
    );

    await composer.getByRole('button', { name: 'Queue work' }).click();
    await expect(composer).toHaveCount(0);
    await checkpoint(
      page,
      'queued',
      'The work issue is opened on GitHub as you — the reconciler claims it for this session.',
      'video-only'
    );

    // ---- Watch the session that will pick it up -----------------------------
    await page.getByRole('button', { name: 'Open details for session feature-auth' }).click();
    const detail = page.getByTestId('session-detail');
    await expect(detail).toBeVisible();
    await expect(detail.getByRole('heading', { name: 'feature-auth' })).toBeVisible();
    await checkpoint(
      page,
      'session-detail',
      'Follow the session’s health, phase and work items as it picks the item up.'
    );

    await mkdir(dirname(CHECKPOINTS), { recursive: true });
    await writeFile(CHECKPOINTS, `${JSON.stringify(checkpoints, null, 2)}\n`, 'utf8');
  });
});
