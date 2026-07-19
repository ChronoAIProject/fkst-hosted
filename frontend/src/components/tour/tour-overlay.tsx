import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Link } from 'react-router-dom';
import { motion, useReducedMotion } from 'framer-motion';
import { useContent } from '@/i18n';
import type { SiteContent } from '@/i18n/types';
import { ModalShell } from '@/components/modals/modal-shell';
import { useTour } from './tour-context';
import { stepCopy, TOUR_STEPS } from './tour-steps';
import type { TourPlacement, TourStep } from './tour-steps';

// The tour's visual layer. It portals to <body> so the spotlight dim and the
// coachmark card sit above every route. Modal steps reuse ModalShell (shared
// dialog chrome + focus trap); spotlight steps are hand-rolled — a single
// large-spread box-shadow dims everything except the ringed target, with an
// anchored, auto-flipping tooltip. A target absent from the DOM (a level/state-
// dependent affordance) degrades to a centered card so the tour never breaks.

/** Gap between the target ring and the tooltip; PAD is the ring's inset around
 *  the target so the highlight breathes. */
const GAP = 12;
const PAD = 6;

/** What counts as reachable-by-Tab inside the coachmark for the focus trap. */
const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])';

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(v, hi));

/** Anchor the tooltip beside the target on the preferred side, flipping to the
 *  opposite (or roomiest) side when the preferred side lacks space, then
 *  clamping fully inside the viewport. */
function placeTooltip(
  rect: DOMRect,
  prefer: TourPlacement,
  w: number,
  h: number,
  vw: number,
  vh: number
): { top: number; left: number } {
  const space: Record<TourPlacement, number> = {
    top: rect.top,
    bottom: vh - rect.bottom,
    left: rect.left,
    right: vw - rect.right,
  };
  const need = (side: TourPlacement) =>
    side === 'top' || side === 'bottom' ? h + GAP + PAD : w + GAP + PAD;

  let side = prefer;
  if (space[side] < need(side)) {
    const opposite: Record<TourPlacement, TourPlacement> = {
      top: 'bottom',
      bottom: 'top',
      left: 'right',
      right: 'left',
    };
    const opp = opposite[side];
    side =
      space[opp] >= need(opp)
        ? opp
        : (Object.keys(space) as TourPlacement[]).sort((a, b) => space[b] - space[a])[0]!;
  }

  let top: number;
  let left: number;
  switch (side) {
    case 'bottom':
      top = rect.bottom + GAP + PAD;
      left = rect.left + rect.width / 2 - w / 2;
      break;
    case 'top':
      top = rect.top - GAP - PAD - h;
      left = rect.left + rect.width / 2 - w / 2;
      break;
    case 'right':
      left = rect.right + GAP + PAD;
      top = rect.top + rect.height / 2 - h / 2;
      break;
    default: // left
      left = rect.left - GAP - PAD - w;
      top = rect.top + rect.height / 2 - h / 2;
  }
  return { top: clamp(top, GAP, vh - h - GAP), left: clamp(left, GAP, vw - w - GAP) };
}

const SECONDARY_BTN =
  'font-ui font-semibold text-[12px] grad-border bg-glass backdrop-blur-glass rounded-control px-3 py-1.5 text-dim hover:text-fg hover:shadow-glow-amber transition-[color,box-shadow] duration-200 cursor-pointer';
const PRIMARY_BTN =
  'anim-sheen font-ui font-semibold text-[12px] bg-grad-accent text-amber-ink rounded-control px-3.5 py-1.5 no-underline inline-flex items-center shadow-[var(--shadow-2),var(--glow-amber)] transition-[filter,box-shadow] hover:brightness-110 cursor-pointer';

/** Progress counter + Skip/Back/Next controls, shared by modal and spotlight
 *  steps. The final (finish) step swaps Skip/Next for a Get Started link and a
 *  Done button. */
function TourNav({ index, total, isLast }: { index: number; total: number; isLast: boolean }) {
  const t = useContent().tour;
  const { skip, back, next, finish } = useTour();
  const progress = t.progress.replace('{n}', String(index + 1)).replace('{m}', String(total));
  return (
    <div className="mt-4 flex items-center justify-between gap-3">
      <span className="font-mono text-[11px] text-ghost">{progress}</span>
      <div className="flex items-center gap-2">
        {index > 0 && (
          <button type="button" onClick={back} className={SECONDARY_BTN}>
            {t.back}
          </button>
        )}
        {isLast ? (
          <>
            {/* Link needs router context — the overlay is mounted inside the
                router root (Shell), so this resolves. */}
            <Link to="/get-started" onClick={finish} className={PRIMARY_BTN}>
              {t.getStarted}
            </Link>
            <button type="button" onClick={finish} className={SECONDARY_BTN}>
              {t.done}
            </button>
          </>
        ) : (
          <>
            <button type="button" onClick={skip} className={SECONDARY_BTN}>
              {t.skip}
            </button>
            <button type="button" onClick={next} className={PRIMARY_BTN}>
              {t.next}
            </button>
          </>
        )}
      </div>
    </div>
  );
}

/** Arrow-key stepping shared by both step kinds (Right = next, Left = back).
 *  Escape is owned per-kind (ModalShell for modal; SpotlightStep below). */
function useArrowKeys() {
  const { next, back } = useTour();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'ArrowRight') {
        e.preventDefault();
        next();
      } else if (e.key === 'ArrowLeft') {
        e.preventDefault();
        back();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [next, back]);
}

/** Centered welcome/finish dialog built on the shared ModalShell chrome. */
function ModalStep({
  step,
  index,
  total,
  content,
}: {
  step: TourStep;
  index: number;
  total: number;
  content: SiteContent;
}) {
  const { skip } = useTour();
  useArrowKeys();
  const copy = stepCopy(content, step);
  const isLast = index === total - 1;
  return (
    <ModalShell
      titleId={`tour-${step.id}`}
      title={copy.title}
      onClose={skip}
      footer={<TourNav index={index} total={total} isLast={isLast} />}
    >
      <p className="text-[14px] leading-relaxed text-dim">{copy.body}</p>
    </ModalShell>
  );
}

/** Dim-the-page coachmark: rings a real element and anchors a tooltip; falls
 *  back to a centered card when the target is not in the DOM. */
function SpotlightStep({
  step,
  index,
  total,
  content,
}: {
  step: TourStep;
  index: number;
  total: number;
  content: SiteContent;
}) {
  const { skip } = useTour();
  const reduce = useReducedMotion() ?? false;
  const cardRef = useRef<HTMLDivElement>(null);
  const [rect, setRect] = useState<DOMRect | null>(null);
  const [found, setFound] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const copy = stepCopy(content, step);
  const t = content.tour;

  useArrowKeys();

  // Locate + measure the target. A missing element flips `found` false so the
  // render degrades to a centered card.
  const measure = useCallback(() => {
    const el = step.target
      ? document.querySelector<HTMLElement>(`[data-tour="${step.target}"]`)
      : null;
    if (el) {
      setFound(true);
      setRect(el.getBoundingClientRect());
    } else {
      setFound(false);
      setRect(null);
    }
  }, [step.target]);

  // On step change: bring the target into view, then measure.
  useLayoutEffect(() => {
    const el = step.target
      ? document.querySelector<HTMLElement>(`[data-tour="${step.target}"]`)
      : null;
    if (el) el.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    measure();
  }, [step.target, measure]);

  // Keep the ring glued to the target as the page resizes or scrolls (rAF
  // throttled; capture:true catches inner scroll containers too).
  useEffect(() => {
    let raf = 0;
    const onChange = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(measure);
    };
    window.addEventListener('resize', onChange);
    window.addEventListener('scroll', onChange, true);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener('resize', onChange);
      window.removeEventListener('scroll', onChange, true);
    };
  }, [measure]);

  // Position the tooltip once its own box has a measured size (anchored beside
  // the target, or centered in the fallback).
  useLayoutEffect(() => {
    const card = cardRef.current;
    if (!card) return;
    const tt = card.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    if (found && rect) {
      setPos(placeTooltip(rect, step.placement, tt.width, tt.height, vw, vh));
    } else {
      setPos({ top: (vh - tt.height) / 2, left: (vw - tt.width) / 2 });
    }
  }, [found, rect, step.placement, index]);

  // Move focus into the card, and trap Tab + own Escape while it is up.
  useEffect(() => {
    const card = cardRef.current;
    if (card) {
      const first = card.querySelector<HTMLElement>(FOCUSABLE);
      (first ?? card).focus();
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        skip();
        return;
      }
      if (e.key !== 'Tab') return;
      const c = cardRef.current;
      if (!c) return;
      const items = [...c.querySelectorAll<HTMLElement>(FOCUSABLE)];
      if (items.length === 0) {
        e.preventDefault();
        c.focus();
        return;
      }
      const first = items[0]!;
      const last = items[items.length - 1]!;
      const active = document.activeElement;
      const inside = active instanceof HTMLElement && c.contains(active) && active !== c;
      if (e.shiftKey && (!inside || active === first)) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && (!inside || active === last)) {
        e.preventDefault();
        first.focus();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [skip, index]);

  const dimBg = 'color-mix(in oklab, var(--bg) 78%, transparent)';

  return (
    <div className="fixed inset-0 z-[60]" data-testid="tour-spotlight">
      {/* Click-catcher: pauses page interaction during the tour. When no target
          is ringed it also paints the dim (the ring's box-shadow paints it
          otherwise). Clicking it is inert — only Skip/Escape dismiss. */}
      <div
        className="fixed inset-0"
        aria-hidden="true"
        style={found ? undefined : { background: dimBg }}
      />

      {found && rect && (
        <motion.div
          aria-hidden="true"
          className="fixed pointer-events-none"
          style={{
            top: rect.top - PAD,
            left: rect.left - PAD,
            width: rect.width + PAD * 2,
            height: rect.height + PAD * 2,
            borderRadius: 10,
            // The huge spread paints the surrounding dim (kept dim, unchanged);
            // the element interior stays clear so the target shows through. A
            // trailing amber bloom rings the highlight so the focus target reads
            // as brand-lit, not just outlined.
            boxShadow: `0 0 0 9999px ${dimBg}, var(--glow-amber)`,
            outline: '2px solid var(--amber)',
            outlineOffset: 2,
          }}
          initial={reduce ? false : { opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={{ duration: reduce ? 0 : 0.15, ease: 'easeOut' }}
        />
      )}

      <motion.div
        ref={cardRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={`tour-${step.id}`}
        tabIndex={-1}
        className="fixed w-[320px] max-w-[calc(100vw-24px)] grad-border grad-border-accent bg-glass backdrop-blur-glass rounded-panel shadow-[var(--highlight-top),var(--shadow-3),var(--glow-amber)] p-5 outline-none"
        style={{
          top: pos?.top ?? 0,
          left: pos?.left ?? 0,
          // Hide until positioned so the card never flashes at 0,0.
          visibility: pos ? 'visible' : 'hidden',
        }}
        initial={reduce ? false : { opacity: 0, scale: 0.98 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={{ duration: reduce ? 0 : 0.15, ease: 'easeOut' }}
      >
        <h3
          id={`tour-${step.id}`}
          className="grad-text grad-text-fg font-display font-semibold text-[15px]"
        >
          {copy.title}
        </h3>
        <p className="mt-2 text-[13px] leading-relaxed text-dim">{copy.body}</p>
        <TourNav index={index} total={total} isLast={index === total - 1} />
        {/* Accessible dismiss echoing the visible Skip, labelled for SR users. */}
        <button
          type="button"
          onClick={skip}
          aria-label={t.closeAria}
          className="absolute top-3 right-3 w-6 h-6 inline-flex items-center justify-center rounded-control text-ghost hover:text-fg transition-colors cursor-pointer"
        >
          <span aria-hidden="true" className="text-[14px] leading-none">
            ✕
          </span>
        </button>
      </motion.div>
    </div>
  );
}

/** The single tour overlay. Renders nothing when the tour is inactive; portals
 *  the active step to <body>. Mounted once (inside Shell) so every route shares
 *  it. */
export function TourOverlay() {
  const { isActive, index, current } = useTour();
  const content = useContent();

  if (!isActive || current == null) return null;

  const total = TOUR_STEPS.length;
  const node =
    current.variant === 'modal' ? (
      <ModalStep key={current.id} step={current} index={index} total={total} content={content} />
    ) : (
      <SpotlightStep key={current.id} step={current} index={index} total={total} content={content} />
    );

  return createPortal(node, document.body);
}
