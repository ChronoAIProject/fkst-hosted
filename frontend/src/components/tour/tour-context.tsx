import { createContext, useCallback, useContext, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { TOUR_STEPS } from './tour-steps';
import type { TourStep } from './tour-steps';

// The guided product tour's state machine. It owns nothing visual — the overlay
// (tour-overlay.tsx) reads this context and renders. Kept deliberately small so
// any route can `useTour()` cheaply.

/** localStorage key that records the tour was auto-prompted for a given login,
 *  so a returning user is never auto-prompted twice on this browser. Manual
 *  re-launch (the topbar `?`) ignores this flag entirely. */
const SEEN_PREFIX = 'fkst-tour-seen-v1:';

/** localStorage accessors that never throw — private-mode / disabled storage
 *  degrades to "not seen" rather than crashing the dashboard. Mirrors the `ls`
 *  guard pattern used in github-auth.tsx. */
const ls = {
  get(k: string): string | null {
    try {
      return window.localStorage.getItem(k);
    } catch {
      return null;
    }
  },
  set(k: string, v: string) {
    try {
      window.localStorage.setItem(k, v);
    } catch {
      /* private mode / storage disabled — ignore */
    }
  },
};

/** The per-login seen key. Exported so tests can assert it directly. */
export function seenKey(userKey: string): string {
  return `${SEEN_PREFIX}${userKey}`;
}

/** Whether the auto-prompt already fired for this login on this browser. A
 *  blank userKey is treated as "seen" so the tour can never auto-fire without a
 *  real per-user key to gate it. */
export function hasSeenTour(userKey: string): boolean {
  if (!userKey) return true;
  return ls.get(seenKey(userKey)) != null;
}

/** Record that the auto-prompt fired for this login. No-op on a blank key. */
export function markTourSeen(userKey: string): void {
  if (!userKey) return;
  ls.set(seenKey(userKey), String(Date.now()));
}

export interface TourContextValue {
  /** True while the tour overlay should be shown. */
  isActive: boolean;
  /** Zero-based index of the current step. */
  index: number;
  /** Total number of steps. */
  total: number;
  /** The current step definition, or null when inactive. */
  current: TourStep | null;
  /** Launch the tour from step 0, ignoring the seen flag (the `?` path). */
  start(): void;
  /** Advance one step; finishing past the last step closes the tour. */
  next(): void;
  /** Go back one step (no-op at step 0). */
  back(): void;
  /** Abandon the tour immediately. */
  skip(): void;
  /** Complete the tour (same close, distinct intent for callers/analytics). */
  finish(): void;
  /** Auto-prompt exactly once per login+browser: starts the tour AND records
   *  the seen flag the moment it starts, so it never auto-fires again. Guarded
   *  so a second call for the same key — or a call while already active — is a
   *  no-op. */
  startIfUnseen(userKey: string): void;
}

// A default inert controller so components that call useTour() render correctly
// even without a provider — the same convention the i18n LanguageContext uses
// for unit tests. The real provider below adds state; the default is a no-op.
const DEFAULT_TOUR: TourContextValue = {
  isActive: false,
  index: 0,
  total: TOUR_STEPS.length,
  current: null,
  start: () => {},
  next: () => {},
  back: () => {},
  skip: () => {},
  finish: () => {},
  startIfUnseen: () => {},
};

const TourContext = createContext<TourContextValue>(DEFAULT_TOUR);

export function TourProvider({ children }: { children: ReactNode }) {
  const [isActive, setIsActive] = useState(false);
  const [index, setIndex] = useState(0);
  // Mirrors isActive synchronously so startIfUnseen can decide whether to fire
  // WITHOUT reading state inside a setState updater (updaters must stay pure —
  // otherwise StrictMode's double-invocation double-runs the localStorage write).
  const isActiveRef = useRef(false);

  const total = TOUR_STEPS.length;

  const start = useCallback(() => {
    isActiveRef.current = true;
    setIndex(0);
    setIsActive(true);
  }, []);

  const close = useCallback(() => {
    isActiveRef.current = false;
    setIsActive(false);
    // Reset so a later re-launch always begins at the welcome step.
    setIndex(0);
  }, []);

  const next = useCallback(() => {
    setIndex((i) => {
      // Advancing past the final step ends the tour rather than overflowing.
      if (i >= total - 1) {
        isActiveRef.current = false;
        setIsActive(false);
        return 0;
      }
      return i + 1;
    });
  }, [total]);

  const back = useCallback(() => {
    setIndex((i) => (i > 0 ? i - 1 : 0));
  }, []);

  const startIfUnseen = useCallback((userKey: string) => {
    // Never auto-fire without a real key, if already seen, or if a tour is
    // already on screen (e.g. the user hit `?` first). All side effects run as
    // plain sequential statements — none inside a state updater.
    if (!userKey || hasSeenTour(userKey) || isActiveRef.current) return;
    // Record the flag the moment the tour auto-starts, so a refresh or a second
    // overview fetch never re-prompts this login.
    markTourSeen(userKey);
    isActiveRef.current = true;
    setIndex(0);
    setIsActive(true);
  }, []);

  const value = useMemo<TourContextValue>(
    () => ({
      isActive,
      index,
      total,
      current: isActive ? (TOUR_STEPS[index] ?? null) : null,
      start,
      next,
      back,
      skip: close,
      finish: close,
      startIfUnseen,
    }),
    [isActive, index, total, start, next, back, close, startIfUnseen]
  );

  return <TourContext.Provider value={value}>{children}</TourContext.Provider>;
}

/** Access the tour controller. Without a provider it returns the inert default
 *  (so a route renders fine in isolation, e.g. in unit tests); inside
 *  `TourProvider` it returns the live controller. */
export function useTour(): TourContextValue {
  return useContext(TourContext);
}
