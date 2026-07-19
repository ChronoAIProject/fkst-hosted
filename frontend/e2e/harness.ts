// Shared E2E harness helpers: screenshot/settle utilities and the sidebar-driven
// navigation used across specs.
//
// WHY navigate via the sidebar (not the canvas): the dashboard's React Flow
// canvas is the OTHER path to drill levels, but the level-navigation buttons it
// exposes (`Open account …`, `Open repository …`) are DUPLICATED in the sidebar
// "Details panel", and the sidebar path is stable regardless of the canvas'
// current height. Scoping to the `complementary` panel also disambiguates the
// otherwise-colliding accessible names. Every spec drills levels through here so
// a single place owns that decision.

import { expect, type Page } from '@playwright/test';

/** Absolute screenshot directory handed down by the orchestrator (the scratchpad
 *  for this session). Kept identical to dashboard.spec's original constant so
 *  every spec drops its shots in one place. */
export const SHOTS =
  '/private/tmp/claude-501/-Users-chronoai-code-work-fkst-hosted/1faa5963-9e29-40ef-a0bd-52444366bc74/scratchpad/ui-shots';

/** Wait out every FINITE entrance animation (drawer slide, overlay fade, row
 *  stagger, route crossfade) so a capture or a post-settle assertion sees the
 *  resting UI — not a mid-transition frame. Infinite loops (spinners, glow) are
 *  skipped because their `finished` promise never resolves. Also robust under
 *  the RouteTransition's brief double-mount: settling lets the exiting route
 *  leave before we assert. */
export async function settle(page: Page): Promise<void> {
  await page.evaluate(() =>
    Promise.all(
      document
        .getAnimations()
        .filter((a) => a.effect?.getComputedTiming().iterations !== Infinity)
        .map((a) => a.finished.catch(() => {}))
    )
  );
}

/** Settle, then screenshot into SHOTS. */
export async function shot(page: Page, name: string, fullPage = false): Promise<void> {
  await settle(page);
  await page.screenshot({ path: `${SHOTS}/${name}.png`, fullPage });
}

/** The dashboard's right "Details panel" sidebar — the stable surface for level
 *  navigation while the canvas height issue is open. */
export const sidebar = (page: Page) =>
  page.getByRole('complementary', { name: 'Details panel' });

/** The session-detail drawer's own tab strip (scoped so per-file log tabs never
 *  collide with it). */
export const drawerTab = (page: Page, name: string | RegExp) =>
  page.getByRole('tablist', { name: 'Session detail sections' }).getByRole('tab', { name });

/** Drill root → account via the sidebar arrow. */
export async function openAccount(page: Page, login: string): Promise<void> {
  await sidebar(page).getByRole('button', { name: `Open account ${login}` }).click();
}

/** Drill account → repo (level 2 sessions) via the sidebar arrow. */
export async function openRepo(page: Page, owner: string, name: string): Promise<void> {
  await sidebar(page).getByRole('button', { name: `Open repository ${owner}/${name}` }).click();
  await expect(page.getByRole('heading', { name: 'Sessions' }).first()).toBeVisible();
}

/** Read the current React Flow zoom (the viewport transform's scale factor).
 *  Used to prove a wheel over the canvas never zooms it. Returns null when the
 *  viewport isn't present. */
export async function reactFlowZoom(page: Page): Promise<number | null> {
  return page.evaluate(() => {
    const vp = document.querySelector('.react-flow__viewport') as HTMLElement | null;
    if (!vp) return null;
    const m = new DOMMatrixReadOnly(getComputedStyle(vp).transform);
    return m.a; // uniform scale → the `a` component is the zoom
  });
}
