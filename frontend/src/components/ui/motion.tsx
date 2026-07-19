import { AnimatePresence, motion, useReducedMotion } from 'framer-motion';
import type { CSSProperties, ReactNode } from 'react';

/**
 * Shared, reduced-motion-aware transition primitives.
 *
 * Every later animation item in the dashboard refactor consumes these wrappers
 * instead of re-deriving framer-motion props, so the whole app animates on one
 * curve and one set of durations. The vocabulary deliberately mirrors the
 * CSS-only keyframes in `index.css` (row-in / overlay-in / modal-in /
 * drawer-in) — this is the JS/AnimatePresence counterpart used wherever an
 * element must animate its OWN unmount (which a CSS class cannot do).
 *
 * Reduced-motion contract: each wrapper internally calls `useReducedMotion()`
 * and, when the user prefers reduced motion, collapses to an INSTANT swap —
 * the element mounts directly at its final visual state and exits with no
 * animated intermediate. Content is never gated behind a skipped animation.
 */

/** The dashboard's single easing curve. Matches `.anim-row-in`'s
 *  cubic-bezier(0.2, 0.7, 0.3, 1) in `index.css` so JS and CSS motion agree.
 *  Typed as a mutable 4-tuple because framer-motion's `BezierDefinition`
 *  rejects a readonly tuple. */
export const MOTION_EASE: [number, number, number, number] = [0.2, 0.7, 0.3, 1];

/** Canonical durations (ms). Exported so consumers stay in lock-step with the
 *  primitives rather than hard-coding their own timings. */
export const MOTION_MS = {
  route: 180,
  fade: 140,
  reveal: 200,
  backdrop: 150,
  modal: 200,
  drawer: 220,
} as const;

/** Per-row delay step for stagger lists, mirroring the CSS `--stagger` cadence. */
export const STAGGER_STEP_MS = 40;

/** framer-motion takes seconds; the constants above are authored in ms. */
const s = (ms: number) => ms / 1000;

/**
 * (a) Route-level crossfade. Wrap the routed content and feed the pathname as
 * `k`; when it changes, the outgoing view fades/lifts out and the incoming one
 * fades/lifts in on the shared curve. Uses `mode="popLayout"` so the entering
 * view is not held back waiting for the old one to leave (routes should feel
 * immediate). Under reduced motion the swap is instant.
 */
export function RouteTransition({
  k,
  children,
  className,
}: {
  /** Route identity (typically the pathname). A new value triggers the swap. */
  k: string;
  children: ReactNode;
  className?: string;
}) {
  const reduce = useReducedMotion();
  return (
    <AnimatePresence mode="popLayout" initial={false}>
      <motion.div
        key={k}
        className={className}
        // initial={false} on the reduced path mounts each keyed view directly
        // at its final state — no enter animation to observe.
        initial={reduce ? false : { opacity: 0, y: 6 }}
        animate={{ opacity: 1, y: 0 }}
        exit={reduce ? { opacity: 1 } : { opacity: 0, y: -6 }}
        transition={{ duration: reduce ? 0 : s(MOTION_MS.route), ease: MOTION_EASE }}
      >
        {children}
      </motion.div>
    </AnimatePresence>
  );
}

/**
 * (b) Keyed crossfade for tab bodies and loaded-vs-loading swaps. `mode="wait"`
 * so the outgoing body fully fades before the incoming one appears (no overlap
 * flash between two differently-sized panels). Instant under reduced motion.
 */
export function FadeSwap({
  k,
  children,
  className,
}: {
  /** Content identity — change it to crossfade to a new body. */
  k: string;
  children: ReactNode;
  className?: string;
}) {
  const reduce = useReducedMotion();
  return (
    <AnimatePresence mode="wait" initial={false}>
      <motion.div
        key={k}
        className={className}
        initial={reduce ? false : { opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={reduce ? { opacity: 1 } : { opacity: 0 }}
        transition={{ duration: reduce ? 0 : s(MOTION_MS.fade), ease: 'easeOut' }}
      >
        {children}
      </motion.div>
    </AnimatePresence>
  );
}

/**
 * (c) Disclosure reveal: animates height:auto + opacity open/closed. Rendered
 * through AnimatePresence so callers can conditionally unmount the body and
 * still get a collapse animation. `overflow: hidden` keeps the content clipped
 * while the height interpolates. Under reduced motion it appears/disappears at
 * full height instantly.
 */
export function Reveal({
  open,
  children,
  className,
}: {
  open: boolean;
  children: ReactNode;
  className?: string;
}) {
  const reduce = useReducedMotion();
  return (
    <AnimatePresence initial={false}>
      {open && (
        <motion.div
          key="reveal"
          className={className}
          style={{ overflow: 'hidden' }}
          initial={reduce ? false : { height: 0, opacity: 0 }}
          animate={{ height: 'auto', opacity: 1 }}
          exit={reduce ? { opacity: 1 } : { height: 0, opacity: 0 }}
          transition={{ duration: reduce ? 0 : s(MOTION_MS.reveal), ease: MOTION_EASE }}
        >
          {children}
        </motion.div>
      )}
    </AnimatePresence>
  );
}

/** Variant-specific panel motion. Modal = scale/opacity/y (mirrors
 *  `@keyframes modal-in`); drawer = opaque slide from the right edge (mirrors
 *  `@keyframes drawer-in`, which deliberately keeps the panel opaque so
 *  underlying content never bleeds through mid-slide). */
function panelMotion(variant: 'modal' | 'drawer', reduce: boolean) {
  if (reduce) {
    // Mount at rest, leave with no visual change: an instant swap.
    return {
      initial: false as const,
      animate: variant === 'modal' ? { opacity: 1, scale: 1, y: 0 } : { x: 0 },
      exit: variant === 'modal' ? { opacity: 1, scale: 1, y: 0 } : { x: 0 },
      duration: 0,
    };
  }
  if (variant === 'modal') {
    return {
      initial: { opacity: 0, scale: 0.96, y: 10 },
      animate: { opacity: 1, scale: 1, y: 0 },
      exit: { opacity: 0, scale: 0.96, y: 10 },
      duration: s(MOTION_MS.modal),
    };
  }
  return {
    initial: { x: '100%' },
    animate: { x: 0 },
    exit: { x: '100%' },
    duration: s(MOTION_MS.drawer),
  };
}

/** Default full-screen scrim + layout per variant. Modal centers its panel;
 *  drawer pins it to the right edge, full height. */
const OVERLAY_LAYOUT: Record<'modal' | 'drawer', string> = {
  modal: 'items-center justify-center p-4',
  drawer: 'items-stretch justify-end',
};

/**
 * (d) Backdrop + animated panel for modals and drawers. Renders through
 * AnimatePresence keyed on `open`, so a caller can flip `open` to false and
 * still get the exit animation before the tree unmounts (a plain conditional
 * `{open && …}` cannot). The backdrop fades; the panel enters/exits per
 * `variant`. Under reduced motion both appear/disappear instantly.
 *
 * This is a MOTION primitive only — it does not own focus trapping or Escape
 * handling (that stays with the caller's dialog chrome, e.g. `ModalShell`).
 */
export function OverlayPresence({
  open,
  variant,
  children,
  onBackdropClick,
  className,
  backdropClassName,
  label,
  role = 'dialog',
}: {
  open: boolean;
  variant: 'modal' | 'drawer';
  children: ReactNode;
  /** Called when the scrim (not the panel) is clicked — typically closes. */
  onBackdropClick?: () => void;
  /** Extra classes for the panel wrapper. */
  className?: string;
  /** Extra classes for the full-screen scrim/layout container. */
  backdropClassName?: string;
  /** Accessible name for the panel when it has no visible labelled heading. */
  label?: string;
  /** Panel ARIA role; defaults to `dialog`. */
  role?: string;
}) {
  // useReducedMotion() is `boolean | null` (null before hydration); treat the
  // unknown state as "not reduced" so panelMotion always gets a concrete flag.
  const reduce = useReducedMotion() ?? false;
  const p = panelMotion(variant, reduce);

  return (
    <AnimatePresence>
      {open && (
        // The scrim is the AnimatePresence-tracked element; the panel nested
        // inside it also runs its exit variant because framer-motion propagates
        // exit to every descendant motion component before unmounting the tree.
        <motion.div
          key="overlay"
          className={[
            'fixed inset-0 z-50 flex bg-[color-mix(in_oklab,var(--bg)_72%,transparent)]',
            OVERLAY_LAYOUT[variant],
            backdropClassName ?? '',
          ]
            .filter(Boolean)
            .join(' ')}
          initial={reduce ? false : { opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={reduce ? { opacity: 1 } : { opacity: 0 }}
          transition={{ duration: reduce ? 0 : s(MOTION_MS.backdrop), ease: 'easeOut' }}
          onClick={onBackdropClick}
        >
          <motion.div
            role={role}
            aria-modal="true"
            aria-label={label}
            className={className}
            // Clicks on the panel must not reach the scrim's close handler.
            onClick={(e) => e.stopPropagation()}
            initial={p.initial}
            animate={p.animate}
            exit={p.exit}
            transition={{ duration: p.duration, ease: MOTION_EASE }}
          >
            {children}
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

/** The inline style carrying the index-based `--stagger` delay for
 *  `.anim-row-in` lists. Exported for callers that need to apply the delay to
 *  their own element type instead of `StaggerItem`'s `<div>`. */
export function staggerStyle(index: number, step: number = STAGGER_STEP_MS): CSSProperties {
  // Guard against NaN/negative indices from unvalidated callers: a bad index
  // must degrade to zero delay, never emit `--stagger: NaNms`.
  const safe = Number.isFinite(index) && index > 0 ? index : 0;
  return { ['--stagger']: `${safe * step}ms` } as CSSProperties;
}

/**
 * (e) A list row that fades/slides in on the shared curve with an index-based
 * stagger. It rides the CSS `.anim-row-in` class (not framer-motion), so it is
 * automatically reduced-motion-safe: `index.css` disables `.anim-row-in` under
 * `prefers-reduced-motion`, leaving the row at its final state.
 */
export function StaggerItem({
  index,
  children,
  className,
  step = STAGGER_STEP_MS,
}: {
  /** Position in the list; drives the per-row delay. */
  index: number;
  children: ReactNode;
  className?: string;
  /** Delay between consecutive rows (ms). */
  step?: number;
}) {
  return (
    <div className={['anim-row-in', className ?? ''].filter(Boolean).join(' ')} style={staggerStyle(index, step)}>
      {children}
    </div>
  );
}
