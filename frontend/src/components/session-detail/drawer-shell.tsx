import { useEffect, useRef } from 'react';
import type { ReactNode } from 'react';

/** What counts as reachable-by-Tab inside the drawer for the focus trap. */
const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** Right-anchored slide-in drawer chrome: dimmed overlay, full-height panel,
 *  Escape-to-close, and the same focus management ModalShell applies (aria-modal
 *  hides the page from assistive tech but does not move keyboard focus). The
 *  caller owns the header + body; this owns the shell + accessibility. */
export function DrawerShell({
  titleId,
  onClose,
  children,
}: {
  /** id of the element that labels the dialog (the drawer title). */
  titleId: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const panelRef = useRef<HTMLDivElement>(null);

  // On open, move focus into the drawer; on close, hand it back to the opener
  // so keyboard users land where they left off.
  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const panel = panelRef.current;
    if (panel) {
      const first = panel.querySelector<HTMLElement>(FOCUSABLE);
      (first ?? panel).focus();
    }
    return () => {
      if (opener?.isConnected) opener.focus();
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        // Stop propagation so the canvas' page-level Escape (level-back) never
        // fires while the drawer is open.
        e.stopPropagation();
        onClose();
        return;
      }
      if (e.key !== 'Tab') return;
      const panel = panelRef.current;
      if (!panel) return;
      const focusable = [...panel.querySelectorAll<HTMLElement>(FOCUSABLE)];
      if (focusable.length === 0) {
        e.preventDefault();
        panel.focus();
        return;
      }
      const first = focusable[0]!;
      const last = focusable[focusable.length - 1]!;
      const active = document.activeElement;
      const inside = active instanceof HTMLElement && panel.contains(active) && active !== panel;
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
  }, [onClose]);

  return (
    <div className="anim-overlay-in fixed inset-0 z-50 flex justify-end bg-[color-mix(in_oklab,var(--bg)_72%,transparent)]">
      {/* Overlay click-to-close: a plain backdrop button so the panel is the
          only interactive chrome behind assistive tech. */}
      <button
        type="button"
        aria-hidden="true"
        tabIndex={-1}
        onClick={onClose}
        className="absolute inset-0 cursor-default"
      />
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className="anim-drawer-in relative w-full max-w-[560px] h-full border-l border-line bg-raise shadow-modal-seat overflow-y-auto"
      >
        {children}
      </div>
    </div>
  );
}
