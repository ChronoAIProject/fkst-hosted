import type { SiteContent, TitleBody } from '@/i18n/types';

/** Which side of the target the coachmark tooltip prefers. The overlay
 *  auto-flips to the opposite side (or the roomiest side) when the preferred
 *  one would push the card out of the viewport. */
export type TourPlacement = 'top' | 'bottom' | 'left' | 'right';

/**
 * One tour step.
 *
 * `variant` decides the presentation:
 *  - `'modal'` — a centered dialog card with no page target (welcome / finish).
 *  - `'spotlight'` — dims the page and rings a real element found via
 *    `[data-tour="<target>"]`. When that element is absent (a level/state-
 *    dependent affordance the user has not navigated to), the overlay degrades
 *    to a centered card so the tour never breaks.
 *
 * `contentKey` selects the `{title, body}` card from the `tour.steps` i18n
 * domain, keeping every user-facing string out of this module.
 */
export interface TourStep {
  /** Stable id (also the analytics/debug handle). */
  id: string;
  /** i18n key under `content.tour.steps`. */
  contentKey: keyof SiteContent['tour']['steps'];
  /** `data-tour` value to spotlight, or `null` for a centered modal step. */
  target: string | null;
  variant: 'modal' | 'spotlight';
  /** Preferred tooltip side for spotlight steps (ignored by modal steps). */
  placement: TourPlacement;
}

/**
 * The ordered tour. Reliably-present targets (canvas, breadcrumb, sidebar,
 * refresh, help, environments) get a real spotlight; the level/state-dependent
 * ones (new-session, session-card, new-work-item, new-repo) spotlight only when
 * the user is on the matching level and otherwise fall back to a centered card.
 */
export const TOUR_STEPS: readonly TourStep[] = [
  { id: 'welcome', contentKey: 'welcome', target: null, variant: 'modal', placement: 'bottom' },
  { id: 'canvas', contentKey: 'canvas', target: 'canvas', variant: 'spotlight', placement: 'bottom' },
  { id: 'breadcrumb', contentKey: 'breadcrumb', target: 'breadcrumb', variant: 'spotlight', placement: 'bottom' },
  { id: 'sidebar', contentKey: 'sidebar', target: 'sidebar', variant: 'spotlight', placement: 'left' },
  { id: 'new-session', contentKey: 'newSession', target: 'new-session', variant: 'spotlight', placement: 'left' },
  { id: 'session-detail', contentKey: 'sessionDetail', target: 'session-card', variant: 'spotlight', placement: 'left' },
  { id: 'work-item', contentKey: 'workItem', target: 'new-work-item', variant: 'spotlight', placement: 'left' },
  { id: 'environments', contentKey: 'environments', target: 'environments', variant: 'spotlight', placement: 'bottom' },
  { id: 'new-repo', contentKey: 'newRepo', target: 'new-repo', variant: 'spotlight', placement: 'left' },
  { id: 'refresh', contentKey: 'refresh', target: 'refresh', variant: 'spotlight', placement: 'bottom' },
  { id: 'help', contentKey: 'help', target: 'help', variant: 'spotlight', placement: 'bottom' },
  { id: 'finish', contentKey: 'finish', target: null, variant: 'modal', placement: 'bottom' },
] as const;

/** Resolve a step's localized copy from the active catalog. */
export function stepCopy(content: SiteContent, step: TourStep): TitleBody {
  return content.tour.steps[step.contentKey];
}
