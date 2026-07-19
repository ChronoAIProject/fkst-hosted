import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useReducedMotion } from 'framer-motion';
import { MOTION_MS, OverlayPresence } from '@/components/ui/motion';
import { ScrollArea } from '@/components/ui/scroll-area';

/** What counts as reachable-by-Tab inside the dialog for the focus trap. */
const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** Shared modal chrome: dimmed overlay, centered dialog card, Escape-to-close,
 *  and full focus management (aria-modal only hides the page from assistive
 *  tech — it does not move keyboard focus, so we must). Callers provide the
 *  heading (labelling the dialog), the body content, and — optionally — a
 *  sticky `footer` bar (typically Cancel/Submit) that stays visible however
 *  tall the body grows. Omitting `footer` renders exactly as before, so
 *  existing callers are unaffected. */
export function ModalShell({
  titleId,
  title,
  onClose,
  children,
  footer,
}: {
  titleId: string;
  title: string;
  onClose: () => void;
  children: ReactNode;
  /** Sticky bottom bar (e.g. Cancel/Submit). Optional — absent = no footer. */
  footer?: ReactNode;
}) {
  const dialogRef = useRef<HTMLDivElement>(null);
  // The element focus should return to when the dialog closes (captured on open).
  const openerRef = useRef<HTMLElement | null>(null);
  // OverlayPresence animates its OWN unmount, but only while it stays mounted
  // as `open` flips false. Callers mount ModalShell conditionally
  // (`{flag && <ModalShell/>}`), so if we forwarded onClose straight through
  // the parent would rip the tree out before the exit animation could play.
  // Instead we drive an internal `open` flag: a close request flips it false
  // (playing the scale-down/fade-out), then we call the parent's onClose once
  // the panel has left, which is when the parent finally unmounts us.
  const [open, setOpen] = useState(true);
  const closingRef = useRef(false);
  // null (pre-hydration) is treated as "motion allowed"; the exit timer below
  // collapses to 0 only when reduced motion is genuinely requested.
  const reduce = useReducedMotion() ?? false;

  const requestClose = useCallback(() => {
    // Guard against a double request (Escape held, rapid clicks) starting two
    // exit passes / firing onClose twice.
    if (closingRef.current) return;
    closingRef.current = true;
    setOpen(false);
  }, []);

  // Hand the parent its onClose only after the exit animation has run, so the
  // unmount it triggers doesn't cut the animation short. Under reduced motion
  // OverlayPresence swaps instantly, so we defer by a single tick (0 ms).
  useEffect(() => {
    if (open) return;
    // Hand focus back to the opener as the close begins — the dialog leaves the
    // DOM when its exit animation resolves, which can precede the deferred
    // unmount, so don't wait for unmount to restore focus.
    if (openerRef.current?.isConnected) openerRef.current.focus();
    const t = window.setTimeout(onClose, reduce ? 0 : MOTION_MS.modal);
    return () => window.clearTimeout(t);
  }, [open, reduce, onClose]);

  // On open, remember the opener and move focus to the first field (or the
  // dialog itself when there is none). Focus is handed back the moment a close
  // is initiated (see below), with an unmount-cleanup fallback for the case
  // where the parent yanks the tree before the close path runs.
  useEffect(() => {
    openerRef.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const dialog = dialogRef.current;
    if (dialog) {
      const first = dialog.querySelector<HTMLElement>(FOCUSABLE);
      (first ?? dialog).focus();
    }
    return () => {
      if (openerRef.current?.isConnected) openerRef.current.focus();
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        // Stop propagation so page-level Escape shortcuts (the canvas back
        // affordance) never fire while a dialog is open.
        e.stopPropagation();
        requestClose();
        return;
      }
      if (e.key !== 'Tab') return;
      // Trap Tab inside the dialog: wrap at both edges, and pull a focus that
      // somehow escaped (or sits on the container) back to the edges too.
      const dialog = dialogRef.current;
      if (!dialog) return;
      const focusable = [...dialog.querySelectorAll<HTMLElement>(FOCUSABLE)];
      if (focusable.length === 0) {
        e.preventDefault();
        dialog.focus();
        return;
      }
      const first = focusable[0]!;
      const last = focusable[focusable.length - 1]!;
      const active = document.activeElement;
      const inside = active instanceof HTMLElement && dialog.contains(active) && active !== dialog;
      if (e.shiftKey && (!inside || active === first)) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && (!inside || active === last)) {
        e.preventDefault();
        first.focus();
      }
    };
    // Capture phase so the dialog wins over any window-level key listener.
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [requestClose]);

  return (
    // OverlayPresence owns MOTION ONLY (scrim fade + panel scale/fade on both
    // open AND close). Its panel is neutralized to `presentation` so the real
    // labelled dialog — with the focus trap ref, aria-labelledby and Escape
    // handling — stays in ModalShell, exactly as before this refactor.
    <OverlayPresence open={open} variant="modal" role="presentation" className="w-full max-w-[460px]">
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className="flex max-h-[85vh] flex-col overflow-hidden border border-line rounded-modal bg-raise shadow-modal-seat"
      >
        <div className="shrink-0 px-6 pt-6 max-[600px]:px-5 max-[600px]:pt-5">
          <h3 id={titleId} className="font-display font-semibold text-modal-title text-fg">
            {title}
          </h3>
        </div>
        {/* Body scrolls internally so the header stays pinned and (when
            present) the footer never gets pushed off a tall form. */}
        <ScrollArea className="px-6 pb-6 pt-4 max-[600px]:px-5 max-[600px]:pb-5">
          <div className="flex flex-col gap-4">{children}</div>
        </ScrollArea>
        {footer != null && (
          <div className="shrink-0 border-t border-line bg-raise px-6 py-4 max-[600px]:px-5">
            {footer}
          </div>
        )}
      </div>
    </OverlayPresence>
  );
}
