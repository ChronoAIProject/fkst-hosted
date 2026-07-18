import { useEffect, useRef } from 'react';
import type { ReactNode } from 'react';

/** What counts as reachable-by-Tab inside the dialog for the focus trap. */
const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** Shared modal chrome: dimmed overlay, centered dialog card, Escape-to-close,
 *  and full focus management (aria-modal only hides the page from assistive
 *  tech — it does not move keyboard focus, so we must). Callers provide the
 *  heading (labelling the dialog) and the body content. */
export function ModalShell({
  titleId,
  title,
  onClose,
  children,
}: {
  titleId: string;
  title: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const dialogRef = useRef<HTMLDivElement>(null);

  // On open, remember the opener and move focus to the first field (or the
  // dialog itself when there is none); on close, hand focus back so keyboard
  // users land where they left off instead of at the document root.
  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const dialog = dialogRef.current;
    if (dialog) {
      const first = dialog.querySelector<HTMLElement>(FOCUSABLE);
      (first ?? dialog).focus();
    }
    return () => {
      if (opener?.isConnected) opener.focus();
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        // Stop propagation so page-level Escape shortcuts (the canvas back
        // affordance) never fire while a dialog is open.
        e.stopPropagation();
        onClose();
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
  }, [onClose]);

  return (
    <div className="anim-overlay-in fixed inset-0 z-50 flex items-center justify-center p-4 bg-[color-mix(in_oklab,var(--bg)_72%,transparent)]">
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className="anim-modal-in w-full max-w-[460px] border border-line rounded-modal bg-raise shadow-modal-seat p-6 max-[600px]:p-5 max-h-[85vh] overflow-y-auto"
      >
        <div className="flex flex-col gap-4">
          <h3 id={titleId} className="font-display font-semibold text-modal-title text-fg">
            {title}
          </h3>
          {children}
        </div>
      </div>
    </div>
  );
}
