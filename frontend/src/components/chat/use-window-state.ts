import { useCallback, useEffect, useState } from 'react';

/**
 * The panel's window state: width, full screen, and pin.
 *
 * Split into its own hook because it is persistence + clamping logic rather than
 * rendering, and because the panel component should not grow a second
 * responsibility to hold it.
 *
 * All three persist in `localStorage` (durable preferences about how you want the
 * surface to behave), unlike the transcript itself which is per-tab.
 */

const WIDTH_KEY = 'fkst-chat-width';
const PINNED_KEY = 'fkst-chat-pinned';

/** Width bounds. Below the minimum the composer and the timeline's expanded JSON
 *  stop being usable; above the maximum the panel stops being a companion to the
 *  page and becomes a takeover. */
export const MIN_WIDTH = 320;
export const MAX_WIDTH = 1100;
export const DEFAULT_WIDTH = 480;

/** Keyboard resize step, so the handle is operable without a pointer. */
export const RESIZE_STEP = 32;

/** Clamp to the bounds AND to the viewport, so a width stored on a wide monitor
 *  cannot render a panel wider than a narrow one. */
export function clampWidth(width: number, viewport: number): number {
  // A 24px gutter keeps the panel from butting against the window edge, matching
  // the `right-3` inset the panel already uses.
  const ceiling = Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, viewport - 24));
  return Math.min(ceiling, Math.max(MIN_WIDTH, Math.round(width)));
}

function readNumber(key: string, fallback: number): number {
  try {
    const raw = window.localStorage.getItem(key);
    if (raw == null) return fallback;
    const parsed = Number(raw);
    return Number.isFinite(parsed) ? parsed : fallback;
  } catch {
    // Blocked or full storage must never stop the panel opening.
    return fallback;
  }
}

function readFlag(key: string): boolean {
  try {
    return window.localStorage.getItem(key) === 'true';
  } catch {
    return false;
  }
}

function write(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // A lost preference is acceptable; a failed interaction is not.
  }
}

export function useWindowState() {
  const [width, setWidthState] = useState(() =>
    clampWidth(readNumber(WIDTH_KEY, DEFAULT_WIDTH), window.innerWidth)
  );
  // Full screen is deliberately NOT persisted: it is a momentary reading mode, and
  // reopening the app already maximised would be a surprise.
  const [fullScreen, setFullScreen] = useState(false);
  const [pinned, setPinnedState] = useState(() => readFlag(PINNED_KEY));

  const setWidth = useCallback((next: number) => {
    const clamped = clampWidth(next, window.innerWidth);
    setWidthState(clamped);
    write(WIDTH_KEY, String(clamped));
  }, []);

  const setPinned = useCallback((next: boolean) => {
    setPinnedState(next);
    write(PINNED_KEY, String(next));
  }, []);

  // A stored width that no longer fits — the window was narrowed, or the panel was
  // sized on another monitor — is re-clamped rather than left overflowing.
  useEffect(() => {
    const onResize = () => setWidthState((current) => clampWidth(current, window.innerWidth));
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);

  return {
    width,
    setWidth,
    fullScreen,
    toggleFullScreen: useCallback(() => setFullScreen((current) => !current), []),
    exitFullScreen: useCallback(() => setFullScreen(false), []),
    pinned,
    setPinned,
    togglePinned: useCallback(() => setPinned(!pinned), [pinned, setPinned]),
  };
}
