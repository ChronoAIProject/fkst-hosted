import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { AnimatePresence, motion, useReducedMotion } from 'framer-motion';
import { MOTION_EASE } from './motion';
import { cn } from '@/lib/utils';

/**
 * Transient toast/notice primitive.
 *
 * Consumers fire ephemeral notices from anywhere in the tree via `useToast()`
 * (typically on a successful mutation), and a single `<Toaster>` render surface
 * — mounted once at the app root — draws them stacked in the bottom-right
 * corner with auto- and manual dismiss.
 *
 * The provider owns the queue and the dismissal timers; the render surface is a
 * separate export so the app root can place it wherever the stacking context
 * demands, without the provider dictating layout. State (the queue) and
 * behavior (show/dismiss) travel through one context.
 *
 * Motion: enter/exit ride framer-motion's `AnimatePresence` (a plain CSS class
 * cannot animate its OWN unmount, which a dismissing toast must), reusing the
 * shared `MOTION_EASE` curve — the same cubic-bezier(0.2,0.7,0.3,1) as the
 * `.anim-notice-in` keyframe in `index.css`. Under `prefers-reduced-motion` the
 * toast mounts directly at its final state and leaves with no animated
 * intermediate, so a notice is never hidden behind a skipped animation.
 */

export type ToastKind = 'success' | 'info' | 'error';

/** What a caller passes to `show()`. */
export interface ToastOptions {
  /** Semaphore intent; drives the accent edge. Defaults to `info`. */
  kind?: ToastKind;
  /** The already-localized notice text. Every user-facing string is i18n at
   *  the call site; the primitive renders it verbatim. */
  message: string;
  /** Auto-dismiss delay in ms. Omitted / invalid → the default (~4s). */
  ttlMs?: number;
}

/** The public API returned by `useToast()`. */
export interface ToastApi {
  /** Enqueue a notice. Returns its id (usable with `dismiss`), or `-1` when the
   *  call was rejected (empty message) and nothing was enqueued. */
  show: (opts: ToastOptions) => number;
  /** Remove a notice early by id. A no-op for an unknown / already-gone id. */
  dismiss: (id: number) => void;
}

/** An active notice in the queue. */
interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

/** Default auto-dismiss window. Long enough to read a short notice, short
 *  enough that a stack of them does not pile up. */
const DEFAULT_TTL_MS = 4000;

/** Notice motion duration (ms). Mirrors the 180ms of `.anim-notice-in` so the
 *  JS-animated toast agrees with the app's CSS-only notices. */
const NOTICE_MS = 180;

/**
 * Validate a caller-supplied TTL at the boundary: unvalidated data can hand us
 * `NaN`, a negative, or zero, any of which would schedule a broken timer
 * (never firing, or firing instantly). Fall back to the default instead.
 */
function resolveTtl(ttlMs: number | undefined): number {
  if (typeof ttlMs === 'number' && Number.isFinite(ttlMs) && ttlMs > 0) return ttlMs;
  return DEFAULT_TTL_MS;
}

interface ToastContextValue extends ToastApi {
  toasts: Toast[];
}

const ToastContext = createContext<ToastContextValue | null>(null);

/** Internal: full context (queue + api) for the render surface. Throws outside
 *  a provider so a mis-mounted `<Toaster>` fails loudly rather than silently
 *  rendering nothing. */
function useToastContext(): ToastContextValue {
  const ctx = useContext(ToastContext);
  if (!ctx) {
    throw new Error('Toast components must be rendered within a <ToastProvider>.');
  }
  return ctx;
}

/**
 * Public hook. Call inside any descendant of `<ToastProvider>` to raise
 * notices. Returns a stable `{ show, dismiss }` — safe to close over in effects
 * and event handlers without re-subscribing.
 */
export function useToast(): ToastApi {
  const { show, dismiss } = useToastContext();
  return useMemo(() => ({ show, dismiss }), [show, dismiss]);
}

/** Holds the notice queue and dismissal timers. Renders only `children`; mount
 *  `<Toaster>` (anywhere below) to actually draw the notices. */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  // Monotonic id source: a ref (not state) so issuing an id never triggers a
  // render, and ids stay unique for the provider's whole lifetime.
  const idRef = useRef(0);
  // id → pending auto-dismiss handle, so a manual dismiss can cancel the timer
  // and unmount cleanup can flush every outstanding one.
  const timers = useRef(new Map<number, number>());

  const dismiss = useCallback((id: number) => {
    const handle = timers.current.get(id);
    if (handle !== undefined) {
      window.clearTimeout(handle);
      timers.current.delete(id);
    }
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const show = useCallback(
    (opts: ToastOptions): number => {
      // An empty / whitespace-only message is a caller bug: emitting it would
      // announce nothing to the polite live region and draw an empty card.
      // Reject explicitly rather than enqueue a meaningless notice.
      const message = typeof opts.message === 'string' ? opts.message : '';
      if (message.trim().length === 0) return -1;

      const id = ++idRef.current;
      const ttlMs = resolveTtl(opts.ttlMs);
      setToasts((prev) => [...prev, { id, kind: opts.kind ?? 'info', message }]);

      const handle = window.setTimeout(() => dismiss(id), ttlMs);
      timers.current.set(id, handle);
      return id;
    },
    [dismiss]
  );

  // Flush every pending timer on unmount so a fired timeout can never call
  // setState on an unmounted provider.
  useEffect(() => {
    const pending = timers.current;
    return () => {
      for (const handle of pending.values()) window.clearTimeout(handle);
      pending.clear();
    };
  }, []);

  const value = useMemo<ToastContextValue>(
    () => ({ toasts, show, dismiss }),
    [toasts, show, dismiss]
  );

  return <ToastContext.Provider value={value}>{children}</ToastContext.Provider>;
}

/** Left-edge accent per intent. Info stays neutral so success/error read as the
 *  only colored states (semaphore used sparingly, per the token guidance). */
const KIND_ACCENT: Record<ToastKind, string> = {
  success: 'border-l-green',
  error: 'border-l-red',
  info: 'border-l-line-2',
};

/**
 * The render surface. Mount exactly once, near the app root. Draws the active
 * notices bottom-right, newest at the bottom of the stack. The container is a
 * persistent `aria-live="polite"` region (present even when empty) so a
 * screen reader announces late-arriving notices; it is `pointer-events-none`
 * so it never blocks the UI beneath, while each card re-enables pointer events
 * for its dismiss control.
 */
export function Toaster({
  dismissLabel = 'Dismiss',
  className,
}: {
  /** Accessible label for the per-toast dismiss control. Consumers pass an
   *  already-localized string; the English default is a bare fallback. */
  dismissLabel?: string;
  className?: string;
}) {
  const { toasts, dismiss } = useToastContext();
  const reduce = useReducedMotion();

  return (
    <div
      aria-live="polite"
      aria-atomic="false"
      className={cn(
        'pointer-events-none fixed bottom-4 right-4 z-[60] flex w-full max-w-sm flex-col gap-2',
        className
      )}
    >
      <AnimatePresence initial={false}>
        {toasts.map((t) => (
          <motion.div
            key={t.id}
            layout={!reduce}
            // Enter risen + faded from below (matching a bottom-anchored stack);
            // exit reverses it. initial={false} under reduced motion mounts the
            // card at rest, and the exit collapses opacity only — no movement.
            initial={reduce ? false : { opacity: 0, y: 8, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={reduce ? { opacity: 0 } : { opacity: 0, y: 8, scale: 0.98 }}
            transition={{ duration: reduce ? 0 : NOTICE_MS / 1000, ease: MOTION_EASE }}
            className={cn(
              'pointer-events-auto flex items-start gap-3 rounded-card border border-line border-l-2 bg-raise-2 px-3 py-2 text-[12.5px] text-dim',
              KIND_ACCENT[t.kind]
            )}
          >
            <span className="flex-1 break-words">{t.message}</span>
            <button
              type="button"
              onClick={() => dismiss(t.id)}
              aria-label={dismissLabel}
              className="-mr-1 flex-none rounded-chip px-1 leading-none text-faint transition-colors hover:text-fg cursor-pointer"
            >
              <span aria-hidden="true">×</span>
            </button>
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  );
}
